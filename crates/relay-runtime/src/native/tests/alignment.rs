use super::*;
use crate::store::{SavedMessage, SavedTool};

fn summary(learned: &str) -> Value {
    json!({"request":"remember the implementation decision","investigated":"read source and ran checks","learned":learned,"completed":"unit checks","next_steps":"real model verification remains","notes":""})
}
fn finish(store: &NativeStore, job: &Value, notes: &[Value], fields: Option<&Value>) {
    store
        .commit_extraction(
            job["job_id"].as_u64().unwrap(),
            job["attempt"].as_u64().unwrap(),
            notes,
            fields,
        )
        .unwrap();
}

#[test]
fn tool_events_survive_before_turn_end_and_retire_shortened_output() {
    let s = Sandbox::new();
    let mut tool:SavedTool=serde_json::from_value(json!({"id":"tool-1","title":"read file","kind":"read","status":"running","input":"file.rs","output":"x".repeat(52000)})).unwrap();
    s.store
        .capture_tool(MAIN, 2, "inspect implementation", &tool)
        .unwrap();
    assert!(
        s.store.claim_job().unwrap().is_none(),
        "An unfinished tool result is not a completed observation"
    );
    tool.status = "completed".into();
    tool.output = format!("{}TAIL-IMPORTANT-DECISION", "x".repeat(30000));
    s.store
        .capture_tool(MAIN, 2, "inspect implementation", &tool)
        .unwrap();
    let reopened = NativeStore::open(s.root.clone()).unwrap();
    let db = reopened.db.lock().unwrap();
    let (active, retired): (u64, u64) = db
        .query_row(
            "SELECT sum(retired=0),sum(retired=1) FROM sources",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((active, retired), (2, 1));
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM sources WHERE instr(body,'TAIL-IMPORTANT-DECISION')>0 AND retired=0)",[],|r|r.get::<_,bool>(0)).unwrap());
    drop(db);
    assert!(reopened.claim_job().unwrap().is_some());
    let before: u64 = s
        .store
        .db
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM source_versions", [], |r| r.get(0))
        .unwrap();
    s.store
        .capture_tool(MAIN, 2, "inspect implementation", &tool)
        .unwrap();
    assert_eq!(
        before,
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM source_versions", [], |r| r
                .get::<_, u64>(0))
            .unwrap()
    );
}

#[test]
fn summaries_follow_observations_and_recover_a_new_generation() {
    let s = Sandbox::new();
    let source = s
        .store
        .ingest(
            "first",
            Some(MAIN),
            "relay",
            "Decision",
            "Use SQLite for offline operation",
        )
        .unwrap();
    s.store.queue_summary(source).unwrap();
    let observation = s.store.claim_job().unwrap().unwrap();
    assert_eq!(observation["kind"], "observation");
    assert!(
        s.store.claim_job().unwrap().is_none(),
        "The checkpoint waits for its observations"
    );
    let mut n = note(source, "Use SQLite because it works offline");
    n["facts"] = json!(["offline storage"]);
    n["files_read"] = json!(["storage.rs"]);
    finish(&s.store, &observation, &[n], None);
    let checkpoint = s.store.claim_job().unwrap().unwrap();
    assert_eq!(checkpoint["kind"], "summary");
    assert_eq!(
        checkpoint["context"]["observations_since_checkpoint"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    finish(
        &s.store,
        &checkpoint,
        &[],
        Some(&summary("SQLite is the offline store")),
    );
    let reopened = NativeStore::open(s.root.clone()).unwrap();
    reopened
        .ingest(
            "second",
            Some(MAIN),
            "relay",
            "Follow-up",
            "Keep that decision; add a migration",
        )
        .unwrap();
    let second = reopened.claim_job().unwrap().unwrap();
    assert_eq!(
        second["context"]["previous_summary"]["summary"]["learned"],
        "SQLite is the offline store"
    );
    let recovery = reopened.context(MAIN, "hi").unwrap();
    assert!(
        recovery.contains("SQLite is the offline store"),
        "Recovery is independent of the next query's keywords"
    );
    let memory = reopened.get_memory(MAIN, &[1]).unwrap();
    assert_eq!(memory[0]["attributes"]["files_read"], json!(["storage.rs"]));
    assert!(
        !reopened
            .context(ThreadId(98), "hi")
            .unwrap()
            .contains("SQLite")
    );
}

#[test]
fn summary_context_is_fenced_and_forgetting_removes_derived_vectors() {
    let s = Sandbox::new();
    let a = s
        .store
        .ingest("a", Some(MAIN), "relay", "a", "original choice")
        .unwrap();
    let j = s.store.claim_job().unwrap().unwrap();
    finish(&s.store, &j, &[note(a, "original choice")], None);
    let b = s
        .store
        .ingest("b", Some(MAIN), "relay", "b", "continue the choice")
        .unwrap();
    s.store.queue_summary(b).unwrap();
    let j = s.store.claim_job().unwrap().unwrap();
    finish(&s.store, &j, &[note(b, "follow-up choice")], None);
    let checkpoint = s.store.claim_job().unwrap().unwrap();
    finish(
        &s.store,
        &checkpoint,
        &[],
        Some(&summary("original choice and follow-up")),
    );
    s.store
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO memory_embeddings SELECT id,0,'profile',1,x'0000803f' FROM memories",
            [],
        )
        .unwrap();
    let last = s
        .store
        .ingest("c", Some(MAIN), "relay", "c", "same plan")
        .unwrap();
    let inflight = s.store.claim_job().unwrap().unwrap();
    assert_eq!(inflight["source_id"], last);
    s.store.forget(MAIN, None, Some(1)).unwrap();
    assert!(
        s.store
            .commit_job(
                inflight["job_id"].as_u64().unwrap(),
                1,
                &[note(last, "same plan")]
            )
            .is_err()
    );
    let db = s.store.db.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM memory_embeddings", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM memory_fts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT fields FROM session_summaries", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "{}"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM memories WHERE body!=''", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn malformed_ack_and_manual_retry_do_not_reuse_observer_leases() {
    use relay_core::memory::*;
    let s = Sandbox::new();
    s.store
        .ingest("a", Some(MAIN), "relay", "a", "evidence")
        .unwrap();
    let j = s.store.claim_job().unwrap().unwrap();
    assert!(
        s.store
            .call(
                OBSERVER,
                "memory_commit_job",
                &json!({"job_id":j["job_id"],"attempt":1})
            )
            .is_err()
    );
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE jobs SET state='failed',retry_attempts=3", [])
        .unwrap();
    MemoryService::apply(&s.store, MemoryCommand::RetryFailed).unwrap();
    let retry = s.store.claim_job().unwrap().unwrap();
    assert_eq!(retry["attempt"], 2);
    assert!(
        s.store
            .commit_job(j["job_id"].as_u64().unwrap(), 1, &[])
            .is_err()
    );
    finish(&s.store, &retry, &[], None);
}

#[test]
fn long_turns_checkpoint_during_tools_and_turn_capture_is_idempotent() {
    let s = Sandbox::new();
    let tool:SavedTool=serde_json::from_value(json!({"id":"large","title":"read","kind":"read","status":"completed","input":"","output":"x".repeat(72000)})).unwrap();
    s.store.capture_tool(MAIN, 2, "request", &tool).unwrap();
    let first = s.store.claim_job().unwrap().unwrap();
    finish(&s.store, &first, &[], None);
    let second = s.store.claim_job().unwrap().unwrap();
    finish(&s.store, &second, &[], None);
    let checkpoint = s.store.claim_job().unwrap().unwrap();
    assert_eq!(
        checkpoint["kind"], "summary",
        "A long tool must checkpoint before its remaining fragments"
    );
    finish(
        &s.store,
        &checkpoint,
        &[],
        Some(&summary("partial read; task not complete")),
    );
    let messages: Vec<SavedMessage> = serde_json::from_value(json!([
        {"id":1,"role":"user","text":"request","complete":true,"tools":[]},
        {"id":2,"role":"assistant","text":"read finished","complete":true,"tools":[tool]}
    ]))
    .unwrap();
    s.store.capture_turn(MAIN, &messages).unwrap();
    let before: u64 = s
        .store
        .db
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    s.store.capture_turn(MAIN, &messages).unwrap();
    assert_eq!(
        before,
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, u64>(0))
            .unwrap()
    );
}

#[test]
fn recall_rechecks_lexical_candidates_after_network_io() {
    let s = Sandbox::new();
    let source = s
        .store
        .ingest("racing", Some(MAIN), "relay", "SQLite", "SQLite decision")
        .unwrap();
    let id = s
        .store
        .write_memory(MAIN, &note(source, "SQLite decision"))
        .unwrap();
    let report = s
        .store
        .search_report_with(
            MAIN,
            "SQLite",
            false,
            &retrieval::SearchFilter::default(),
            || {
                s.store.forget(MAIN, None, Some(id))?;
                bail!("Embedding request timed out")
            },
        )
        .unwrap();
    assert!(report["results"].as_array().unwrap().is_empty());
    assert_eq!(report["retrieval"]["mode"], "lexical");
    assert_eq!(
        report["retrieval"]["warning"],
        "Embedding request timed out"
    );
}

#[test]
fn reviewing_compressed_memory_preserves_metadata_and_pages_provenance() {
    use relay_core::memory::{MemoryCommand, MemoryService};
    let s = Sandbox::new();
    let mut refs = vec![];
    for i in 0..40 {
        let source = s
            .store
            .ingest(
                &format!("evidence:{i}"),
                Some(MAIN),
                "relay",
                "source",
                "source body",
            )
            .unwrap();
        refs.push(json!({"source_id":source,"revision":1}));
    }
    let mut n = note(1, "A compressed session observation");
    n["sources"] = json!(refs);
    n["facts"] = json!(["Check evidence before claiming completion"]);
    let old = s.store.write_memory(MAIN, &n).unwrap();
    let id = MemoryService::apply(&s.store, MemoryCommand::Confirm(old))
        .unwrap()
        .unwrap();
    let first = s
        .store
        .call(MAIN, "memory_get", &json!({"ids":[id]}))
        .unwrap();
    assert_eq!(first[0]["evidence"].as_array().unwrap().len(), 32);
    assert_eq!(first[0]["attributes"]["facts"], n["facts"]);
    let second = s
        .store
        .call(
            MAIN,
            "memory_get",
            &json!({"ids":[id],"evidence_offset":first[0]["next_evidence_offset"]}),
        )
        .unwrap();
    assert_eq!(second[0]["evidence"].as_array().unwrap().len(), 8);
    assert_eq!(second[0]["next_evidence_offset"], Value::Null);
}

#[test]
fn message_identity_outlives_chat_buffers_and_forgotten_sources() {
    let s = Sandbox::new();
    // This represents v1 data written before the dedicated counter existed.
    let source = s
        .store
        .ingest(
            "relay:0:turn:24:part:0",
            Some(MAIN),
            "relay",
            "old turn",
            "old decision",
        )
        .unwrap();
    s.store.forget(MAIN, Some(source), None).unwrap();
    assert_eq!(s.store.reserve_message_ids(MAIN, 1).unwrap(), 25);
    let reopened = NativeStore::open(s.root.clone()).unwrap();
    assert_eq!(reopened.reserve_message_ids(MAIN, 1).unwrap(), 27);
    assert_eq!(reopened.reserve_message_ids(MAIN, 100).unwrap(), 100);
}

#[test]
fn observer_recovers_an_orphaned_lease_without_another_pending_job() {
    let s = Sandbox::new();
    s.store
        .ingest("orphan", Some(MAIN), "relay", "source", "event")
        .unwrap();
    s.store.claim_job().unwrap().unwrap();
    s.store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE jobs SET lease_until=unixepoch()-1", [])
        .unwrap();
    assert_eq!(
        s.store.next_observer_session().unwrap(),
        Some("thread:0".into())
    );
    assert_eq!(s.store.claim_job().unwrap().unwrap()["attempt"], 2);
}
