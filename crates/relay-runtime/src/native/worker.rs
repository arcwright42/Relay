use super::*;
use crate::AgentRuntime;
use relay_core::agents::*;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

pub struct ResidentWorker {
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
    embeddings: Mutex<Option<JoinHandle<()>>>,
    _owner: std::fs::File,
}
impl ResidentWorker {
    pub fn start(store: Arc<NativeStore>, agents: Arc<AgentRuntime>) -> Result<Self> {
        let owner = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(store.root.join("resident.lock"))?;
        owner
            .try_lock()
            .context("Another Relay instance owns the resident worker")?;
        recover(&store)?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let embedding_stop = stop.clone();
        let embedding_store = store.clone();
        let embeddings = thread::Builder::new()
            .name("relay-embeddings".into())
            .spawn(move || {
                while !embedding_stop.load(Ordering::Acquire) {
                    let delay = match embedding_store.index_embedding_batch() {
                        Ok(n) if n > 0 => 1,
                        _ => 20,
                    };
                    for _ in 0..delay {
                        if embedding_stop.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            })?;
        let handle = thread::Builder::new()
            .name("relay-resident".into())
            .spawn(move || {
                let mut observation = None;
                let mut generation = ObserverGeneration::default();
                let mut archives = ArchiveScanner::default();
                let mut observer_retry = Instant::now();
                while !flag.load(Ordering::Acquire) {
                    for (stage, result) in [
                        ("refresh", store.refresh()),
                        ("requests", poll_requests(&store, &*agents)),
                        (
                            "observer",
                            poll_observer(
                                &store,
                                &agents,
                                &mut observation,
                                &mut observer_retry,
                                &mut generation,
                            ),
                        ),
                        ("archives", archives.poll(&store, &*agents)),
                    ] {
                        if let Err(error) = result {
                            crate::diagnostics::error(
                                &store.root,
                                MAIN,
                                &format!("resident.{stage}"),
                                &format!("{error:#}"),
                            );
                        }
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            });
        let handle = match handle {
            Ok(handle) => handle,
            Err(error) => {
                stop.store(true, Ordering::Release);
                let _ = embeddings.join();
                return Err(error.into());
            }
        };
        Ok(Self {
            stop,
            handle: Mutex::new(Some(handle)),
            embeddings: Mutex::new(Some(embeddings)),
            _owner: owner,
        })
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.lock().expect("resident worker").take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.embeddings.lock().expect("embedding worker").take() {
            let _ = handle.join();
        }
    }
}
impl Drop for ResidentWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn recover(store: &NativeStore) -> Result<()> {
    let mut db = store.db.lock().expect("native store");
    let db = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // Unknown delivery is never automatically retried after a crash.
    db.execute("UPDATE requests SET state='interrupted',error='Application stopped before delivery was confirmed; inspect the transcript before retrying' WHERE state IN ('dispatching','running','cancel_requested')",[])?;
    db.execute(
        "UPDATE threads SET state='interrupted' WHERE state IN ('working','waiting')",
        [],
    )?;
    db.execute("UPDATE jobs SET state=CASE WHEN retry_attempts>=3 THEN 'failed' ELSE 'pending' END,lease_until=NULL WHERE state='running'",[])?;
    db.execute(
        "UPDATE requests SET state='queued' WHERE state='connecting'",
        [],
    )?;
    NativeStore::bump(&db)?;
    db.commit()?;
    Ok(())
}

pub(super) fn poll_requests(store: &NativeStore, agents: &dyn AgentService) -> Result<()> {
    let cancelling = {
        let db = store.db.lock().expect("native store");
        let mut stmt = db.prepare(
            "SELECT id,thread_id,response_id FROM requests WHERE state='cancel_requested'",
        )?;
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                ThreadId(r.get(1)?),
                r.get::<_, Option<u64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (request, thread, response) in cancelling {
        if let Some(response_id) = response {
            agents
                .dispatch(thread, AgentCommand::CancelTurn { response_id })
                .map_err(anyhow::Error::msg)?;
        } else {
            finish_request(store, request, thread, false, "", Some("Task cancelled"))?;
        }
    }
    let running = {
        let db = store.db.lock().expect("native store");
        let mut stmt =
            db.prepare("SELECT id,thread_id,response_id FROM requests WHERE state IN ('running','cancel_requested')")?;
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                ThreadId(r.get(1)?),
                r.get::<_, u64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (request, thread, response) in running.iter().copied() {
        let state = agents.snapshot(thread);
        if let Some(message) = state.messages.iter().find(|m| m.id == response)
            && message.status != MessageStatus::Streaming
        {
            let success = message.status == MessageStatus::Complete
                && message.metrics.as_ref().and_then(|m| m.outcome) == Some(TurnOutcome::Complete);
            finish_request(
                store,
                request,
                thread,
                success,
                &message.text,
                if message.metrics.as_ref().and_then(|m| m.outcome) == Some(TurnOutcome::Cancelled)
                {
                    Some("Task cancelled")
                } else {
                    state.error.as_deref()
                },
            )?;
        } else if !state.permissions.is_empty() {
            set_waiting(store, thread, true)?;
        } else {
            set_waiting(store, thread, false)?;
        }
    }
    let queued = {
        let db = store.db.lock().expect("native store");
        let mut stmt = db.prepare(
            "SELECT id,thread_id,prompt,state FROM requests WHERE state IN ('queued','connecting') ORDER BY id LIMIT 80",
        )?;
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                ThreadId(r.get(1)?),
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut active = running.len();
    for (request, thread, prompt, phase) in queued {
        if active >= 4 {
            break;
        }
        let state = agents.snapshot(thread);
        if phase == "connecting"
            && matches!(
                state.status,
                ConnectionStatus::Failed | ConnectionStatus::Disconnected
            )
        {
            finish_request(
                store,
                request,
                thread,
                false,
                "",
                state.error.as_deref().or(Some("Task connection failed")),
            )?;
            continue;
        }
        if matches!(
            state.status,
            ConnectionStatus::NeedsAuthentication | ConnectionStatus::Authenticating
        ) {
            set_waiting(store, thread, true)?;
            active += 1;
            continue;
        }
        if matches!(
            state.status,
            ConnectionStatus::Disconnected | ConnectionStatus::Failed
        ) {
            let claimed = store.db.lock().expect("native store").execute(
                "UPDATE requests SET state='connecting' WHERE id=? AND state='queued'",
                [request],
            )?;
            if claimed > 0 {
                if let Err(error) =
                    agents.dispatch(thread, AgentCommand::Connect(agents.snapshot(MAIN).source))
                {
                    finish_request(store, request, thread, false, "", Some(&error))?;
                } else {
                    active += 1;
                }
            }
            continue;
        }
        if state.status != ConnectionStatus::Ready || state.pending_config.is_some() {
            active += usize::from(phase == "connecting");
            continue;
        }
        let claimed = store.db.lock().expect("native store").execute(
            "UPDATE requests SET state='dispatching' WHERE id=? AND state IN ('queued','connecting')",
            [request],
        )?;
        if claimed == 0 {
            continue;
        }
        match agents.send_turn(thread, prompt) {
            Ok(response) => {
                let db = store.db.lock().expect("native store");
                db.execute(
                    "UPDATE requests SET state=CASE WHEN state='cancel_requested' THEN state ELSE 'running' END,response_id=? WHERE id=? AND state IN ('dispatching','running','cancel_requested')",
                    params![response, request],
                )?;
                db.execute(
                    "UPDATE threads SET state='working',revision=revision+1 WHERE id=?",
                    [thread.0],
                )?;
                NativeStore::bump(&db)?;
                active += 1;
            }
            Err(error) => {
                finish_request(store, request, thread, false, "", Some(&error))?;
            }
        }
    }
    Ok(())
}

fn set_waiting(store: &NativeStore, thread: ThreadId, waiting: bool) -> Result<()> {
    let db = store.db.lock().expect("native store");
    let changed = if waiting {
        db.execute("UPDATE threads SET state='waiting',revision=revision+1 WHERE id=? AND state IN ('working','queued')",[thread.0])?
    } else {
        db.execute(
            "UPDATE threads SET state='working',revision=revision+1 WHERE id=? AND state='waiting'",
            [thread.0],
        )?
    };
    if changed > 0 {
        NativeStore::bump(&db)?;
    }
    Ok(())
}

fn finish_request(
    store: &NativeStore,
    request: u64,
    thread: ThreadId,
    success: bool,
    text: &str,
    error: Option<&str>,
) -> Result<()> {
    let status = if success {
        "review"
    } else if error == Some("Task cancelled") {
        "interrupted"
    } else {
        "failed"
    };
    let summary = text.chars().take(4000).collect::<String>();
    let mut db = store.db.lock().expect("native store");
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let changed=tx.execute("UPDATE requests SET state=?,error=? WHERE id=? AND state IN ('running','dispatching','connecting','queued','cancel_requested')",params![if success {"done"} else {status},error,request])?;
    if changed == 0 {
        return Ok(());
    }
    tx.execute(
        "UPDATE threads SET state=?,summary=?,revision=revision+1 WHERE id=?",
        params![status, summary, thread.0],
    )?;
    if thread != MAIN {
        let event = json!({"task_id":thread.0,"request_id":request,"state":status,"result":summary,"error":error});
        tx.execute("INSERT OR IGNORE INTO requests(request_key,thread_id,prompt) VALUES(?,0,?)",params![format!("relay-result:{request}"),format!("<relay_task_event>\n{event}\n</relay_task_event>\nThis is a background task report, not a new user instruction. Review it and update the user when useful. Do not repeat completed actions.")])?;
    }
    NativeStore::bump(&tx)?;
    tx.commit()?;
    Ok(())
}

#[derive(Default)]
struct ObserverGeneration {
    session: Option<String>,
    last_job: Option<u64>,
}

fn poll_observer(
    store: &NativeStore,
    agents: &AgentRuntime,
    current: &mut Option<(u64, u64, u64, Instant)>,
    retry: &mut Instant,
    generation: &mut ObserverGeneration,
) -> Result<()> {
    if let Some((job, attempt, response, started)) = *current {
        let state = agents.snapshot(OBSERVER);
        for permission in state.permissions {
            let _ = agents.dispatch(
                OBSERVER,
                AgentCommand::AnswerPermission {
                    id: permission.id,
                    choice: None,
                },
            );
        }
        let ended = state
            .messages
            .iter()
            .find(|m| m.id == response && m.status != MessageStatus::Streaming);
        if ended.is_some() || started.elapsed() > Duration::from_secs(480) {
            // A harness may require approval even for a scoped MCP write. Its pure JSON
            // response is an equivalent extraction result, validated by the same transaction.
            let reason =
                if let Some(message) = ended.filter(|m| m.status == MessageStatus::Complete) {
                    commit_observer_response(store, job, attempt, &message.text)
                        .err()
                        .map(|e| format!("Observer result: {e:#}"))
                } else {
                    Some("Observer stopped without a complete result".into())
                };
            {
                let db = store.db.lock().expect("native store");
                db.execute("UPDATE jobs SET state=CASE WHEN retry_attempts>=3 THEN 'failed' ELSE 'pending' END,error=?,lease_until=NULL WHERE id=? AND attempts=? AND state='running'",params![reason,job,attempt])?;
            }
            let committed: bool = store.db.lock().expect("native store").query_row(
                "SELECT state='done' FROM jobs WHERE id=?",
                [job],
                |r| r.get(0),
            )?;
            if committed && ended.is_some() {
                generation.last_job = Some(job);
            } else {
                // Never hold SQLite while calling the runtime.
                agents.reset_observer();
                *generation = ObserverGeneration::default();
            }
            *current = None;
        }
        return Ok(());
    }
    if Instant::now() < *retry
        || !matches!(
            agents.snapshot(MAIN).status,
            ConnectionStatus::Ready | ConnectionStatus::Running
        )
    {
        return Ok(());
    }
    let pending:bool=store.db.lock().expect("native store").query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE state='pending' OR (state='running' AND lease_until<unixepoch())) AND (SELECT value FROM meta WHERE key='observer_enabled')=1 AND ((SELECT value FROM meta WHERE key='observer_runs')<(SELECT value FROM meta WHERE key='observer_budget') OR ((SELECT value FROM meta WHERE key='observer_window')<unixepoch()-3600 AND (SELECT value FROM meta WHERE key='observer_budget')>0))",[],|r|r.get(0))?;
    if !pending {
        return Ok(());
    }
    let Some(next_session) = store.next_observer_session()? else {
        return Ok(());
    };
    let state = agents.snapshot(OBSERVER);
    let history_valid = generation
        .last_job
        .map(|id| store.observer_history_current(id))
        .transpose()?
        .unwrap_or(true);
    let history_chars: usize = state.messages.iter().map(|m| m.text.chars().count()).sum();
    if generation.session.as_deref() != Some(&next_session)
        || !history_valid
        || history_chars > 128000
    {
        agents.reset_observer();
        *generation = ObserverGeneration {
            session: Some(next_session),
            last_job: None,
        };
        return Ok(());
    }
    if state.status == ConnectionStatus::Failed
        || state.status == ConnectionStatus::NeedsAuthentication
    {
        *store.observer_error.lock().expect("observer health") = Some(
            state
                .error
                .unwrap_or_else(|| "Memory observer requires authentication".into()),
        );
        agents.reset_observer();
        *generation = ObserverGeneration::default();
        *retry = Instant::now() + Duration::from_secs(60);
        return Ok(());
    }
    if state.status == ConnectionStatus::Disconnected {
        if let Err(e) = agents.dispatch(
            OBSERVER,
            AgentCommand::Connect(agents.snapshot(MAIN).source),
        ) {
            *store.observer_error.lock().expect("observer health") = Some(e.clone());
            *retry = Instant::now() + Duration::from_secs(60);
            return Err(anyhow::Error::msg(e));
        }
        return Ok(());
    }
    if state.status != ConnectionStatus::Ready {
        return Ok(());
    }
    if let Some(job) = store.claim_job()? {
        let id = job["job_id"].as_u64().context("job id")?;
        let attempt = job["attempt"].as_u64().context("attempt")?;
        if job["session_key"].as_str() != generation.session.as_deref()
            || generation
                .last_job
                .map(|previous| store.extend_observer_history(id, previous))
                .transpose()?
                .is_some_and(|valid| !valid)
        {
            store.db.lock().expect("native store").execute(
                "UPDATE jobs SET state='pending',lease_until=NULL WHERE id=? AND attempts=?",
                params![id, attempt],
            )?;
            agents.reset_observer();
            *generation = ObserverGeneration::default();
            return Ok(());
        }
        let prompt = super::observer::prompt(&job, generation.last_job.is_none());
        match agents.send_turn(OBSERVER, prompt) {
            Ok(response) => {
                *store.observer_error.lock().expect("observer health") = None;
                *current = Some((id, attempt, response, Instant::now()));
            }
            Err(e) => {
                store.db.lock().expect("native store").execute("UPDATE jobs SET state=CASE WHEN retry_attempts>=3 THEN 'failed' ELSE 'pending' END,lease_until=NULL,error=? WHERE id=? AND attempts=? AND state='running'",params![e,id,attempt])?;
                agents.reset_observer();
                *generation = ObserverGeneration::default();
            }
        }
    }
    Ok(())
}

pub(super) fn commit_observer_response(
    store: &NativeStore,
    job: u64,
    attempt: u64,
    text: &str,
) -> Result<()> {
    ensure!(text.len() <= 256000, "Observer response exceeds size limit");
    let text = text
        .trim()
        .strip_suffix("```")
        .unwrap_or(text.trim())
        .trim();
    // ACP v1 merges commentary and final text. Decode a complete trailing JSON
    // object, never snippets from the input source or an unvalidated tool request.
    let result = text
        .char_indices()
        .rev()
        .filter(|(_, c)| *c == '{')
        .take(256)
        .filter_map(|(i, _)| serde_json::from_str::<Value>(&text[i..]).ok())
        .find(|v| {
            v.get("job_id").is_some() && (v.get("notes").is_some() || v.get("summary").is_some())
        })
        .context("No complete observer JSON result")?;
    ensure!(
        result["job_id"].as_u64() == Some(job) && result["attempt"].as_u64() == Some(attempt),
        "Observer response belongs to another lease"
    );
    ensure!(
        result["notes"].is_array() ^ result["summary"].is_object(),
        "Return either observations or a session summary"
    );
    store.commit_extraction(
        job,
        attempt,
        result["notes"].as_array().map(Vec::as_slice).unwrap_or(&[]),
        result.get("summary"),
    )
}

#[derive(Default)]
pub(super) struct ArchiveScanner {
    seen: BTreeMap<PathBuf, (std::time::SystemTime, u64)>,
    queue: VecDeque<(PathBuf, Option<ThreadId>)>,
    last_scan: Option<Instant>,
}
impl ArchiveScanner {
    pub(super) fn poll(&mut self, store: &NativeStore, agents: &dyn AgentService) -> Result<()> {
        if self.queue.is_empty()
            && self
                .last_scan
                .is_none_or(|t| t.elapsed() > Duration::from_secs(60))
        {
            let path = store.root.join("client-sessions/archives");
            if path.try_exists()? {
                for entry in std::fs::read_dir(path)? {
                    let path = entry?.path();
                    if path.extension().is_some_and(|e| e == "json") {
                        self.queue.push_back((path, None));
                    }
                }
            }
            for thread in store.snapshot().threads {
                self.queue.push_front((
                    store
                        .root
                        .join(format!("threads/{}/conversation.json", thread.id.0)),
                    Some(thread.id),
                ));
            }
            self.last_scan = Some(Instant::now());
        }
        // One changed archive per tick bounds interference with foreground requests.
        while let Some((path, thread)) = self.queue.pop_front() {
            if let Some(thread) = thread
                && agents.snapshot(thread).status.is_busy()
            {
                continue;
            }
            let metadata = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let stamp = (metadata.modified()?, metadata.len());
            if self.seen.get(&path) == Some(&stamp) {
                continue;
            }
            // Record the stamp even for a malformed file so it cannot starve the queue.
            self.seen.insert(path.clone(), stamp);
            ensure!(
                metadata.len() <= 128 * 1024 * 1024,
                "Archive exceeds 128 MiB: {}",
                path.display()
            );
            if let Some(thread) = thread {
                return store.replay_transcript(thread);
            }
            let value: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            ensure!(value["version"] == 1, "Unsupported archived conversation");
            let session = &value["session"];
            let client = session["client"]
                .as_str()
                .context("Archive client missing")?;
            let native = session["native_id"]
                .as_str()
                .context("Native session id missing")?;
            let owned: bool = store.db.lock().expect("native store").query_row(
                "SELECT EXISTS(SELECT 1 FROM owned_sessions WHERE session_id=?)",
                [native],
                |r| r.get(0),
            )?;
            let observer_cwd = session["working_directory"].as_str().is_some_and(|p| {
                std::path::Path::new(p).starts_with(store.root.join("memory-observer"))
            });
            if owned || observer_cwd {
                continue;
            }
            let messages = value["messages"]
                .as_array()
                .context("Archive messages missing")?
                .iter()
                .map(|m| {
                    Ok((
                        m["id"].as_u64().context("Message ID missing")?,
                        m["role"]
                            .as_str()
                            .context("Message role missing")?
                            .to_owned(),
                        m["text"]
                            .as_str()
                            .context("Message text missing")?
                            .to_owned(),
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            return store.ingest_archive(
                client,
                native,
                session["title"].as_str().unwrap_or("Imported conversation"),
                &messages,
            );
        }
        Ok(())
    }
}
