//! Explicit real-Harness smoke test. All Relay data stays in an isolated temporary directory.
use relay_core::{ThreadId, agents::*, threads::ThreadService};
use relay_runtime::{AgentRuntime, native::*};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn until(mut check: impl FnMut() -> anyhow::Result<bool>, seconds: u64) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if check()? {
            return Ok(());
        }
        anyhow::ensure!(Instant::now() < deadline, "Native probe timed out");
        std::thread::sleep(Duration::from_millis(250));
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--relay-mcp") {
        return serve_stdio(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("missing root"))?
                .into(),
            ThreadId(
                args.get(3)
                    .ok_or_else(|| anyhow::anyhow!("missing scope"))?
                    .parse()?,
            ),
        );
    }
    anyhow::ensure!(
        args.iter().any(|s| s == "--execute"),
        "Pass --execute to use the local Codex login and model allowance"
    );
    let root = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("relay-native-probe-{}", uuid::Uuid::new_v4()))
        });
    let observer_only = args.iter().any(|s| s == "--observer-only");
    anyhow::ensure!(
        !root.join("relay.sqlite3").exists()
            || (observer_only
                && root
                    .file_name()
                    .is_some_and(|s| s.to_string_lossy().starts_with("relay-native-probe-"))),
        "Probe requires a new isolated data directory or its own --observer-only retry"
    );
    let store = Arc::new(NativeStore::open(root.clone())?);
    store.migrate()?;
    let agents = Arc::new(AgentRuntime::with_native(root.clone(), store.clone()));
    let worker = ResidentWorker::start(store.clone(), agents.clone())?;
    println!("Isolated Relay data: {}", root.display());
    let outcome = (|| -> anyhow::Result<()> {
        agents
            .dispatch(MAIN, AgentCommand::Connect(AgentSource::Auto))
            .map_err(anyhow::Error::msg)?;
        until(
            || {
                let state = agents.snapshot(MAIN);
                anyhow::ensure!(
                    !matches!(
                        state.status,
                        ConnectionStatus::Failed | ConnectionStatus::NeedsAuthentication
                    ),
                    "Harness unavailable: {:?} {:?}",
                    state.status,
                    state.error
                );
                Ok(state.status == ConnectionStatus::Ready)
            },
            120,
        )?;
        println!("Main ACP session ready");
        let db = rusqlite::Connection::open(root.join("relay.sqlite3"))?;
        if !observer_only {
            agents.send_turn(MAIN,"This is an isolated Relay integration check. Do not use shell, browse, edit files, or ask for confirmation. Use only the relay MCP tools. First call memory_settings(enabled=false,runs_per_hour=2). My explicit preference for this test is: prefer concise Chinese replies. Save that as a confirmed global preference with memory_write using the source_id and revision in the current-message evidence, and topics [relay-probe]. Verify it using memory_search. Then call create_task exactly once with request_key=relay-native-probe-task, title=Native probe, instructions=Reply with CHILD_NATIVE_PROBE_OK without using tools or taking other actions. Your final reply should be MAIN_NATIVE_PROBE_READY. The task is asynchronous; do not wait in a tool loop.".into()).map_err(anyhow::Error::msg)?;
            until(
                || {
                    store.refresh()?;
                    let state = agents.snapshot(MAIN);
                    anyhow::ensure!(
                        state.status != ConnectionStatus::Failed,
                        "Main failed: {:?}",
                        state.error
                    );
                    Ok(store
                        .activity()
                        .iter()
                        .any(|t| matches!(t.state.as_str(), "review" | "completed"))
                        && state.status == ConnectionStatus::Ready
                        && state
                            .messages
                            .iter()
                            .filter(|m| {
                                m.role == MessageRole::Assistant
                                    && m.status == MessageStatus::Complete
                            })
                            .count()
                            >= 2)
                },
                240,
            )?;
            let confirmed: u64 = db.query_row(
                "SELECT count(*) FROM memories WHERE status='confirmed'",
                [],
                |r| r.get(0),
            )?;
            until(
                || {
                    let reports:u64=db.query_row("SELECT count(*) FROM requests WHERE request_key LIKE 'relay-result:%' AND state='done'",[],|r|r.get(0))?;
                    Ok(reports == 1)
                },
                20,
            )?;
            anyhow::ensure!(confirmed > 0, "Confirmed memory was not saved");
            let result: String = db.query_row(
                "SELECT summary FROM threads WHERE kind='task' ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                result.contains("CHILD_NATIVE_PROBE_OK"),
                "Child task did not return the expected result"
            );
            println!("Confirmed memory saved; child task finished; one report returned to main");
        }
        agents.send_turn(MAIN,"Continue the isolated integration check. Call memory_settings(enabled=true,runs_per_hour=4). Then reply OBSERVER_ENABLED. Do not use any other tools.".into()).map_err(anyhow::Error::msg)?;
        until(
            || {
                let count: u64 =
                    db.query_row("SELECT count(*) FROM jobs WHERE state='done'", [], |r| {
                        r.get(0)
                    })?;
                Ok(count > 0)
            },
            180,
        )?;
        let candidates: u64 = db.query_row(
            "SELECT count(*) FROM memories WHERE status='candidate'",
            [],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            candidates > 0,
            "Observer did not extract any candidate evidence"
        );
        println!("Observer committed candidate observations with provenance");
        Ok(())
    })();
    worker.shutdown();
    agents.shutdown();
    outcome
}
