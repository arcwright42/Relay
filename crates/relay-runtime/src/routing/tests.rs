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
                memory: (1..=4)
                    .map(|id| relay_core::MemoryItem {
                        id: relay_core::MemoryId(id),
                        kind: relay_core::MemoryKind::Fact,
                        name: format!("Routing title {id} {}", "中".repeat(85)),
                        content: "PRIVATE_MEMORY_CONTENT".into(),
                        source: Some(relay_core::MemorySource::Message { message_id: 999 }),
                    })
                    .collect(),
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
        Ok(None),
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
    let titles =
        body["questions"]["destination"]["criteria"]["project_42"]["project_memory_titles"]
            .as_array()
            .unwrap();
    assert_eq!(titles.len(), 3);
    for (title, id) in titles.iter().zip([4, 3, 2]) {
        let title = title.as_str().unwrap();
        assert!(title.starts_with(&format!("Routing title {id} ")));
        assert_eq!(title.chars().count(), 80);
    }
    assert!(!request.contains("Routing title 1"));
    for excluded in [
        "PRIVATE_NOTE",
        "PRIVATE_REPLY",
        "PRIVATE_INSTRUCTIONS",
        "PRIVATE_MEMORY_CONTENT",
        "source_message_id",
    ] {
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
        (400, Duration::ZERO, RoutingError::InvalidResponse),
        (401, Duration::ZERO, RoutingError::Unauthorized),
        (402, Duration::ZERO, RoutingError::QuotaExceeded),
        (413, Duration::ZERO, RoutingError::InputTooLarge),
        (422, Duration::ZERO, RoutingError::InvalidResponse),
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
    for provider in [
        RoutingProvider::OpenRouter,
        RoutingProvider::TypeSafe,
        RoutingProvider::Vercel,
    ] {
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
        r#"{"version":1,"provider":"openrouter","key":""}"#,
        r#"{"version":1,"provider":"openrouter","key":"bad key"}"#,
    ] {
        assert!(credentials::decode(bad.as_bytes()).is_err());
    }
}

struct UntouchedKeychain;
impl Credentials for UntouchedKeychain {
    fn read(&self) -> Result<Option<Credential>, RoutingError> {
        panic!("Environment configuration must not access Keychain")
    }
    fn write(&self, _: Option<&Credential>) -> Result<(), RoutingError> {
        panic!("Environment configuration must not write to Keychain")
    }
}

#[test]
fn openrouter_http_routes_reuse_and_new_with_environment_key_and_versioned_response() {
    for (choice, existing, new, expected) in [
        (
            "project_42",
            0.96,
            0.02,
            RouteTarget::Existing(ProjectId(42)),
        ),
        (NEW, 0.02, 0.96, RouteTarget::NewProject),
    ] {
        let mut reply = response(choice, existing, new, 0.02, 0.94);
        reply["model"] = json!("typesafe/jev-1.13-20260917");
        reply["id"] = json!("gen-dec-test");
        reply["provider"] = json!("TypeSafe");
        let (url, server) = server(200, reply.to_string(), Duration::ZERO);
        let router = JevRouter::with_credentials(
            Arc::new(Projects::default()),
            Arc::new(Agents),
            Box::new(UntouchedKeychain),
            credentials::from_environment(Ok("openrouter-test-key".into()), Path::new("unused")),
            Some(url.replace("/v1/systemone", "/api/alpha/decisions")),
            Duration::from_secs(2),
        );
        let snapshot = router.snapshot();
        assert_eq!(snapshot.provider, RoutingProvider::OpenRouter);
        assert_eq!(
            snapshot.credential_source,
            Some(RoutingCredentialSource::Environment)
        );
        assert!(snapshot.configured);
        let decision = router.decide("继续完善 Relay 的项目界面").unwrap();
        assert_eq!(decision.automatic, Some(expected));
        assert_eq!(decision.model, "typesafe/jev-1.13-20260917");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/alpha/decisions HTTP/1.1\r\n"));
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer openrouter-test-key")
        );
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(
            headers
                .to_lowercase()
                .contains("content-type: application/json")
        );
        assert!(!body.contains("openrouter-test-key"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["model"], OPENROUTER_MODEL);
        assert_eq!(body["state"]["user_request"], "继续完善 Relay 的项目界面");
        let criteria = &body["questions"]["destination"]["criteria"];
        assert_eq!(criteria["project_42"]["project_name"], "Relay");
        assert_eq!(
            criteria["project_42"]["description_excerpt"],
            "A Rust desktop agent client"
        );
        assert!(criteria[NEW].is_string());
        assert!(criteria[UNCLEAR].is_string());
        assert_eq!(router.remove_key(), Err(RoutingError::ExternallyConfigured));
        assert_eq!(
            router.save_key(RoutingProvider::Vercel, "other-key".into()),
            Err(RoutingError::ExternallyConfigured)
        );
        assert_eq!(router.snapshot(), snapshot);
    }
}

#[test]
fn openrouter_model_validation_accepts_only_the_requested_release_and_date_suffix() {
    let router = router(
        ENDPOINT.into(),
        Duration::from_secs(1),
        Arc::new(Projects::default()),
    );
    let (_, _, targets) = router.request("A task").unwrap();
    for (model, valid) in [
        (OPENROUTER_MODEL, true),
        ("typesafe/jev-1.13-20260917", true),
        ("typesafe/jev-1.14-20260917", false),
        ("typesafe/jev-1.13-other", false),
        ("typesafe/jev-1.13-20260917-extra", false),
        ("typesafe/jev-1.13-202609", false),
        (MODEL, false),
    ] {
        let mut answer = response("project_42", 0.95, 0.03, 0.02, 0.925);
        answer["model"] = json!(model);
        let parsed = parse(
            &serde_json::to_vec(&answer).unwrap(),
            &targets,
            7,
            OPENROUTER_MODEL,
        );
        assert_eq!(parsed.is_ok(), valid, "{model}");
        if let Ok(decision) = parsed {
            assert_eq!(
                decision.automatic,
                Some(RouteTarget::Existing(ProjectId(42)))
            );
        }
    }
}

#[test]
fn dotenv_parsing_precedence_and_keychain_fallback_never_mutate_process_environment() {
    use std::env::VarError;
    let root = std::env::temp_dir().join(format!("relay-env-{}", crate::installer::unique_id()));
    std::fs::create_dir_all(&root).unwrap();
    let env_file = root.join(".env");
    let before = std::env::var_os("OPENROUTER_API_KEY");
    std::fs::write(&env_file, "# Jev configuration\nexport OPENROUTER_API_KEY = 'file-test-key' # inline comment\nOPENROUTER_API_KEY=second-key\nUNRELATED=ignore-me\n").unwrap();
    let key = credentials::from_environment(Err(VarError::NotPresent), &env_file)
        .unwrap()
        .unwrap();
    assert_eq!(key.credential.key, "file-test-key");
    assert_eq!(key.source, RoutingCredentialSource::EnvFile);
    let key = credentials::from_environment(Ok("process-test-key".into()), &env_file)
        .unwrap()
        .unwrap();
    assert_eq!(key.credential.key, "process-test-key");
    assert_eq!(key.source, RoutingCredentialSource::Environment);
    let configured = JevRouter::with_credentials(
        Arc::new(Projects::default()),
        Arc::new(Agents),
        Box::new(UntouchedKeychain),
        credentials::from_environment(Err(VarError::NotPresent), &env_file),
        None,
        Duration::from_secs(1),
    );
    assert_eq!(
        configured.snapshot().credential_source,
        Some(RoutingCredentialSource::EnvFile)
    );
    assert_eq!(
        configured.remove_key(),
        Err(RoutingError::ExternallyConfigured)
    );
    for contents in ["OPENROUTER_API_KEY=\n", "UNRELATED=ignore-me\n"] {
        std::fs::write(&env_file, contents).unwrap();
        let configured = JevRouter::with_credentials(
            Arc::new(Projects::default()),
            Arc::new(Agents),
            Box::new(MemoryKey(Mutex::new(Some(Credential {
                provider: RoutingProvider::Vercel,
                key: "saved-key".into(),
            })))),
            credentials::from_environment(Err(VarError::NotPresent), &env_file),
            None,
            Duration::from_secs(1),
        );
        assert_eq!(configured.snapshot().provider, RoutingProvider::Vercel);
        assert_eq!(
            configured.snapshot().credential_source,
            Some(RoutingCredentialSource::Keychain)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        credentials::from_environment(Err(VarError::NotPresent), &env_file)
            .unwrap()
            .is_none()
    );
    assert_eq!(std::env::var_os("OPENROUTER_API_KEY"), before);
}

#[test]
fn broken_external_configuration_is_reported_without_falling_back_to_saved_keys() {
    use std::env::VarError;
    let root =
        std::env::temp_dir().join(format!("relay-bad-env-{}", crate::installer::unique_id()));
    std::fs::create_dir_all(&root).unwrap();
    let env_file = root.join(".env");
    for (contents, process_key, expected) in [
        (
            "OPENROUTER_API_KEY='unterminated\n",
            Err(VarError::NotPresent),
            RoutingError::ConfigurationFile,
        ),
        (
            "OPENROUTER_API_KEY='bad key'\n",
            Err(VarError::NotPresent),
            RoutingError::InvalidKey,
        ),
        (
            "OPENROUTER_API_KEY='valid-file-key'\n",
            Ok("bad\nkey".into()),
            RoutingError::InvalidKey,
        ),
    ] {
        std::fs::write(&env_file, contents).unwrap();
        let router = JevRouter::with_credentials(
            Arc::new(Projects::default()),
            Arc::new(Agents),
            Box::new(UntouchedKeychain),
            credentials::from_environment(process_key, &env_file),
            None,
            Duration::from_secs(1),
        );
        assert!(!router.snapshot().configured);
        assert_eq!(router.snapshot().error, Some(expected));
        assert_eq!(router.decide("A task").unwrap_err(), expected);
        assert_eq!(
            router.save_key(RoutingProvider::TypeSafe, "other-key".into()),
            Err(expected)
        );
    }
    std::fs::write(&env_file, "not valid env syntax").unwrap();
    assert!(
        credentials::from_environment(Ok("valid-process-key".into()), &env_file)
            .unwrap()
            .is_some()
    );
    std::fs::remove_dir_all(root).unwrap();
}
