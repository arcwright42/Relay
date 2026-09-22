use super::*;
use relay_core::{ContextId, ContextItem, Project, ProjectId, agents::*, projects::*};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

struct Projects(Mutex<ProjectCatalog>);
impl Default for Projects {
    fn default() -> Self {
        Self(Mutex::new(ProjectCatalog {
            revision: 7,
            error: None,
            projects: vec![Project {
                id: ProjectId(42),
                revision: 1,
                name: "Relay".into(),
                description: "A Rust desktop agent client".into(),
                instructions: "PRIVATE_INSTRUCTIONS".into(),
                context: vec![ContextItem {
                    id: ContextId(1),
                    name: "Private".into(),
                    content: "PRIVATE_NOTE".into(),
                    included: true,
                }],
            }],
        }))
    }
}
impl ProjectService for Projects {
    fn snapshot(&self) -> ProjectCatalog {
        self.0.lock().unwrap().clone()
    }
    fn apply(&self, _: ProjectCommand) -> Result<ProjectId, String> {
        panic!("Deciding must never mutate projects")
    }
}
struct Agents;
impl AgentService for Agents {
    fn revision(&self) -> u64 {
        0
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        AgentSnapshot {
            messages: [
                (MessageRole::User, "Build the workspace"),
                (MessageRole::Assistant, "PRIVATE_REPLY"),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (role, text))| ChatMessage {
                id: index as u64,
                role,
                text: text.into(),
                status: MessageStatus::Complete,
                tools: vec![],
                metrics: None,
            })
            .collect(),
            ..Default::default()
        }
    }
    fn dispatch(&self, _: ProjectId, _: AgentCommand) -> Result<(), String> {
        panic!("Deciding must never run an agent")
    }
}
struct MemoryKey(Mutex<Option<Credential>>);
impl Credentials for MemoryKey {
    fn read(&self) -> Result<Option<Credential>, RoutingError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn write(&self, credential: Option<&Credential>) -> Result<(), RoutingError> {
        *self.0.lock().unwrap() = credential.cloned();
        Ok(())
    }
}
fn router(endpoint: String, timeout: Duration, projects: Arc<Projects>) -> JevRouter {
    JevRouter::with_credentials(
        projects,
        Arc::new(Agents),
        Box::new(MemoryKey(Mutex::new(Some(Credential {
            provider: RoutingProvider::TypeSafe,
            key: "test-key".into(),
        })))),
        Some(endpoint),
        timeout,
    )
}
fn response(choice: &str, existing: f64, new: f64, unclear: f64, confidence: f64) -> Value {
    json!({"model": MODEL, "answers": {"destination": {"type": "choice", "choice": choice, "confidence": confidence,
        "probabilities": {"project_42": existing, NEW: new, UNCLEAR: unclear}}}, "usage": {"input_tokens": 100, "output_tokens": 3}})
}
fn server(status: u16, body: String, delay: Duration) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let end;
        loop {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                end = request.len();
                break;
            }
        }
        let headers = String::from_utf8(request.clone()).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        request.resize(end + length, 0);
        stream.read_exact(&mut request[end..]).unwrap();
        thread::sleep(delay);
        let reply = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(reply.as_bytes());
        String::from_utf8(request).unwrap()
    });
    (url, task)
}

#[test]
fn actual_http_uses_documented_schema_and_excludes_full_context() {
    let (url, server) = server(
        200,
        response("project_42", 0.95, 0.03, 0.02, 0.925).to_string(),
        Duration::ZERO,
    );
    let router = router(url, Duration::from_secs(2), Arc::new(Projects::default()));
    let result = router.decide("继续完善 Relay 的项目界面").unwrap();
    assert_eq!(result.automatic, Some(RouteTarget::Existing(ProjectId(42))));
    assert_eq!(result.catalog_revision, 7);
    let request = server.join().unwrap();
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer test-key")
    );
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["questions"]["destination"]["type"], "choice");
    assert!(request.contains("Build the workspace"));
    for excluded in ["PRIVATE_NOTE", "PRIVATE_REPLY", "PRIVATE_INSTRUCTIONS"] {
        assert!(!request.contains(excluded));
    }
}

#[test]
fn probability_thresholds_distinguish_reuse_new_and_ambiguous() {
    let router = router(
        ENDPOINT.into(),
        Duration::from_secs(1),
        Arc::new(Projects::default()),
    );
    let (_, _, targets) = router.request("A task").unwrap();
    for (answer, expected) in [
        (
            response("project_42", 0.90, 0.08, 0.02, 0.85),
            Some(RouteTarget::Existing(ProjectId(42))),
        ),
        (
            response(NEW, 0.03, 0.95, 0.02, 0.925),
            Some(RouteTarget::NewProject),
        ),
        (response(NEW, 0.10, 0.88, 0.02, 0.82), None),
        (response("project_42", 0.50, 0.49, 0.01, 0.25), None),
        (response(UNCLEAR, 0.01, 0.01, 0.98, 0.97), None),
        (response("project_42", 0.95, 0.03, 0.02, 0.5), None),
    ] {
        assert_eq!(
            parse(&serde_json::to_vec(&answer).unwrap(), &targets, 7, MODEL)
                .unwrap()
                .automatic,
            expected
        );
    }
}

#[test]
fn malformed_foreign_or_inconsistent_decisions_are_rejected() {
    let router = router(
        ENDPOINT.into(),
        Duration::from_secs(1),
        Arc::new(Projects::default()),
    );
    let (_, _, targets) = router.request("A task").unwrap();
    let baseline = response("project_42", 0.95, 0.03, 0.02, 0.925);
    for (pointer, value) in [
        ("/model", json!("unexpected-model")),
        ("/answers/destination/choice", json!("project_999")),
        ("/answers/destination/type", json!("noul")),
        ("/answers/destination/confidence", json!(1.1)),
        ("/answers/destination/probabilities/project_42", json!(-0.1)),
        (
            "/answers/destination/probabilities/new_project",
            json!(0.95),
        ),
    ] {
        let mut answer = baseline.clone();
        *answer.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            parse(&serde_json::to_vec(&answer).unwrap(), &targets, 7, MODEL).unwrap_err(),
            RoutingError::InvalidResponse
        );
    }
    let mut missing = baseline;
    missing["answers"]["destination"]["probabilities"]
        .as_object_mut()
        .unwrap()
        .remove(UNCLEAR);
    assert!(parse(&serde_json::to_vec(&missing).unwrap(), &targets, 7, MODEL).is_err());
}

#[test]
fn transport_errors_and_timeouts_never_become_new_projects() {
    for (status, delay, error) in [
        (401, Duration::ZERO, RoutingError::Unauthorized),
        (429, Duration::ZERO, RoutingError::RateLimited),
        (500, Duration::ZERO, RoutingError::Unavailable),
        (302, Duration::ZERO, RoutingError::Unavailable),
        (200, Duration::from_millis(100), RoutingError::Unavailable),
        (200, Duration::ZERO, RoutingError::InvalidResponse),
    ] {
        let (url, server) = server(status, "Do not expose this response body".into(), delay);
        let router = router(
            url,
            Duration::from_millis(50),
            Arc::new(Projects::default()),
        );
        assert_eq!(router.decide("A task").unwrap_err(), error);
        server.join().unwrap();
    }
}

#[test]
fn configuration_and_input_checks_do_not_make_network_calls() {
    let router = router(
        "http://127.0.0.1:1".into(),
        Duration::from_secs(1),
        Arc::new(Projects::default()),
    );
    assert!(router.snapshot().configured);
    assert_eq!(
        router.save_key(RoutingProvider::TypeSafe, "bad\nkey".into()),
        Err(RoutingError::InvalidKey)
    );
    assert!(router.snapshot().configured);
    assert_eq!(
        router.decide(&"x".repeat(12_001)).unwrap_err(),
        RoutingError::InputTooLarge
    );
    router.remove_key().unwrap();
    assert!(!router.snapshot().configured);
    assert_eq!(
        router.decide("A task").unwrap_err(),
        RoutingError::NotConfigured
    );
    router
        .save_key(RoutingProvider::TypeSafe, "new-test-key".into())
        .unwrap();
    assert!(router.snapshot().configured);
}

#[test]
fn project_creation_requires_the_catalog_that_was_classified() {
    let root =
        std::env::temp_dir().join(format!("relay-routing-{}", crate::installer::unique_id()));
    let projects = crate::ProjectStore::new(root.clone());
    let revision = projects.revision();
    let command = ProjectCommand::CreateAtRevision {
        expected_catalog_revision: revision,
        draft: ProjectDraft {
            name: "New request".into(),
            ..Default::default()
        },
    };
    let id = projects.apply(command.clone()).unwrap();
    assert!(projects.apply(command).is_err());
    assert!(projects.project(id).is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vercel_compatible_http_uses_its_model_and_saved_gateway_credential() {
    let mut reply = response(NEW, 0.02, 0.96, 0.02, 0.94);
    reply["model"] = json!(VERCEL_MODEL);
    let (url, server) = server(200, reply.to_string(), Duration::ZERO);
    let router = router(url, Duration::from_secs(2), Arc::new(Projects::default()));
    router
        .save_key(RoutingProvider::Vercel, "gateway-test-key".into())
        .unwrap();
    assert_eq!(router.snapshot().provider, RoutingProvider::Vercel);
    assert_eq!(
        router.decide("Plan my holiday").unwrap().automatic,
        Some(RouteTarget::NewProject)
    );
    let request = server.join().unwrap();
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer gateway-test-key")
    );
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["model"], VERCEL_MODEL);
    assert_eq!(body["questions"]["destination"]["type"], "choice");
}

#[test]
fn provider_and_key_are_one_versioned_record_and_bad_records_are_preserved() {
    for provider in [RoutingProvider::TypeSafe, RoutingProvider::Vercel] {
        let credential = Credential {
            provider,
            key: "test-key".into(),
        };
        let stored = credentials::encode(&credential).unwrap();
        let restored = credentials::decode(&stored).unwrap();
        assert_eq!(restored.provider, provider);
        assert_eq!(restored.key, "test-key");
    }
    for bad in [
        r#"{"version":2,"provider":"vercel","key":"test"}"#,
        r#"{"version":1,"provider":"unknown","key":"test"}"#,
    ] {
        assert!(credentials::decode(bad.as_bytes()).is_err());
    }
}
