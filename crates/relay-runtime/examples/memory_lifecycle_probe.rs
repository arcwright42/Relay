//! Real ACP observer, checkpoints, and recovery. Requires --execute; uses disposable data.
use anyhow::{Context, Result, ensure};
use relay_core::{ThreadId, agents::*};
use relay_runtime::{AgentRuntime, native::*};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn until(mut check: impl FnMut() -> Result<bool>, seconds: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if check()? {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "Lifecycle probe timed out");
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn connect(agents: &AgentRuntime) -> Result<()> {
    agents
        .dispatch(MAIN, AgentCommand::Connect(AgentSource::Auto))
        .map_err(anyhow::Error::msg)?;
    until(
        || {
            let state = agents.snapshot(MAIN);
            ensure!(
                !matches!(
                    state.status,
                    ConnectionStatus::Failed | ConnectionStatus::NeedsAuthentication
                ),
                "Main ACP error: {:?}",
                state.error
            );
            Ok(state.status == ConnectionStatus::Ready)
        },
        120,
    )
}
fn turn(agents: &AgentRuntime, prompt: &str) -> Result<String> {
    let id = agents
        .send_turn(MAIN, prompt.into())
        .map_err(anyhow::Error::msg)?;
    until(
        || {
            let state = agents.snapshot(MAIN);
            ensure!(
                state.status != ConnectionStatus::Failed,
                "ACP failed: {:?}",
                state.error
            );
            ensure!(
                state.permissions.is_empty(),
                "Unexpected permission request in a read-only probe"
            );
            Ok(state
                .messages
                .iter()
                .any(|m| m.id == id && m.status != MessageStatus::Streaming))
        },
        180,
    )?;
    let state = agents.snapshot(MAIN);
    let message = state
        .messages
        .iter()
        .find(|m| m.id == id)
        .context("Reply disappeared")?;
    ensure!(
        message.status == MessageStatus::Complete,
        "Reply did not finish"
    );
    Ok(message.text.clone())
}
fn drain(db: &rusqlite::Connection) -> Result<()> {
    until(
        || {
            let failures: u64 =
                db.query_row("SELECT count(*) FROM jobs WHERE state='failed'", [], |r| {
                    r.get(0)
                })?;
            ensure!(
                failures == 0,
                "Observer extraction failed; inspect the isolated database"
            );
            let pending: u64 = db.query_row(
                "SELECT count(*) FROM jobs WHERE state IN ('pending','running')",
                [],
                |r| r.get(0),
            )?;
            let checkpoints: u64 =
                db.query_row("SELECT count(*) FROM session_summaries", [], |r| r.get(0))?;
            Ok(pending == 0 && checkpoints > 0)
        },
        360,
    )
}
fn open(root: &Path) -> Result<(Arc<AgentRuntime>, ResidentWorker)> {
    let store = Arc::new(NativeStore::open(root.to_owned())?);
    store.migrate()?;
    let agents = Arc::new(AgentRuntime::with_native(root.to_owned(), store.clone()));
    let worker = ResidentWorker::start(store, agents.clone())?;
    Ok((agents, worker))
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--relay-mcp") {
        return serve_stdio(
            args.get(2).context("MCP root")?.into(),
            ThreadId(args.get(3).context("MCP scope")?.parse()?),
        );
    }
    ensure!(
        args.iter().any(|a| a == "--execute"),
        "Pass --execute to use the local Codex login and model allowance"
    );
    let root =
        std::env::temp_dir().join(format!("relay-memory-lifecycle-{}", uuid::Uuid::new_v4()));
    println!("Isolated lifecycle data: {}", root.display());
    let store = NativeStore::open(root.clone())?;
    drop(store);
    let db = rusqlite::Connection::open(root.join("relay.sqlite3"))?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute("UPDATE meta SET value=0 WHERE key='observer_enabled'", [])?;
    let (agents, worker) = open(&root)?;
    let first = (|| -> Result<u64> {
        connect(&agents)?;
        turn(
            &agents,
            "这是隔离记忆验收中的虚构项目 Kestrel。我们已经决定本地记忆采用 SQLite，理由是必须离线运行，不能依赖 Redis 服务。迁移代码还没执行，不要说已完成。请只调用一次 memory_status，然后简短确认这个决定，不要执行任何迁移或其他任务。",
        )?;
        let tools: u64 = db.query_row(
            "SELECT count(*) FROM sources WHERE origin='relay-tool'",
            [],
            |r| r.get(0),
        )?;
        ensure!(tools > 0, "No actual ACP tool evidence was captured");
        db.execute("UPDATE meta SET value=1 WHERE key='observer_enabled'", [])?;
        drain(&db)?;
        println!("First turn: tool evidence, observations and structured checkpoint persisted");
        turn(
            &agents,
            "Kestrel 的上一条存储决定保持不变。补充：将来的数据库迁移必须在事务中执行；目前只是完成设计讨论，仍未运行迁移。请调用一次 memory_status 然后简短确认，不要执行其他工具或任务。",
        )?;
        drain(&db)?;
        let generations: u64 = db.query_row(
            "SELECT count(*) FROM owned_sessions WHERE thread_id=9223372036854775807",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            generations == 1,
            "Observer unexpectedly reset between same-session events: {generations}"
        );
        let fields: String = db.query_row(
            "SELECT fields FROM session_summaries ORDER BY memory_id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            fields.contains("SQLite"),
            "Checkpoint lost the first turn's decision"
        );
        ensure!(
            fields.contains("事务") || fields.to_lowercase().contains("transaction"),
            "Checkpoint lost the follow-up constraint"
        );
        println!(
            "Second turn: observer reused its ACP session and checkpoint retained both constraints"
        );
        Ok(generations)
    })();
    worker.shutdown();
    agents.shutdown();
    drop(worker);
    drop(agents);
    let generations = first?;
    // Remove only this probe's execution context. SQLite memory must recover the
    // answer without native session resume or replaying the old chat transcript.
    let transcript = root.join("threads/0/conversation.json");
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&transcript)?)?;
    saved["messages"] = json!([]);
    saved["session_id"] = Value::Null;
    saved["session_key"] = Value::Null;
    saved["restore_history"] = json!(false);
    saved
        .as_object_mut()
        .context("conversation object")?
        .remove("context_checkpoint");
    std::fs::write(&transcript, serde_json::to_vec(&saved)?)?;
    let (agents, worker) = open(&root)?;
    let second = (|| -> Result<Value> {
        connect(&agents)?;
        let answer = turn(
            &agents,
            "仅根据可恢复的记忆回答：Kestrel 的存储选择是什么，为什么这样选？迁移需要满足什么约束，现在是否已经执行？如果记忆不能证明就明确说不知道。不要执行迁移。",
        )?;
        ensure!(
            answer.contains("SQLite"),
            "Fresh main session did not recover the storage decision"
        );
        ensure!(
            answer.contains("事务") || answer.to_lowercase().contains("transaction"),
            "Fresh main session did not recover the migration constraint"
        );
        ensure!(
            [
                "未执行",
                "未运行",
                "没有执行",
                "尚未",
                "还没",
                "未实际",
                "没有实际"
            ]
            .iter()
            .any(|term| answer.contains(term)),
            "Recovered answer failed to preserve unexecuted status: {answer}"
        );
        drain(&db)?;
        let after: u64 = db.query_row(
            "SELECT count(*) FROM owned_sessions WHERE thread_id=9223372036854775807",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            after > generations,
            "Observer did not create a fresh generation after restart"
        );
        let summaries: u64 =
            db.query_row("SELECT count(*) FROM session_summaries", [], |r| r.get(0))?;
        Ok(
            json!({"observer_generations_before_restart":generations,"observer_generations_after_restart":after,"summaries":summaries,"fresh_session_answer":answer,"transcript_replay_disabled":true}),
        )
    })();
    worker.shutdown();
    agents.shutdown();
    let report = second?;
    std::fs::write(
        root.join("lifecycle-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "Lifecycle checks passed. Report: {}",
        root.join("lifecycle-report.json").display()
    );
    Ok(())
}
