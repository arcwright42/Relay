use super::*;
use crate::resident::{MAIN, RETIRED_OBSERVER, ResidentStore};
use claude_mem::ClaudeMemProvider;
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("relay-memory-provider-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn config(&self, url: &str) -> MemoryConfig {
        MemoryConfig {
            provider: ProviderKind::ClaudeMem,
            claude_mem_url: Some(url.into()),
            namespace: Some("relay-contract-test".into()),
            ..Default::default()
        }
    }
    fn provider(&self, url: &str) -> ClaudeMemProvider {
        ClaudeMemProvider::open(&self.0, self.config(url)).unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Debug)]
struct Request {
    method: String,
    target: String,
    body: Value,
}
struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Mock {
    fn new(handler: impl Fn(&Request) -> (u16, Value) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut input = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                input.read_line(&mut line).unwrap();
                let parts = line.split_whitespace().collect::<Vec<_>>();
                let (method, target) = (parts[0].to_owned(), parts[1].to_owned());
                let mut length = 0;
                let mut chunked = false;
                loop {
                    line.clear();
                    input.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let header = line.to_ascii_lowercase();
                    if let Some(v) = header.strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap()
                    }
                    if header.starts_with("transfer-encoding:") && header.contains("chunked") {
                        chunked = true
                    }
                }
                let mut body = vec![];
                if chunked {
                    loop {
                        line.clear();
                        input.read_line(&mut line).unwrap();
                        let n = usize::from_str_radix(line.trim(), 16).unwrap();
                        if n == 0 {
                            break;
                        }
                        let mut chunk = vec![0; n];
                        input.read_exact(&mut chunk).unwrap();
                        body.extend(chunk);
                        line.clear();
                        input.read_line(&mut line).unwrap();
                    }
                } else {
                    body.resize(length, 0);
                    input.read_exact(&mut body).unwrap();
                }
                let request = Request {
                    method,
                    target,
                    body: if body.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&body).unwrap()
                    },
                };
                let (status, response) = handler(&request);
                received.lock().unwrap().push(request);
                let response = response.to_string();
                write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            handle: Some(handle),
        }
    }
    fn posts(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .cloned()
            .collect()
    }
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.handle.take().unwrap().join().unwrap();
    }
}

fn acknowledge(r: &Request) -> (u16, Value) {
    (
        200,
        if r.target == "/api/health" {
            json!({"status":"ok","version":"13.31.0","initialized":true})
        } else if r.target == "/api/sessions/init" {
            json!({"status":"initialized","sessionDbId":1})
        } else if r.target == "/api/sessions/session-end" {
            json!({"status":"accepted"})
        } else {
            json!({"status":"queued"})
        },
    )
}

fn turn() -> MemoryEvent {
    MemoryEvent::Turn {
        directory: std::env::temp_dir(),
        thread: MAIN,
        messages: vec![
            MemoryMessage {
                id: 1,
                role: "user".into(),
                text: "Use WAL for the durable queue".into(),
                complete: true,
                tools: vec![],
            },
            MemoryMessage {
                id: 2,
                role: "assistant".into(),
                text: "Implemented WAL; crash recovery is still unverified.".into(),
                complete: true,
                tools: vec![MemoryToolEvent {
                    id: "tool-1".into(),
                    title: "Edit".into(),
                    status: "completed".into(),
                    input: "queue.rs".into(),
                    output: "WAL enabled".into(),
                }],
            },
        ],
    }
}

#[test]
fn lifecycle_is_durable_ordered_deduplicated_and_survives_context_policy_changes() {
    let sandbox = Sandbox::new();
    let server = Mock::new(acknowledge);
    let provider = sandbox.provider(&server.url);
    provider
        .record(&MemoryEvent::User {
            directory: std::env::temp_dir(),
            thread: MAIN,
            message: 1,
            text: "Use WAL for the durable queue".into(),
        })
        .unwrap();
    provider.record(&turn()).unwrap();
    provider.record(&turn()).unwrap();
    provider
        .record(&MemoryEvent::SessionEnd {
            thread: MAIN,
            last_message: 2,
        })
        .unwrap();
    assert!(
        server.posts().is_empty(),
        "Capture must journal locally without blocking on HTTP"
    );
    assert_eq!(
        provider.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"],
        5
    );
    provider.drain().unwrap();
    provider.drain().unwrap();
    let posts = server.posts();
    assert_eq!(
        posts.iter().map(|p| p.target.as_str()).collect::<Vec<_>>(),
        [
            "/api/sessions/init",
            "/api/sessions/observations",
            "/api/sessions/observations",
            "/api/sessions/summarize",
            "/api/sessions/session-end"
        ]
    );
    assert_eq!(posts[0].body["platformSource"], "relay-global");
    assert_eq!(
        posts[1].body["tool_use_id"],
        "relay-contract-test-thread-0:tool:2:tool-1"
    );
    assert_eq!(posts[2].body["tool_name"], "RelayAssistantResponse");
    assert_eq!(
        posts[3].body["last_assistant_message"],
        "Implemented WAL; crash recovery is still unverified."
    );
    drop(provider);
    let mut config = sandbox.config(&server.url);
    config.context = ContextMode::Disabled;
    config.import_history = true;
    let reopened = ClaudeMemProvider::open(&sandbox.0, config).unwrap();
    reopened.record(&turn()).unwrap();
    reopened.drain().unwrap();
    assert_eq!(server.posts().len(), 5);
    assert_eq!(
        reopened.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["accepted"],
        5
    );
}

#[test]
fn offline_capture_is_retained_and_ambiguous_posts_block_the_session_until_resolved() {
    let online = Arc::new(AtomicBool::new(false));
    let live = online.clone();
    let ambiguous = Arc::new(AtomicBool::new(true));
    let failed = ambiguous.clone();
    let server = Mock::new(move |r| {
        if r.target == "/api/health" && !live.load(Ordering::Acquire) {
            return (503, json!({"error":"offline"}));
        }
        if r.method == "POST" && failed.load(Ordering::Acquire) {
            return (500, json!({"error":"possibly stored"}));
        }
        acknowledge(r)
    });
    let sandbox = Sandbox::new();
    let provider = sandbox.provider(&server.url);
    provider.record(&turn()).unwrap();
    assert_eq!(provider.drain().unwrap()["offline"], true);
    assert!(server.posts().is_empty());
    online.store(true, Ordering::Release);
    provider.drain().unwrap();
    let status = provider.call(MAIN, "memory_status", &json!({})).unwrap();
    assert_eq!(status["delivery"]["counts"]["uncertain"], 1);
    assert_eq!(status["delivery"]["counts"]["pending"], 3);
    provider.apply(MemoryCommand::RetryFailed).unwrap();
    provider.drain().unwrap();
    assert_eq!(server.posts().len(), 1);
    let id = status["delivery"]["issues"][0]["id"].as_u64().unwrap();
    ambiguous.store(false, Ordering::Release);
    provider.resolve(id, "accepted").unwrap();
    provider.drain().unwrap();
    assert_eq!(server.posts().len(), 4);
    assert!(provider.resolve(id, "retry").is_err());
}

#[test]
fn tasks_cannot_read_delete_or_inject_other_scopes() {
    let sandbox = Sandbox::new();
    let server = Mock::new(|r| {
        (
            200,
            if r.target.starts_with("/api/observation/") {
                let id = r.target.rsplit('/').next().unwrap().parse::<u64>().unwrap();
                json!({"id":id,"project":if id==3 {"another-project"}else{"relay-contract-test"},"platform_source":if id==1 {"relay-global"} else {"relay-task-22"},"title":"Scoped memory","narrative":"Original"})
            } else if r.target.starts_with("/api/search") {
                json!({"observations":[],"totalResults":0})
            } else {
                json!({})
            },
        )
    });
    let provider = sandbox.provider(&server.url);
    let task = ThreadId(7);
    assert!(
        provider
            .call(task, "memory_get", &json!({"ids":[1]}))
            .is_ok()
    );
    assert!(
        provider
            .call(task, "memory_get", &json!({"ids":[2]}))
            .is_err()
    );
    assert!(
        provider
            .call(MAIN, "memory_forget", &json!({"id":3}))
            .is_err()
    );
    assert!(
        provider
            .call(task, "memory_forget", &json!({"id":1}))
            .is_err()
    );
    assert!(
        provider
            .call(
                task,
                "memory_write",
                &json!({"text":"Leak","project":"another"})
            )
            .is_err()
    );
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.method == "DELETE" || r.method == "POST")
    );
    provider
        .call(task, "memory_search", &json!({"query":"needle & other"}))
        .unwrap();
    provider
        .context(&ContextRequest {
            thread: task,
            query: "q",
            fresh_session: true,
        })
        .unwrap();
    let requests = server.requests.lock().unwrap();
    let scoped = requests
        .iter()
        .filter(|r| {
            r.target.starts_with("/api/search") || r.target.starts_with("/api/context/inject")
        })
        .collect::<Vec<_>>();
    assert_eq!(scoped.len(), 4);
    assert!(
        scoped
            .iter()
            .all(|r| r.target.contains("project=relay-contract-test")
                && (r.target.contains("platformSource=relay-task-7")
                    || r.target.contains("platformSource=relay-global")))
    );
    assert!(
        scoped
            .iter()
            .any(|r| r.target.contains("needle%20%26%20other"))
    );
}

#[test]
fn unsupported_native_semantics_are_absent_from_both_ui_capabilities_and_mcp() {
    let sandbox = Sandbox::new();
    let provider = sandbox.provider("http://127.0.0.1:9");
    let info = provider.provider();
    assert_eq!(info.id, "claude-mem");
    assert!(info.capabilities.forget_memory && info.capabilities.retry);
    assert!(provider.tools(RETIRED_OBSERVER).is_empty());
    let store = ResidentStore::open(sandbox.0.clone()).unwrap();
    let mut output = vec![];
    protocol::serve(&store,&provider,MAIN,std::io::Cursor::new(b"{\"id\":1,\"method\":\"tools/list\"}\n{\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"memory_confirm\",\"arguments\":{\"id\":1}}}\n"),&mut output).unwrap();
    let lines = String::from_utf8(output).unwrap();
    let messages = lines
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect::<Vec<_>>();
    let tools = messages[0]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "create_task"));
    assert!(tools.iter().any(|t| t["name"] == "memory_summaries"));
    assert!(
        !tools
            .iter()
            .any(|t| t["name"] == "memory_commit_job" || t["name"] == "memory_confirm")
    );
    assert_eq!(messages[1]["result"]["isError"], true);
}

#[test]
fn config_validates_loopback_preserves_invalid_files_and_invalidates_execution_identity() {
    let sandbox = Sandbox::new();
    assert_eq!(
        MemoryConfig::load(&sandbox.0).unwrap().provider,
        ProviderKind::ClaudeMem
    );
    for url in [
        "https://127.0.0.1:8",
        "http://example.com:8",
        "http://127.0.0.1:8/path",
        "http://user@127.0.0.1:8",
        "http://127.0.0.1:8/?q=x",
        "http://127.0.0.1",
    ] {
        assert!(sandbox.config(url).validate().is_err(), "{url}");
    }
    assert!(sandbox.config("http://[::1]:37777").validate().is_ok());
    let other = Sandbox::new();
    let inferred = MemoryConfig {
        claude_mem_url: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    };
    let first = ClaudeMemProvider::open(&sandbox.0, inferred.clone()).unwrap();
    let moved = ClaudeMemProvider::open(&other.0, inferred).unwrap();
    assert_ne!(
        first.identity(),
        moved.identity(),
        "Implicit namespaces must invalidate checkpoints after a data-directory move"
    );
    assert_eq!(
        sandbox.provider("http://127.0.0.1:9").identity(),
        other.provider("http://127.0.0.1:9").identity(),
        "An explicit shared namespace keeps the same provider identity"
    );
    let a = MemoryConfig::default();
    let b = sandbox.config("http://127.0.0.1:8");
    assert_ne!(a.identity(), b.identity());
    assert_ne!(
        crate::session_key(&AgentSnapshot::default(), Some(&a.identity())),
        crate::session_key(&AgentSnapshot::default(), Some(&b.identity()))
    );
    b.save(&sandbox.0).unwrap();
    assert_eq!(
        MemoryConfig::load(&sandbox.0).unwrap().identity(),
        b.identity()
    );
    fs::write(
        sandbox.0.join("memory-provider.json"),
        b"{\"version\":99,\"provider\":\"native\"}",
    )
    .unwrap();
    assert!(MemoryConfig::load(&sandbox.0).is_err());
    assert!(
        run_cli(
            &sandbox.0,
            &["--memory-provider-set".into(), "native".into()]
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(sandbox.0.join("memory-provider.json")).unwrap(),
        "{\"version\":99,\"provider\":\"native\"}"
    );
}

#[test]
fn history_import_is_opt_in_and_disabled_context_does_no_io() {
    let sandbox = Sandbox::new();
    let mut config = sandbox.config("http://127.0.0.1:9");
    config.context = ContextMode::Disabled;
    let provider = ClaudeMemProvider::open(&sandbox.0, config).unwrap();
    assert!(!provider.imports_history());
    provider
        .record(&MemoryEvent::Archive {
            directory: None,
            client: "codex".into(),
            session: "old".into(),
            title: "Old".into(),
            messages: vec![(1, "user".into(), "old history".into())],
        })
        .unwrap();
    provider
        .record(&MemoryEvent::User {
            directory: std::env::temp_dir(),
            thread: RETIRED_OBSERVER,
            message: 1,
            text: "never recursively observe".into(),
        })
        .unwrap();
    assert_eq!(
        provider
            .context(&ContextRequest {
                thread: MAIN,
                query: "q",
                fresh_session: true
            })
            .unwrap(),
        ""
    );
    assert_eq!(provider.drain().unwrap()["accepted"], 0);
    assert!(!ContextMode::Session.applies(false));
    assert!(ContextMode::Session.applies(true));
}

#[test]
#[cfg(feature = "test-support")]
fn runtime_captures_tool_results_before_turn_end_without_native_memory_tables() {
    use crate::AgentRuntime;
    use relay_acp::Event;
    let sandbox = Sandbox::new();
    let server = Mock::new(acknowledge);
    let store = Arc::new(ResidentStore::open(sandbox.0.clone()).unwrap());
    let mut config = sandbox.config(&server.url);
    config.context = ContextMode::Disabled;
    let provider = Arc::new(ClaudeMemProvider::open(&sandbox.0, config).unwrap());
    let runtime = AgentRuntime::with_memory(sandbox.0.clone(), store.clone(), provider.clone());
    runtime.snapshot(MAIN);
    let (connection, receiver) = relay_acp::test_connection();
    runtime.connections.lock().unwrap().insert(MAIN, connection);
    runtime.shared.event(
        MAIN,
        0,
        Event::Ready {
            session_id: "provider-runtime-test".into(),
            configs: vec![],
            resumed: false,
        },
    );
    runtime
        .dispatch(
            MAIN,
            AgentCommand::Send("Remember the queue decision".into()),
        )
        .unwrap();
    let start = std::time::Instant::now();
    let context = loop {
        if let Ok(relay_acp::Command::Prompt { context, .. }) = receiver.try_recv() {
            break context.unwrap();
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    };
    assert!(context.contains("relay_runtime"));
    assert!(!context.contains("relay_memory"));
    runtime.shared.event(
        MAIN,
        0,
        Event::Tool(ToolActivity {
            id: "edit-queue".into(),
            title: "Edit".into(),
            status: "completed".into(),
            input: "queue.rs".into(),
            output: "WAL enabled".into(),
        }),
    );
    assert_eq!(
        provider.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"],
        2
    );
    assert_eq!(
        runtime.snapshot(MAIN).messages.last().unwrap().status,
        MessageStatus::Streaming
    );
    runtime.shared.event(
        MAIN,
        0,
        Event::Text("WAL was added; recovery remains unverified.".into()),
    );
    runtime.shared.event(
        MAIN,
        0,
        Event::TurnEnded {
            outcome: TurnOutcome::Complete,
            usage: None,
        },
    );
    runtime.shutdown();
    provider.drain().unwrap();
    provider.drain().unwrap();
    assert_eq!(
        server
            .posts()
            .iter()
            .filter(|r| r.target == "/api/sessions/summarize")
            .count(),
        1
    );
    assert_eq!(
        server
            .posts()
            .iter()
            .filter(|r| r.target == "/api/sessions/session-end")
            .count(),
        1
    );
    assert!(
        server
            .posts()
            .iter()
            .filter(|r| r.target != "/api/sessions/session-end")
            .all(|r| r.body["cwd"].is_string())
    );
    let db = store.db.lock().unwrap();
    for table in ["sources", "memories", "jobs", "embedding_jobs"] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?)",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !exists,
            "A fresh Relay database must not create the retired {table} table"
        );
    }
}

#[test]
fn a_crashed_post_becomes_uncertain_only_under_the_delivery_owner_lock() {
    let sandbox = Sandbox::new();
    let server = Mock::new(acknowledge);
    let provider = sandbox.provider(&server.url);
    provider.record(&turn()).unwrap();
    let directory = fs::read_dir(sandbox.0.join("memory-providers"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let db = rusqlite::Connection::open(directory.join("delivery.sqlite3")).unwrap();
    db.execute(
        "UPDATE delivery SET state='sending' WHERE id=(SELECT min(id) FROM delivery)",
        [],
    )
    .unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("delivery.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let reopened = sandbox.provider(&server.url);
    assert_eq!(reopened.drain().unwrap()["busy"], true);
    let state: String = db
        .query_row("SELECT state FROM delivery ORDER BY id LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "sending");
    drop(lock);
    reopened.drain().unwrap();
    assert!(server.posts().is_empty());
    let state: String = db
        .query_row("SELECT state FROM delivery ORDER BY id LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "uncertain");
}

#[test]
fn missing_scope_metadata_is_resolved_from_worker_sessions_before_disclosure() {
    let sandbox = Sandbox::new();
    let server = Mock::new(|r| {
        (
            200,
            if r.target.starts_with("/api/observation/") {
                json!({"id":1,"project":"relay-contract-test","memory_session_id":"upstream-session","title":"Private"})
            } else if r.target == "/api/sdk-sessions/batch" {
                json!([{"memory_session_id":"upstream-session","project":"relay-contract-test","platform_source":"relay-task-22"}])
            } else {
                json!({})
            },
        )
    });
    let provider = sandbox.provider(&server.url);
    assert!(
        provider
            .call(ThreadId(7), "memory_get", &json!({"ids":[1]}))
            .is_err()
    );
    let result = provider
        .call(ThreadId(22), "memory_get", &json!({"ids":[1]}))
        .unwrap();
    assert_eq!(
        result["data"]["observations"][0]["platform_source"],
        "relay-task-22"
    );
}

#[test]
#[ignore = "requires RELAY_TEST_CLAUDE_MEM_URL pointing to an isolated real Worker"]
fn real_worker_manual_memory_roundtrip() {
    // Opt-in: no model call; must point at a dedicated disposable Worker.
    let url = std::env::var("RELAY_TEST_CLAUDE_MEM_URL").expect("isolated Worker URL required");
    let sandbox = Sandbox::new();
    let mut config = sandbox.config(&url);
    config.namespace = Some(format!("relay-contract-{}", uuid::Uuid::new_v4()));
    let provider = ClaudeMemProvider::open(&sandbox.0, config).unwrap();
    let saved=provider.call(MAIN,"memory_write",&json!({"title":"Copper Finch 742","text":"Copper Finch 742 uses a durable SQLite WAL outbox. This synthetic record tests Relay's Claude-Mem adapter."})).unwrap();
    let id = saved["data"]["saved"]["id"].as_u64().unwrap();
    assert!(
        provider
            .detail(id)
            .unwrap()
            .body
            .contains("Copper Finch 742")
    );
    assert!(
        provider
            .overview(&MemoryQuery::default())
            .unwrap()
            .memories
            .iter()
            .any(|r| r.id == id)
    );
    let search = provider
        .call(MAIN, "memory_search", &json!({"query":"Copper Finch 742"}))
        .unwrap();
    assert!(
        search["data"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == id),
        "Saved observation must be retrievable by its distinctive title"
    );
    provider
        .call(MAIN, "memory_timeline", &json!({"observation_id":id}))
        .unwrap();
    let context = provider
        .context(&ContextRequest {
            thread: MAIN,
            query: "Copper",
            fresh_session: true,
        })
        .unwrap();
    // Upstream's concept filter excludes manual saves (which have no concepts).
    // They remain retrievable via search/get; do not pretend they were injected.
    assert!(context.contains("recent context"));
    assert!(!context.contains("work_state_write"));
    provider.call(MAIN, "memory_summaries", &json!({})).unwrap();
    provider.apply(MemoryCommand::ForgetMemory(id)).unwrap();
    assert!(provider.detail(id).is_err());
}

#[test]
#[ignore = "requires RELAY_TEST_CLAUDE_MEM_LIFECYCLE_URL; invokes the Worker's configured model"]
fn real_worker_extracts_observations_and_summary_from_relay_lifecycle() {
    // Explicit opt-in: runs the Worker's configured observation model on synthetic data.
    let url =
        std::env::var("RELAY_TEST_CLAUDE_MEM_LIFECYCLE_URL").expect("isolated Worker URL required");
    let sandbox = Sandbox::new();
    let mut config = sandbox.config(&url);
    config.namespace = Some(format!("relay-lifecycle-{}", uuid::Uuid::new_v4()));
    let provider = ClaudeMemProvider::open(&sandbox.0, config).unwrap();
    provider.record(&turn()).unwrap();
    provider.drain().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(240);
    loop {
        let observations = provider.overview(&MemoryQuery::default()).unwrap();
        let summaries = provider.call(MAIN, "memory_summaries", &json!({})).unwrap();
        if !observations.memories.is_empty()
            && !summaries["data"]["summaries"]
                .as_array()
                .unwrap()
                .is_empty()
        {
            let context = provider
                .context(&ContextRequest {
                    thread: MAIN,
                    query: "WAL",
                    fresh_session: true,
                })
                .unwrap();
            assert!(!context.contains("work_state_write"));
            assert!(
                observations
                    .memories
                    .iter()
                    .any(|m| context.contains(&m.title)),
                "Extracted observations must reach context"
            );
            eprintln!(
                "Real Claude-Mem lifecycle: {} observations, {} summaries",
                observations.memories.len(),
                summaries["data"]["summaries"].as_array().unwrap().len()
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Worker accepted lifecycle events but did not produce both observations and summary within 240s"
        );
        thread::sleep(Duration::from_secs(2));
    }
    provider
        .record(&MemoryEvent::SessionEnd {
            thread: MAIN,
            last_message: 2,
        })
        .unwrap();
    provider.drain().unwrap();
}

#[test]
fn old_native_config_selects_claude_mem_without_importing_history_or_altering_the_file() {
    let sandbox = Sandbox::new();
    let bytes = br#"{"version":1,"provider":"native","context":"disabled","import_history":true,"claude_mem_url":"http://127.0.0.1:9"}"#;
    fs::write(sandbox.0.join("memory-provider.json"), bytes).unwrap();
    let config = MemoryConfig::load(&sandbox.0).unwrap();
    assert_eq!(config.version, 2);
    assert_eq!(config.provider, ProviderKind::ClaudeMem);
    assert_eq!(config.context, ContextMode::Disabled);
    assert!(!config.import_history);
    let provider = super::open(&sandbox.0).unwrap();
    assert_eq!(provider.provider().id, "claude-mem");
    assert_eq!(
        fs::read(sandbox.0.join("memory-provider.json")).unwrap(),
        bytes
    );
    assert!(
        !sandbox.0.join("relay.sqlite3").exists(),
        "Opening memory must not initialize the coordinator database"
    );
    assert!(
        run_cli(
            &sandbox.0,
            &["--memory-provider-set".into(), "native".into()]
        )
        .is_err()
    );
    assert!(
        run_cli(
            &sandbox.0,
            &[
                "--memory-configure".into(),
                "unused".into(),
                "unused".into()
            ]
        )
        .is_err()
    );
    assert_eq!(
        fs::read(sandbox.0.join("memory-provider.json")).unwrap(),
        bytes
    );
}

#[test]
fn existing_claude_mem_config_keeps_its_delivery_queue_after_upgrade() {
    let sandbox = Sandbox::new();
    let config = sandbox.config("http://127.0.0.1:9");
    let provider = ClaudeMemProvider::open(&sandbox.0, config.clone()).unwrap();
    provider
        .record(&MemoryEvent::User {
            thread: MAIN,
            message: 1,
            text: "Pending event".into(),
            directory: sandbox.0.clone(),
        })
        .unwrap();
    let mut old = serde_json::to_value(&config).unwrap();
    old["version"] = json!(1);
    old["import_history"] = json!(true);
    fs::write(sandbox.0.join("memory-provider.json"), old.to_string()).unwrap();
    let upgraded = MemoryConfig::load(&sandbox.0).unwrap();
    assert!(upgraded.import_history);
    let reopened = ClaudeMemProvider::open(&sandbox.0, upgraded).unwrap();
    assert_eq!(
        reopened.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"],
        1
    );
}

#[cfg(feature = "test-support")]
#[test]
fn offline_worker_preserves_capture_and_allows_the_agent_turn_to_run() {
    use crate::{AgentRuntime, Event};
    let sandbox = Sandbox::new();
    let server = Mock::new(|_| (503, json!({"error":"Worker offline"})));
    let provider = Arc::new(sandbox.provider(&server.url));
    let store = Arc::new(ResidentStore::open(sandbox.0.clone()).unwrap());
    let runtime = AgentRuntime::with_memory(sandbox.0.clone(), store, provider.clone());
    let (connection, receiver) = relay_acp::test_connection();
    runtime.connections.lock().unwrap().insert(MAIN, connection);
    runtime.shared.event(
        MAIN,
        0,
        Event::Ready {
            session_id: "offline-case".into(),
            configs: vec![],
            resumed: false,
        },
    );
    runtime
        .send_turn(MAIN, "Continue working while memory is offline".into())
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        if let Ok(relay_acp::Command::Prompt { context, .. }) = receiver.try_recv() {
            let context = context.unwrap();
            assert!(context.contains("relay_runtime"));
            assert!(context.contains("Memory context is unavailable"));
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        runtime
            .snapshot(MAIN)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Memory context unavailable"))
    );
    assert_eq!(
        provider.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"],
        1
    );
    provider.drain().unwrap();
    assert_eq!(
        provider.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"],
        1
    );
    runtime.shutdown();
    let reopened = sandbox.provider(&server.url);
    assert!(
        reopened.call(MAIN, "memory_status", &json!({})).unwrap()["delivery"]["counts"]["pending"]
            .as_u64()
            .unwrap()
            >= 1
    );
}
