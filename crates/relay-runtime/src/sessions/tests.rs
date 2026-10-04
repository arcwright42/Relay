use super::*;
use std::io::Write;

fn fixture() -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "relay-client-session-{}",
        crate::installer::unique_id()
    ));
    let home = root.join("native-client");
    let source = home.join("sessions/2026/09/30/rollout.jsonl");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(root.join("workspace")).unwrap();
    (root, home, source)
}

fn record(role: &str, text: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({"timestamp":"2026-09-30T00:01:00Z","type":"response_item","payload":{"type":"message","role":role,"phase":if role == "assistant" {Some("final_answer")} else {None},"content":[{"type":"input_text","text":text}]}})
    )
}

fn seed(source: &Path, cwd: &Path) {
    let header = serde_json::json!({"timestamp":"2026-09-30T00:00:00Z","type":"session_meta","payload":{"id":"native-session-1","cwd":cwd}});
    fs::write(
        source,
        format!(
            "{header}\n{}{}{}",
            record("developer", "private system data"),
            record(
                "user",
                "<environment_context>injected</environment_context>"
            ),
            record("user", "original question")
        ),
    )
    .unwrap();
}

fn wait(
    store: &ClientSessionStore,
    predicate: impl Fn(&ClientSessionsSnapshot) -> bool,
) -> ClientSessionsSnapshot {
    let start = Instant::now();
    loop {
        let view = store.snapshot();
        if predicate(&view) {
            return view;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Local session worker stalled: {:?}",
            view.error
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn append_cursor_survives_restart_without_thread_assignment() {
    let (root, home, source) = fixture();
    seed(&source, &root.join("workspace"));
    let store = ClientSessionStore::with_home(root.join("relay"), home.clone());
    let first = wait(&store, |v| !v.syncing && v.last_sync_unix.is_some());
    assert!(first.error.is_none());
    assert_eq!(first.sessions.len(), 1);
    assert_eq!(first.sessions[0].message_count, 1);
    assert!(
        (CLIENT_SYNC_INTERVAL_SECS..=CLIENT_SYNC_INTERVAL_SECS + 2)
            .contains(&(first.next_sync_unix.unwrap() - first.last_sync_unix.unwrap()))
    );
    let id = first.sessions[0].id.clone();
    store.shutdown();
    // Old assignment metadata is ignored while the archive and append cursor survive.
    let index_path = root.join("relay/client-sessions/index.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    legacy["bindings"] = serde_json::json!({id.0.clone(): 2});
    legacy["sessions"][&id.0]["thread"] = serde_json::json!(2);
    fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let old = fs::read(&source).unwrap();
    let added = record("assistant", "an externally written reply");
    fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap()
        .write_all(added.as_bytes())
        .unwrap();
    let restarted = ClientSessionStore::with_home(root.join("relay"), home);
    wait(&restarted, |v| {
        !v.syncing && v.sessions.first().is_some_and(|s| s.message_count == 2)
    });
    restarted
        .dispatch(ClientSessionsCommand::Open(id.clone()))
        .unwrap();
    let opened = wait(&restarted, |v| v.detail.is_some());
    let detail = opened.detail.unwrap();
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[1].text, "an externally written reply");
    let stable_id = detail.messages[0].id;
    restarted.dispatch(ClientSessionsCommand::Sync).unwrap();
    let repeated = wait(&restarted, |v| !v.syncing);
    assert_eq!(repeated.updated_sessions, 0);
    assert_eq!(repeated.detail.unwrap().messages[0].id, stable_id);
    // An unchanged native file can rebuild an accidentally deleted Relay archive.
    fs::remove_file(archive_path(&root.join("relay/client-sessions"), &id)).unwrap();
    restarted.dispatch(ClientSessionsCommand::Sync).unwrap();
    let rebuilt = wait(&restarted, |v| !v.syncing);
    assert!(rebuilt.error.is_none());
    assert_eq!(rebuilt.updated_sessions, 1);
    assert_eq!(rebuilt.detail.unwrap().messages.len(), 2);
    assert_eq!(
        fs::read(&source).unwrap(),
        [old, added.into_bytes()].concat()
    );
    restarted.shutdown();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_compaction_record_keeps_visible_messages_without_archiving_context() {
    let (root, _, source) = fixture();
    seed(&source, &root.join("workspace"));
    let compacted = serde_json::json!({
        "type": "compacted",
        "payload": {"replacement_history": "x".repeat(17 * 1024 * 1024)}
    });
    let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
    writeln!(file, "{compacted}").unwrap();
    file.write_all(record("assistant", "visible reply after compaction").as_bytes())
        .unwrap();
    let metadata = codex::metadata(&source, codex::FileStamp::read(&source).unwrap()).unwrap();
    let (archive, _) = codex::read(&source, metadata, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(archive.messages.len(), 2);
    assert_eq!(archive.messages[1].text, "visible reply after compaction");
    assert_eq!(archive.offset, fs::metadata(&source).unwrap().len());
    assert!(serde_json::to_vec(&archive).unwrap().len() < 4096);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_lines_rewrites_and_hidden_records_are_handled_without_duplicates() {
    let (root, _, source) = fixture();
    seed(&source, &root.join("workspace"));
    let stopped = AtomicBool::new(false);
    let metadata = codex::metadata(&source, codex::FileStamp::read(&source).unwrap()).unwrap();
    let (first, _) = codex::read(&source, metadata, None, &stopped).unwrap();
    assert_eq!(first.messages.len(), 1);
    let original_offset = first.offset;
    let added = record("assistant", "complete reply");
    let split = added.len() / 2;
    fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap()
        .write_all(&added.as_bytes()[..split])
        .unwrap();
    let metadata = codex::metadata(&source, codex::FileStamp::read(&source).unwrap()).unwrap();
    let (partial, _) = codex::read(&source, metadata, Some(first), &stopped).unwrap();
    assert_eq!(partial.offset, original_offset);
    assert_eq!(partial.messages.len(), 1);
    fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap()
        .write_all(&added.as_bytes()[split..])
        .unwrap();
    let metadata = codex::metadata(&source, codex::FileStamp::read(&source).unwrap()).unwrap();
    let (complete, _) = codex::read(&source, metadata, Some(partial), &stopped).unwrap();
    assert_eq!(complete.messages.len(), 2);
    let original = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        original.replace("original question", "rewritten content"),
    )
    .unwrap();
    let metadata = codex::metadata(&source, codex::FileStamp::read(&source).unwrap()).unwrap();
    let (rewritten, _) = codex::read(&source, metadata, Some(complete), &stopped).unwrap();
    assert_eq!(rewritten.messages.len(), 2);
    assert_eq!(rewritten.messages[0].text, "rewritten content");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_complete_line_preserves_archive_and_unknown_index_is_not_overwritten() {
    let (root, home, source) = fixture();
    seed(&source, &root.join("workspace"));
    let store = ClientSessionStore::with_home(root.join("relay"), home.clone());
    let first = wait(&store, |v| !v.syncing && !v.sessions.is_empty());
    let path = archive_path(&root.join("relay/client-sessions"), &first.sessions[0].id);
    let previous = fs::read(&path).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap()
        .write_all(b"{broken}\n")
        .unwrap();
    store.dispatch(ClientSessionsCommand::Sync).unwrap();
    let failed = wait(&store, |v| !v.syncing);
    assert_eq!(failed.failed_files, 1);
    assert_eq!(failed.sessions[0].message_count, 1);
    assert_eq!(fs::read(&path).unwrap(), previous);
    store.shutdown();
    let index = root.join("relay/client-sessions/index.json");
    let unknown = r#"{"version":99,"sessions":{},"last_sync_unix":null}"#;
    fs::write(&index, unknown).unwrap();
    let blocked = ClientSessionStore::with_home(root.join("relay"), home);
    blocked.dispatch(ClientSessionsCommand::Sync).unwrap();
    wait(&blocked, |v| !v.syncing && v.error.is_some());
    blocked.shutdown();
    assert_eq!(fs::read_to_string(&index).unwrap(), unknown);
    fs::remove_dir_all(root).unwrap();
}
