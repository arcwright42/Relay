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

/// Coordinates tasks only. The selected memory engine owns all memory background work.
pub struct ResidentWorker {
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
    memory: Box<dyn crate::memory::MemoryWorker>,
    _owner: std::fs::File,
}
impl ResidentWorker {
    pub fn start(store: Arc<ResidentStore>, agents: Arc<AgentRuntime>) -> Result<Self> {
        let provider = agents
            .memory_provider()
            .context("Memory provider is not configured")?;
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
        let memory = provider.clone().start()?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::Builder::new()
            .name("relay-resident".into())
            .spawn(move || {
                let mut archives = ArchiveScanner::default();
                while !flag.load(Ordering::Acquire) {
                    let work = (|| -> Result<()> {
                        store.refresh()?;
                        poll_requests(&store, &*agents)?;
                        if provider.imports_history() {
                            archives.poll(&store, &*agents, &*provider)?;
                        }
                        Ok(())
                    })();
                    if let Err(error) = work {
                        crate::diagnostics::error(
                            &store.root,
                            MAIN,
                            "resident.tasks",
                            &format!("{error:#}"),
                        );
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            });
        let handle = match handle {
            Ok(h) => h,
            Err(e) => {
                memory.shutdown();
                return Err(e.into());
            }
        };
        Ok(Self {
            stop,
            handle: Mutex::new(Some(handle)),
            memory,
            _owner: owner,
        })
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.lock().expect("resident worker").take() {
            let _ = h.join();
        }
        self.memory.shutdown();
    }
}
impl Drop for ResidentWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn recover(store: &ResidentStore) -> Result<()> {
    let mut db = store.db.lock().expect("resident store");
    let db = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // Unknown delivery is never automatically retried after a crash.
    db.execute("UPDATE requests SET state='interrupted',error='Application stopped before delivery was confirmed; inspect the transcript before retrying' WHERE state IN ('dispatching','running','cancel_requested')",[])?;
    db.execute(
        "UPDATE threads SET state='interrupted' WHERE state IN ('working','waiting')",
        [],
    )?;
    db.execute(
        "UPDATE requests SET state='queued' WHERE state='connecting'",
        [],
    )?;
    ResidentStore::bump(&db)?;
    db.commit()?;
    Ok(())
}

pub(super) fn poll_requests(store: &ResidentStore, agents: &dyn AgentService) -> Result<()> {
    let cancelling = {
        let db = store.db.lock().expect("resident store");
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
        let db = store.db.lock().expect("resident store");
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
        let db = store.db.lock().expect("resident store");
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
            let claimed = store.db.lock().expect("resident store").execute(
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
        let claimed = store.db.lock().expect("resident store").execute(
            "UPDATE requests SET state='dispatching' WHERE id=? AND state IN ('queued','connecting')",
            [request],
        )?;
        if claimed == 0 {
            continue;
        }
        match agents.send_turn(thread, prompt) {
            Ok(response) => {
                let db = store.db.lock().expect("resident store");
                db.execute(
                    "UPDATE requests SET state=CASE WHEN state='cancel_requested' THEN state ELSE 'running' END,response_id=? WHERE id=? AND state IN ('dispatching','running','cancel_requested')",
                    params![response, request],
                )?;
                db.execute(
                    "UPDATE threads SET state='working',revision=revision+1 WHERE id=?",
                    [thread.0],
                )?;
                ResidentStore::bump(&db)?;
                active += 1;
            }
            Err(error) => {
                finish_request(store, request, thread, false, "", Some(&error))?;
            }
        }
    }
    Ok(())
}

fn set_waiting(store: &ResidentStore, thread: ThreadId, waiting: bool) -> Result<()> {
    let db = store.db.lock().expect("resident store");
    let changed = if waiting {
        db.execute("UPDATE threads SET state='waiting',revision=revision+1 WHERE id=? AND state IN ('working','queued')",[thread.0])?
    } else {
        db.execute(
            "UPDATE threads SET state='working',revision=revision+1 WHERE id=? AND state='waiting'",
            [thread.0],
        )?
    };
    if changed > 0 {
        ResidentStore::bump(&db)?;
    }
    Ok(())
}

fn finish_request(
    store: &ResidentStore,
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
    let mut db = store.db.lock().expect("resident store");
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
    ResidentStore::bump(&tx)?;
    tx.commit()?;
    Ok(())
}

#[derive(Default)]
pub(super) struct ArchiveScanner {
    seen: BTreeMap<PathBuf, (std::time::SystemTime, u64)>,
    queue: VecDeque<(PathBuf, Option<ThreadId>)>,
    last_scan: Option<Instant>,
}
impl ArchiveScanner {
    pub(super) fn poll(
        &mut self,
        store: &ResidentStore,
        agents: &dyn AgentService,
        provider: &dyn crate::memory::MemoryProvider,
    ) -> Result<()> {
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
                let saved = crate::store::load(&store.root, thread)?;
                let directory = saved
                    .cwd
                    .unwrap_or_else(|| agents.snapshot(thread).working_directory);
                let messages = saved
                    .messages
                    .into_iter()
                    .map(|m| crate::memory::MemoryMessage::from(&m.into_message()))
                    .collect::<Vec<_>>();
                for (index, message) in messages.iter().enumerate() {
                    let event = if message.role == "user" {
                        crate::memory::MemoryEvent::User {
                            thread,
                            message: message.id,
                            text: message.text.clone(),
                            directory: directory.clone(),
                        }
                    } else {
                        crate::memory::MemoryEvent::Turn {
                            thread,
                            messages: messages[index.saturating_sub(1)..=index].to_vec(),
                            directory: directory.clone(),
                        }
                    };
                    if let Err(error) = provider.record(&event) {
                        self.seen.remove(&path);
                        return Err(error);
                    }
                }
                return Ok(());
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
            let owned: bool = store.db.lock().expect("resident store").query_row(
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
            let captured = provider.record(&crate::memory::MemoryEvent::Archive {
                directory: session["working_directory"].as_str().map(PathBuf::from),
                client: client.into(),
                session: native.into(),
                title: session["title"]
                    .as_str()
                    .unwrap_or("Imported conversation")
                    .into(),
                messages,
            });
            if let Err(error) = captured {
                self.seen.remove(&path);
                return Err(error);
            }
            return Ok(());
        }
        Ok(())
    }
}
