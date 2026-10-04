use super::*;
struct Sandbox {
    root: PathBuf,
    store: NativeStore,
}
impl Sandbox {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("relay-native-test-{}", uuid::Uuid::new_v4()));
        let store = NativeStore::open(root.clone()).unwrap();
        Self { root, store }
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn note(source: u64, body: &str) -> Value {
    json!({"title":"Relay 记忆方案","body":body,"kind":"decision","scope":null,"status":"confirmed","sources":[{"source_id":source,"revision":1}],"topics":["Relay","记忆"]})
}

#[test]
fn main_room_and_task_identity_survive_reopen_without_prompt_classification() {
    let s = Sandbox::new();
    let id = s
        .store
        .create_task("task-a", "报告", "研究 native memory")
        .unwrap();
    assert_ne!(id, MAIN.0);
    assert_eq!(
        s.store
            .create_task("task-a", "报告", "研究 native memory")
            .unwrap(),
        id
    );
    assert!(s.store.create_task("task-a", "报告", "different").is_err());
    let reopened = NativeStore::open(s.root.clone()).unwrap();
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
fn extraction_is_atomic_idempotent_and_stale_results_cannot_commit() {
    let s = Sandbox::new();
    let source = s
        .store
        .ingest("event-a", Some(MAIN), "relay", "决定", "采用原生存储")
        .unwrap();
    s.store
        .ingest("event-a", Some(MAIN), "relay", "决定", "采用原生存储")
        .unwrap();
    let job = s.store.claim_job().unwrap().unwrap();
    let id = job["job_id"].as_u64().unwrap();
    s.store
        .commit_job(
            id,
            job["attempt"].as_u64().unwrap(),
            &[note(source, "原生存储")],
        )
        .unwrap();
    s.store
        .commit_job(
            id,
            job["attempt"].as_u64().unwrap(),
            &[note(source, "原生存储")],
        )
        .unwrap();
    let found = s.store.search(MAIN, "原生存储", false).unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(found[0]["status_or_origin"], "candidate");
    s.store
        .ingest("event-a", Some(MAIN), "relay", "修订", "暂缓实现")
        .unwrap();
    assert!(
        s.store
            .search(MAIN, "原生存储", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let job = s.store.claim_job().unwrap().unwrap();
    let id = job["job_id"].as_u64().unwrap();
    s.store
        .ingest("event-a", Some(MAIN), "relay", "再次修订", "重新评估")
        .unwrap();
    assert!(
        s.store
            .commit_job(
                id,
                job["attempt"].as_u64().unwrap(),
                &[note(source, "暂缓实现")]
            )
            .is_err()
    );
}

#[test]
fn chinese_short_queries_provenance_supersession_and_forget_work_together() {
    let s = Sandbox::new();
    let source = s
        .store
        .ingest("event", Some(MAIN), "relay", "偏好", "用户选择中文")
        .unwrap();
    let id = s
        .store
        .write_memory(MAIN, &note(source, "用中文回答"))
        .unwrap();
    assert_eq!(s.store.search(MAIN, "中文", false).unwrap()[0]["id"], id);
    assert_eq!(
        s.store.get_memory(MAIN, &[id]).unwrap()[0]["evidence"][0]["source_id"],
        source
    );
    let mut n = note(source, "中英文均可");
    n["supersedes"] = json!(id);
    let next = s.store.write_memory(MAIN, &n).unwrap();
    assert!(s.store.get_memory(MAIN, &[id]).is_err());
    assert!(s.store.get_memory(MAIN, &[next]).is_ok());
    s.store.forget(MAIN, Some(source), None).unwrap();
    s.store
        .ingest("event", Some(MAIN), "relay", "偏好", "用户再次选择中文")
        .unwrap();
    assert!(
        s.store
            .search(MAIN, "中文", true)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(s.store.get_memory(MAIN, &[next]).is_err());
    assert!(s.store.claim_job().unwrap().is_none());
}

#[test]
fn tools_enforce_task_scope_and_observer_cannot_confirm_or_delegate() {
    let s = Sandbox::new();
    let id = s.store.create_task("t", "task", "do work").unwrap();
    let source = s
        .store
        .ingest(
            "private",
            Some(MAIN),
            "relay",
            "private",
            "main-only source",
        )
        .unwrap();
    assert!(
        s.store
            .write_memory(ThreadId(id), &note(source, "global"))
            .is_err()
    );
    assert!(
        s.store
            .call(ThreadId(id), "create_task", &json!({}))
            .is_err()
    );
    assert!(s.store.call(OBSERVER, "memory_write", &json!({})).is_err());
    assert!(s.store.timeline(ThreadId(id), source).is_err());
    let global = s
        .store
        .write_memory(MAIN, &note(source, "shared preference"))
        .unwrap();
    assert!(s.store.get_memory(ThreadId(id), &[global]).is_ok());
}

#[test]
fn legacy_import_preserves_files_and_never_reuses_old_project_as_main_room() {
    let s = Sandbox::new();
    let bytes=serde_json::to_vec(&json!({"version":3,"projects":[{"id":7,"name":"旧项目","description":"历史","instructions":"仅这个任务","context":[],"memory":[{"kind":"decision","content":"旧决定"}]}]})).unwrap();
    std::fs::write(s.root.join("projects.json"), &bytes).unwrap();
    s.store.migrate().unwrap();
    s.store.migrate().unwrap();
    assert_eq!(std::fs::read(s.root.join("projects.json")).unwrap(), bytes);
    assert_eq!(s.store.snapshot().threads.len(), 2);
    assert_eq!(s.store.thread(MAIN).unwrap().instructions, "");
    assert!(
        s.store
            .search(MAIN, "旧决定", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        s.store
            .search(MAIN, "旧决定", true)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn expired_attempts_and_invalid_batches_never_publish_partial_memory() {
    let s = Sandbox::new();
    let source = s
        .store
        .ingest(
            "event",
            Some(MAIN),
            "relay",
            "Decision",
            "native persistence",
        )
        .unwrap();
    let first = s.store.claim_job().unwrap().unwrap();
    let id = first["job_id"].as_u64().unwrap();
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE jobs SET lease_until=0 WHERE id=?", [id])
        .unwrap();
    let second = s.store.claim_job().unwrap().unwrap();
    assert_eq!(second["attempt"], 2);
    assert!(
        s.store
            .commit_job(id, 1, &[note(source, "old attempt")])
            .is_err()
    );
    assert!(
        s.store
            .commit_job(id, 2, &[note(source, "valid"), json!({"title":"invalid"})])
            .is_err()
    );
    assert!(
        s.store
            .search(MAIN, "", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let output =
        json!({"job_id":id,"attempt":2,"notes":[note(source,"current attempt")]}).to_string();
    worker::commit_observer_response(&s.store, id, 2, &output).unwrap();
    assert_eq!(
        s.store
            .search(MAIN, "current", false)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(worker::commit_observer_response(&s.store, id, 1, &output).is_err());
}

#[test]
fn source_revisions_retire_shortened_archives_and_preserve_full_tail_text() {
    let s = Sandbox::new();
    let text = format!("{}NEEDLE_AT_END", "长".repeat(50000));
    s.store
        .ingest_archive(
            "codex",
            "session-a",
            "Long session",
            &[(1, "user".into(), text)],
        )
        .unwrap();
    let found = s.store.search(MAIN, "NEEDLE_AT_END", true).unwrap();
    let id = found[0]["id"].as_u64().unwrap();
    let memory = s.store.write_memory(MAIN, &note(id, "old tail")).unwrap();
    assert!(
        s.store.read_source(MAIN, id, 0).unwrap()["body"]
            .as_str()
            .unwrap()
            .contains("NEEDLE_AT_END")
    );
    s.store
        .ingest_archive(
            "codex",
            "session-a",
            "Short session",
            &[(1, "user".into(), "现在只保留短文".into())],
        )
        .unwrap();
    assert!(
        s.store
            .search(MAIN, "NEEDLE_AT_END", true)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(s.store.get_memory(MAIN, &[memory]).is_err());
    let source = s.store.search(MAIN, "短文", true).unwrap()[0]["id"]
        .as_u64()
        .unwrap();
    assert!(
        s.store
            .write_memory(MAIN, &note(source, "wrong revision"))
            .is_err()
    );
    let mut corrected = note(source, "current revision");
    corrected["sources"][0]["revision"] = json!(2);
    s.store.write_memory(MAIN, &corrected).unwrap();
    let revisions: u64 = s
        .store
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM source_versions WHERE source_id=?",
            [source],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revisions, 2);
    assert!(s.store.search(MAIN, "\" OR *", true).is_ok());
}

#[test]
fn topics_collect_multiple_sessions_without_changing_their_identity() {
    let s = Sandbox::new();
    for session in ["a", "b"] {
        s.store
            .ingest_archive(
                "codex",
                session,
                "Session",
                &[(1, "user".into(), format!("{session}: choose SQLite"))],
            )
            .unwrap();
        let job = s.store.claim_job().unwrap().unwrap();
        s.store
            .commit_job(
                job["job_id"].as_u64().unwrap(),
                job["attempt"].as_u64().unwrap(),
                &[note(
                    job["source_id"].as_u64().unwrap(),
                    "SQLite persistence",
                )],
            )
            .unwrap();
    }
    let topics = s
        .store
        .call(MAIN, "memory_topics", &json!({"topic":"relay"}))
        .unwrap();
    assert_eq!(topics["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(s.store.snapshot().threads.len(), 1);
    s.store
        .call(
            MAIN,
            "memory_merge_topics",
            &json!({"from":"relay","into":"记忆"}),
        )
        .unwrap();
    assert_eq!(
        s.store
            .topics(MAIN, None)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn extraction_pause_and_budget_do_not_disable_retrieval() {
    let s = Sandbox::new();
    for key in ["first", "second"] {
        s.store
            .ingest(key, Some(MAIN), "relay", "Event", key)
            .unwrap();
    }
    s.store
        .call(
            MAIN,
            "memory_settings",
            &json!({"enabled":false,"runs_per_hour":1}),
        )
        .unwrap();
    assert!(s.store.claim_job().unwrap().is_none());
    assert_eq!(
        s.store
            .search(MAIN, "", true)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    s.store
        .call(
            MAIN,
            "memory_settings",
            &json!({"enabled":true,"runs_per_hour":1}),
        )
        .unwrap();
    assert!(s.store.claim_job().unwrap().is_some());
    assert!(s.store.claim_job().unwrap().is_none());
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE meta SET value=0 WHERE key='observer_window'", [])
        .unwrap();
    assert!(s.store.claim_job().unwrap().is_some());
}

#[test]
fn headless_mcp_negotiates_lists_tools_and_enforces_scope_on_calls() {
    let s = Sandbox::new();
    let id = s.store.create_task("task", "Title", "Work").unwrap();
    let input=[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_task","arguments":{"request_key":"forbidden","title":"Title","instructions":"Work"}}}),
    ].iter().map(|v|format!("{v}\n")).collect::<String>();
    let mut output = Vec::new();
    mcp::serve(
        &s.store,
        ThreadId(id),
        std::io::Cursor::new(input),
        &mut output,
    )
    .unwrap();
    let lines = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "relay-native");
    assert!(
        lines[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "create_task")
    );
    assert_eq!(lines[2]["result"]["isError"], true);
    assert_eq!(s.store.snapshot().threads.len(), 2);
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
fn durable_transcripts_repair_a_missed_capture_once_and_keep_tool_evidence() {
    use relay_core::agents::*;
    let s = Sandbox::new();
    let messages = [
        ChatMessage {
            id: 1,
            role: MessageRole::User,
            text: "Verify the result".into(),
            status: MessageStatus::Complete,
            tools: vec![],
            metrics: None,
        },
        ChatMessage {
            id: 2,
            role: MessageRole::Assistant,
            text: "Verified".into(),
            status: MessageStatus::Complete,
            tools: vec![ToolActivity {
                id: "test".into(),
                title: "Tests".into(),
                status: "completed".into(),
                input: "cargo test".into(),
                output: "ALL_CHECKS_PASSED".into(),
            }],
            metrics: None,
        },
    ];
    crate::store::save(
        &s.root,
        MAIN,
        &crate::store::SavedThread {
            version: 2,
            messages: messages
                .iter()
                .map(crate::store::SavedMessage::from_message)
                .collect(),
            ..Default::default()
        },
    )
    .unwrap();
    s.store.replay_transcript(MAIN).unwrap();
    s.store.replay_transcript(MAIN).unwrap();
    assert_eq!(
        s.store
            .search(MAIN, "ALL_CHECKS_PASSED", true)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let count: u64 = s
        .store
        .db
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
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
fn archive_scan_never_reextracts_the_observers_own_conversations() {
    let s = Sandbox::new();
    let dir = s.root.join("client-sessions/archives");
    std::fs::create_dir_all(&dir).unwrap();
    s.store
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO owned_sessions VALUES('internal-session',?)",
            [OBSERVER.0],
        )
        .unwrap();
    for (key, cwd) in [
        ("internal-session", s.root.join("files")),
        ("observer-by-path", s.root.join("memory-observer/workspace")),
        ("external-session", s.root.join("outside")),
    ] {
        let archive = json!({"version":1,"session":{"client":"codex","native_id":key,"title":"Session","working_directory":cwd},"messages":[{"id":1,"role":"user","text":"remember this source"}]});
        std::fs::write(dir.join(format!("{key}.json")), archive.to_string()).unwrap();
    }
    let mut scanner = worker::ArchiveScanner::default();
    for _ in 0..5 {
        scanner.poll(&s.store, &TaskAgents::default()).unwrap();
    }
    let found = s.store.search(MAIN, "remember", true).unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(
        found[0]["status_or_origin"],
        "client:codex:external-session"
    );
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
        .call(MAIN, "cancel_task", &json!({"task_id":task.0}))
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
        .call(MAIN, "cancel_task", &json!({"task_id":task.0}))
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

#[test]
fn observer_final_json_survives_acp_commentary_coalescing() {
    let s = Sandbox::new();
    let source = s
        .store
        .ingest(
            "event",
            Some(MAIN),
            "relay",
            "Preference",
            "Prefer concise Chinese",
        )
        .unwrap();
    let job = s.store.claim_job().unwrap().unwrap();
    let id = job["job_id"].as_u64().unwrap();
    let payload = json!({"job_id":id,"attempt":1,"notes":[note(source,"用简洁中文回答")]});
    let text = format!("I will extract observations.\n```json\n{payload}\n```");
    worker::commit_observer_response(&s.store, id, 1, &text).unwrap();
    assert_eq!(
        s.store
            .search(MAIN, "简洁中文", false)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
