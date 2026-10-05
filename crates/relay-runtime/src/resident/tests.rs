use super::*;
pub(super) struct Sandbox {
    pub(super) root: PathBuf,
    pub(super) store: ResidentStore,
}

impl Sandbox {
    pub(super) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("relay-resident-test-{}", uuid::Uuid::new_v4()));
        let store = ResidentStore::open(root.clone()).unwrap();
        Self { root, store }
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn main_room_and_task_identity_survive_reopen_without_prompt_classification() {
    let s = Sandbox::new();
    let id = s
        .store
        .create_task("task-a", "报告", "研究 memory provider")
        .unwrap();
    assert_ne!(id, MAIN.0);
    assert_eq!(
        s.store
            .create_task("task-a", "报告", "研究 memory provider")
            .unwrap(),
        id
    );
    assert!(s.store.create_task("task-a", "报告", "different").is_err());
    let reopened = ResidentStore::open(s.root.clone()).unwrap();
    assert_eq!(reopened.snapshot().threads[0].id, MAIN);
    assert_eq!(reopened.snapshot().threads.len(), 2);
    reopened.enqueue("follow-a", id, "用中文").unwrap();
    reopened.enqueue("follow-a", id, "用中文").unwrap();
    let n: i64 = reopened
        .db
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn direct_task_followups_share_the_outbox_without_duplicating_worker_requests() {
    let s = Sandbox::new();
    let task = ThreadId(s.store.create_task("created", "Title", "Work").unwrap());
    s.store
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE requests SET state='dispatching' WHERE thread_id=?",
            [task.0],
        )
        .unwrap();
    s.store.track_turn(task, 2, "Work").unwrap();
    s.store.track_turn(task, 2, "Work").unwrap();
    s.store
        .track_turn(task, 4, "A direct UI follow-up")
        .unwrap();
    let count: u64 = s
        .store
        .db
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn cancellation_before_dispatch_never_sends_the_task_and_marks_it_interrupted() {
    let s = Sandbox::new();
    let task = ThreadId(
        s.store
            .create_task("cancel", "Cancel me", "Do work")
            .unwrap(),
    );
    s.store
        .call_task(MAIN, "cancel_task", &json!({"task_id":task.0}))
        .unwrap();
    let agents = TaskAgents::default();
    worker::poll_requests(&s.store, &agents).unwrap();
    assert!(!agents.sent.lock().unwrap().contains(&task));
    s.store.refresh().unwrap();
    assert_eq!(s.store.activity()[0].state, "interrupted");
}

#[test]
fn cancellation_racing_with_dispatch_keeps_its_response_binding() {
    let s = Sandbox::new();
    let task = ThreadId(
        s.store
            .create_task("cancel", "Cancel me", "Do work")
            .unwrap(),
    );
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE requests SET state='dispatching'", [])
        .unwrap();
    s.store
        .call_task(MAIN, "cancel_task", &json!({"task_id":task.0}))
        .unwrap();
    s.store.track_turn(task, 2, "Do work").unwrap();
    let db = s.store.db.lock().unwrap();
    let (count, state, response): (u64, String, u64) = db
        .query_row("SELECT count(*),state,response_id FROM requests", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(
        (count, state.as_str(), response),
        (1, "cancel_requested", 2)
    );
}

#[derive(Default)]
struct TaskAgents {
    states: Mutex<std::collections::BTreeMap<ThreadId, relay_core::agents::AgentSnapshot>>,
    sent: Mutex<Vec<ThreadId>>,
}
impl relay_core::agents::AgentService for TaskAgents {
    fn revision(&self) -> u64 {
        1
    }
    fn snapshot(&self, id: ThreadId) -> relay_core::agents::AgentSnapshot {
        self.states
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_else(|| relay_core::agents::AgentSnapshot {
                status: relay_core::agents::ConnectionStatus::Ready,
                ..Default::default()
            })
    }
    fn dispatch(
        &self,
        id: ThreadId,
        command: relay_core::agents::AgentCommand,
    ) -> std::result::Result<(), String> {
        if matches!(command, relay_core::agents::AgentCommand::Connect(_)) {
            self.states.lock().unwrap().insert(
                id,
                relay_core::agents::AgentSnapshot {
                    status: relay_core::agents::ConnectionStatus::Failed,
                    error: Some("connection failed".into()),
                    ..Default::default()
                },
            );
        }
        Ok(())
    }
    fn send_turn(&self, id: ThreadId, _: String) -> std::result::Result<u64, String> {
        use relay_core::agents::*;
        self.sent.lock().unwrap().push(id);
        self.states.lock().unwrap().insert(
            id,
            AgentSnapshot {
                status: ConnectionStatus::Running,
                messages: vec![ChatMessage {
                    id: 42,
                    role: MessageRole::Assistant,
                    text: String::new(),
                    status: MessageStatus::Streaming,
                    tools: vec![],
                    metrics: None,
                }],
                ..Default::default()
            },
        );
        Ok(42)
    }
}

#[test]
fn task_completion_correlates_response_and_delivers_one_durable_report() {
    use relay_core::agents::*;
    let s = Sandbox::new();
    let task = ThreadId(s.store.create_task("task", "Title", "Work").unwrap());
    let agents = TaskAgents::default();
    agents.states.lock().unwrap().insert(
        MAIN,
        AgentSnapshot {
            status: ConnectionStatus::Running,
            ..Default::default()
        },
    );
    worker::poll_requests(&s.store, &agents).unwrap();
    worker::poll_requests(&s.store, &agents).unwrap();
    assert_eq!(*agents.sent.lock().unwrap(), [task]);
    {
        let mut states = agents.states.lock().unwrap();
        let state = states.get_mut(&task).unwrap();
        state.messages.push(ChatMessage {
            id: 99,
            role: MessageRole::Assistant,
            text: "unrelated later message".into(),
            status: MessageStatus::Complete,
            tools: vec![],
            metrics: Some(TurnMetrics {
                outcome: Some(TurnOutcome::Complete),
                ..Default::default()
            }),
        });
    }
    worker::poll_requests(&s.store, &agents).unwrap();
    let count = || {
        s.store
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM requests WHERE request_key LIKE 'relay-result:%'",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap()
    };
    assert_eq!(count(), 0);
    {
        let mut states = agents.states.lock().unwrap();
        let state = states.get_mut(&task).unwrap();
        state.messages[0].status = MessageStatus::Complete;
        state.messages[0].text = "Result with evidence".into();
        state.messages[0].metrics = Some(TurnMetrics {
            outcome: Some(TurnOutcome::Complete),
            ..Default::default()
        });
        state.status = ConnectionStatus::Ready;
    }
    worker::poll_requests(&s.store, &agents).unwrap();
    worker::poll_requests(&s.store, &agents).unwrap();
    assert_eq!(count(), 1);
    s.store.refresh().unwrap();
    assert_eq!(s.store.activity()[0].state, "review");
    assert_eq!(s.store.activity()[0].summary, "Result with evidence");
    agents.states.lock().unwrap().remove(&MAIN);
    worker::poll_requests(&s.store, &agents).unwrap();
    worker::recover(&s.store).unwrap();
    worker::poll_requests(&s.store, &agents).unwrap();
    assert_eq!(*agents.sent.lock().unwrap(), [task, MAIN]);
}

#[test]
fn connection_failure_is_terminal_and_unknown_delivery_is_not_replayed() {
    use relay_core::agents::*;
    let s = Sandbox::new();
    let task = ThreadId(s.store.create_task("first", "Title", "Work").unwrap());
    let agents = TaskAgents::default();
    agents
        .states
        .lock()
        .unwrap()
        .insert(task, AgentSnapshot::default());
    worker::poll_requests(&s.store, &agents).unwrap();
    worker::poll_requests(&s.store, &agents).unwrap();
    s.store.refresh().unwrap();
    assert_eq!(s.store.activity()[0].state, "failed");
    let next = s.store.enqueue("next", task.0, "Retry explicitly").unwrap();
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE requests SET state='dispatching' WHERE id=?", [next])
        .unwrap();
    worker::recover(&s.store).unwrap();
    worker::poll_requests(&s.store, &agents).unwrap();
    assert!(!agents.sent.lock().unwrap().contains(&task));
}

#[test]
fn retired_schema_upgrade_preserves_memory_bytes_and_message_identity() {
    for version in [1, 2] {
        let s = Sandbox::new();
        {
            let db = s.store.db.lock().unwrap();
            db.execute("UPDATE meta SET value=? WHERE key='version'", [version])
                .unwrap();
            db.execute_batch("INSERT INTO threads(id,kind,name) VALUES(7,'task','Existing task'),(9223372036854775807,'observer','Old observer');
                CREATE TABLE sources(id INTEGER PRIMARY KEY,thread_id INTEGER,source_key TEXT,body TEXT,forgotten INTEGER);
                INSERT INTO sources VALUES(1,7,'relay:7:tool:84:read:part:0','旧来源',1);
                CREATE TABLE memories(id INTEGER PRIMARY KEY,body TEXT,status TEXT);
                INSERT INTO memories VALUES(1,'旧记忆保持原样','confirmed');
                CREATE TABLE jobs(id INTEGER PRIMARY KEY,state TEXT);
                INSERT INTO jobs VALUES(1,'pending');
                INSERT INTO owned_sessions VALUES('old-session',7);
                INSERT INTO requests(request_key,thread_id,prompt) VALUES('old-request',7,'Existing queued work');").unwrap();
            if version == 1 {
                db.execute("DROP TABLE message_sequences", []).unwrap();
            } else {
                db.execute("INSERT INTO message_sequences VALUES(7,501)", [])
                    .unwrap();
            }
        }
        let reopened = ResidentStore::open(s.root.clone()).unwrap();
        assert!(reopened.thread(ThreadId(7)).is_some());
        assert!(reopened.thread(RETIRED_OBSERVER).is_none());
        assert!(
            reopened
                .enqueue("forbidden", RETIRED_OBSERVER.0, "Do not restart")
                .is_err()
        );
        let expected = if version == 1 { 85 } else { 501 };
        assert_eq!(
            reopened.reserve_message_ids(ThreadId(7), 1).unwrap(),
            expected
        );
        let again = ResidentStore::open(s.root.clone()).unwrap();
        assert_eq!(
            again.reserve_message_ids(ThreadId(7), 1).unwrap(),
            expected + 2
        );
        let db = again.db.lock().unwrap();
        let row: (String,String,String,i64) = db.query_row(
            "SELECT s.body,m.body,j.state,(SELECT value FROM meta WHERE key='version') FROM sources s,memories m,jobs j", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            row,
            (
                "旧来源".into(),
                "旧记忆保持原样".into(),
                "pending".into(),
                3
            )
        );
        assert_eq!(
            db.query_row(
                "SELECT session_id FROM owned_sessions WHERE thread_id=7",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "old-session"
        );
    }
}

#[test]
fn future_database_is_rejected_before_schema_changes() {
    let s = Sandbox::new();
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE meta SET value=99 WHERE key='version'", [])
        .unwrap();
    let before: String = s
        .store
        .db
        .lock()
        .unwrap()
        .query_row("SELECT group_concat(sql) FROM sqlite_master", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(ResidentStore::open(s.root.clone()).is_err());
    let db = s.store.db.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM meta WHERE key='version'", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        99
    );
    assert_eq!(
        db.query_row("SELECT group_concat(sql) FROM sqlite_master", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        before
    );
}

#[test]
fn legacy_project_import_preserves_files_without_reextracting_old_memory() {
    let s = Sandbox::new();
    let bytes = serde_json::to_vec(&json!({"version":3,"projects":[{"id":7,"name":"旧项目","instructions":"仅这个任务","context":[],"memory":[{"content":"旧决定"}]}]})).unwrap();
    std::fs::write(s.root.join("projects.json"), &bytes).unwrap();
    let transcript = crate::store::SavedThread {
        version: 3,
        session_id: Some("old-harness".into()),
        session_key: Some("old-key".into()),
        ..Default::default()
    };
    crate::store::write_json(&s.root.join("projects/7/conversation.json"), &transcript).unwrap();
    let original = std::fs::read(s.root.join("projects/7/conversation.json")).unwrap();
    s.store.migrate().unwrap();
    s.store.migrate().unwrap();
    assert_eq!(std::fs::read(s.root.join("projects.json")).unwrap(), bytes);
    assert_eq!(
        std::fs::read(s.root.join("projects/7/conversation.json")).unwrap(),
        original
    );
    assert_eq!(s.store.snapshot().threads.len(), 2);
    assert!(s.store.thread(MAIN).unwrap().instructions.is_empty());
    let saved = crate::store::load(&s.root, ThreadId(7)).unwrap();
    assert!(saved.session_id.is_none() && saved.session_key.is_none() && saved.restore_history);
    let db = s.store.db.lock().unwrap();
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name IN ('sources','memories','jobs','memory_fts','embedding_jobs'))", [], |r|r.get::<_,bool>(0)).unwrap());
}

#[test]
fn task_tools_keep_scope_and_review_boundaries() {
    let s = Sandbox::new();
    let a = ThreadId(s.store.create_task("a", "First", "Work").unwrap());
    let b = ThreadId(s.store.create_task("b", "Second", "Other work").unwrap());
    assert_eq!(task_tools(MAIN).len(), 6);
    assert_eq!(task_tools(a).len(), 2);
    assert!(task_tools(RETIRED_OBSERVER).is_empty());
    assert!(
        s.store
            .call_task(
                a,
                "create_task",
                &json!({"request_key":"recursive","title":"No","instructions":"No"})
            )
            .is_err()
    );
    assert!(
        s.store
            .call_task(a, "inspect_task", &json!({"task_id":b.0}))
            .is_err()
    );
    assert!(
        s.store
            .call_task(
                a,
                "update_task",
                &json!({"task_id":a.0,"state":"completed","summary":"Unreviewed"})
            )
            .is_err()
    );
    assert!(
        s.store
            .call_task(
                MAIN,
                "update_task",
                &json!({"task_id":a.0,"state":"completed","summary":"Still queued"})
            )
            .is_err()
    );
    s.store
        .call_task(
            a,
            "update_task",
            &json!({"task_id":a.0,"state":"review","summary":"Review evidence"}),
        )
        .unwrap();
    assert_eq!(
        s.store
            .call_task(MAIN, "inspect_task", &json!({"task_id":a.0}))
            .unwrap()["summary"],
        "Review evidence"
    );
}

#[test]
fn history_scanner_excludes_owned_and_retired_observer_sessions() {
    let s = Sandbox::new();
    let dir = s.root.join("client-sessions/archives");
    std::fs::create_dir_all(&dir).unwrap();
    s.store
        .register_session(MAIN, "internal-session", 0)
        .unwrap();
    for (id, cwd) in [
        ("internal-session", s.root.join("files")),
        ("old-observer", s.root.join("memory-observer/workspace")),
        ("external-session", s.root.join("outside")),
    ] {
        let archive = json!({"version":1,"session":{"client":"codex","native_id":id,"title":"Session","working_directory":cwd},"messages":[{"id":1,"role":"user","text":"A historical message"}]});
        std::fs::write(dir.join(format!("{id}.json")), archive.to_string()).unwrap();
    }
    crate::memory::MemoryConfig {
        claude_mem_url: Some("http://127.0.0.1:9".into()),
        import_history: true,
        ..Default::default()
    }
    .save(&s.root)
    .unwrap();
    let provider = crate::memory::open(&s.root).unwrap();
    let mut scanner = worker::ArchiveScanner::default();
    for _ in 0..5 {
        scanner
            .poll(&s.store, &TaskAgents::default(), &*provider)
            .unwrap();
    }
    assert_eq!(
        provider.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"],
        1
    );
}
