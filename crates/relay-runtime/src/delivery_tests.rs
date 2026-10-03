use super::*;
use relay_core::projects::{MemoryKind, MemorySource, ProjectCommand, ProjectDraft};

struct Fixture {
    runtime: AgentRuntime,
    projects: Arc<ProjectStore>,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("relay-delivery-{}", installer::unique_id()));
        let projects = Arc::new(ProjectStore::new(root.clone()));
        let runtime = AgentRuntime::new(root.clone(), projects.clone());
        Self {
            runtime,
            projects,
            root,
        }
    }
    fn note(&self, content: &str) {
        let project = self.projects.project(ProjectId(1)).unwrap();
        self.projects
            .apply(ProjectCommand::SaveContext {
                project: project.id,
                expected_revision: project.revision,
                id: project.context.first().map(|item| item.id),
                name: "Reference".into(),
                content: content.into(),
                included: true,
            })
            .unwrap();
    }
    fn memory(&self, content: &str) {
        let project = self.projects.project(ProjectId(1)).unwrap();
        self.projects
            .apply(ProjectCommand::SaveMemory {
                project: project.id,
                expected_revision: project.revision,
                id: project.memory.first().map(|item| item.id),
                kind: MemoryKind::Decision,
                name: "Chosen architecture".into(),
                content: content.into(),
                source: Some(MemorySource::Message { message_id: 12 }),
            })
            .unwrap();
    }
    fn ready(&self, resumed: bool) {
        self.runtime.shared.event(
            ProjectId(1),
            0,
            Event::Ready {
                session_id: "test-session".into(),
                configs: vec![],
                resumed,
            },
        );
    }
    fn finish(&self, outcome: TurnOutcome) {
        self.runtime.shared.event(
            ProjectId(1),
            0,
            Event::TurnEnded {
                outcome,
                usage: None,
            },
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.runtime.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

macro_rules! prompt {
    ($runtime:expr, $receiver:expr, $text:expr) => {{
        $runtime
            .dispatch(ProjectId(1), AgentCommand::Send($text.into()))
            .unwrap();
        let started = Instant::now();
        loop {
            if let Ok(command) = $receiver.try_recv() {
                let AcpCommand::Prompt { text, context } = command else {
                    panic!("expected prompt")
                };
                assert_eq!(text, $text);
                break context;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "prompt was not delivered"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }};
}

#[test]
fn a_custom_workfolder_still_delivers_the_personal_file_destination_to_the_agent() {
    use relay_core::files::FileService;
    let fixture = Fixture::new();
    let custom = fixture.root.join("code-repository");
    std::fs::create_dir_all(&custom).unwrap();
    fixture
        .runtime
        .dispatch(
            ProjectId(1),
            AgentCommand::SetWorkingDirectory(custom.clone()),
        )
        .unwrap();
    let files = FileStore::new(fixture.root.clone());
    let personal = files.list(PathBuf::new()).unwrap().workspace;
    let (connection, received) = relay_acp::test_connection();
    fixture
        .runtime
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    // Changing folders advances the connection generation.
    let generation = fixture.runtime.shared.projects.lock().unwrap()[&ProjectId(1)].generation;
    fixture.runtime.shared.event(
        ProjectId(1),
        generation,
        Event::Ready {
            session_id: "personal-files-session".into(),
            configs: vec![],
            resumed: false,
        },
    );
    let context = prompt!(fixture.runtime, received, "Create a report").unwrap();
    let payload: serde_json::Value = serde_json::from_str(context.lines().last().unwrap()).unwrap();
    assert_eq!(
        payload["changes"]["files_directory"],
        personal.to_string_lossy().as_ref()
    );
    assert_eq!(
        fixture.runtime.snapshot(ProjectId(1)).working_directory,
        custom.canonicalize().unwrap()
    );
    fixture.runtime.shared.event(
        ProjectId(1),
        generation,
        Event::TurnEnded {
            outcome: TurnOutcome::Complete,
            usage: None,
        },
    );
    assert!(prompt!(fixture.runtime, received, "Continue").is_none());
    fixture.runtime.shared.event(
        ProjectId(1),
        generation,
        Event::TurnEnded {
            outcome: TurnOutcome::Complete,
            usage: None,
        },
    );
}

#[test]
fn memory_updates_removals_and_unconfirmed_changes_reach_the_next_acp_turn() {
    let fixture = Fixture::new();
    fixture.memory("Use the existing project store");
    let (connection, received) = relay_acp::test_connection();
    fixture
        .runtime
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    fixture.ready(false);
    let initial = prompt!(fixture.runtime, received, "First").unwrap();
    assert!(initial.contains("\"kind\":\"snapshot\""));
    assert!(initial.contains("Use the existing project store"));
    assert!(initial.contains("\"source_message_id\":12"));
    fixture.finish(TurnOutcome::Complete);
    assert!(prompt!(fixture.runtime, received, "Unchanged").is_none());
    fixture.finish(TurnOutcome::Complete);

    fixture.memory("Share memory through ACP context");
    let changed = prompt!(fixture.runtime, received, "Updated").unwrap();
    assert!(changed.contains("\"kind\":\"delta\""));
    assert!(changed.contains("\"upsert_memories\""));
    assert!(changed.contains("Share memory through ACP context"));
    assert!(!changed.contains("Use the existing project store"));
    fixture.finish(TurnOutcome::Cancelled);
    let retried = prompt!(fixture.runtime, received, "Continue").unwrap();
    assert!(retried.contains("\"kind\":\"snapshot\""));
    assert!(retried.contains("Share memory through ACP context"));
    fixture.finish(TurnOutcome::Complete);

    let project = fixture.projects.project(ProjectId(1)).unwrap();
    let id = project.memory[0].id;
    fixture
        .projects
        .apply(ProjectCommand::RemoveMemory {
            project: project.id,
            expected_revision: project.revision,
            id,
        })
        .unwrap();
    let removed = prompt!(fixture.runtime, received, "After removal").unwrap();
    assert!(removed.contains(&format!("\"remove_memory_ids\":[{}]", id.0)));
    assert!(!removed.contains("Share memory through ACP context"));
    fixture.finish(TurnOutcome::Complete);

    fixture.memory("Memory survives a new native session");
    fixture.ready(false);
    let new_session = prompt!(fixture.runtime, received, "New session").unwrap();
    assert!(new_session.contains("\"kind\":\"snapshot\""));
    assert!(new_session.contains("Memory survives a new native session"));
    fixture.finish(TurnOutcome::Complete);
    fixture.runtime.shutdown();
    let restored = AgentRuntime::new(fixture.root.clone(), fixture.projects.clone());
    let (connection, received) = relay_acp::test_connection();
    restored
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    restored.shared.event(
        ProjectId(1),
        0,
        Event::Ready {
            session_id: "test-session".into(),
            configs: vec![],
            resumed: true,
        },
    );
    assert!(prompt!(restored, received, "After restart").is_none());
    restored.shutdown();
}

#[test]
fn stable_session_sends_snapshot_once_then_only_updates_and_preserves_durable_cursor() {
    let fixture = Fixture::new();
    fixture.note("Original reference");
    let (connection, received) = relay_acp::test_connection();
    fixture
        .runtime
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    fixture.ready(false);
    let initial = prompt!(fixture.runtime, received, "First").unwrap();
    assert!(initial.contains("Original reference"));
    let saved = store::load(&fixture.root, ProjectId(1)).unwrap();
    assert!(
        saved.context_checkpoint.uncertain,
        "checkpoint must be persisted before dispatch"
    );
    assert!(saved.context_checkpoint.acknowledged.is_none());
    fixture.finish(TurnOutcome::Complete);
    assert!(prompt!(fixture.runtime, received, "Second").is_none());
    fixture.finish(TurnOutcome::Complete);
    fixture.note("Updated reference");
    let delta = prompt!(fixture.runtime, received, "Third").unwrap();
    assert!(delta.contains("\"kind\":\"delta\""));
    assert!(delta.contains("Updated reference"));
    assert!(!delta.contains("Original reference"));
    fixture.finish(TurnOutcome::Complete);
    let saved = store::load(&fixture.root, ProjectId(1)).unwrap();
    assert!(!saved.context_checkpoint.uncertain);
    let baseline = saved.context_checkpoint.baseline_revision;
    fixture.runtime.shutdown();
    let restored = AgentRuntime::new(fixture.root.clone(), fixture.projects.clone());
    let (connection, received) = relay_acp::test_connection();
    restored
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    restored.shared.event(
        ProjectId(1),
        0,
        Event::Ready {
            session_id: "test-session".into(),
            configs: vec![],
            resumed: true,
        },
    );
    assert!(prompt!(restored, received, "After restart").is_none());
    assert_eq!(
        restored.shared.projects.lock().unwrap()[&ProjectId(1)]
            .context_checkpoint
            .baseline_revision,
        baseline
    );
    restored.shutdown();
}

#[test]
fn cancelled_failed_and_refused_context_is_resynchronized_even_if_user_undoes_it() {
    for outcome in [
        TurnOutcome::Cancelled,
        TurnOutcome::Failed,
        TurnOutcome::Refused,
    ] {
        let fixture = Fixture::new();
        fixture.note("Baseline");
        let (connection, received) = relay_acp::test_connection();
        fixture
            .runtime
            .connections
            .lock()
            .unwrap()
            .insert(ProjectId(1), connection);
        fixture.ready(false);
        prompt!(fixture.runtime, received, "Initial");
        fixture.finish(TurnOutcome::Complete);
        fixture.note("Unconfirmed update");
        prompt!(fixture.runtime, received, "Could be interrupted");
        fixture.finish(outcome);
        assert!(
            store::load(&fixture.root, ProjectId(1))
                .unwrap()
                .context_checkpoint
                .uncertain
        );
        fixture.note("Baseline");
        let next = prompt!(fixture.runtime, received, "Continue").unwrap();
        assert!(next.contains("\"kind\":\"snapshot\""));
        assert!(next.contains("Baseline"));
        assert!(!next.contains("Unconfirmed update"));
    }
}

#[test]
fn editing_while_running_waits_for_next_turn_and_failed_resume_restores_history() {
    let fixture = Fixture::new();
    fixture.note("Before send");
    let (connection, received) = relay_acp::test_connection();
    fixture
        .runtime
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    fixture.ready(false);
    let first = prompt!(fixture.runtime, received, "First task").unwrap();
    fixture.note("Edited during turn");
    assert!(!first.contains("Edited during turn"));
    fixture.finish(TurnOutcome::Complete);
    let second = prompt!(fixture.runtime, received, "Next task").unwrap();
    assert!(second.contains("\"kind\":\"delta\""));
    assert!(second.contains("Edited during turn"));
    fixture.finish(TurnOutcome::Complete);
    fixture.ready(false);
    let restored = prompt!(fixture.runtime, received, "New session").unwrap();
    assert!(restored.contains("\"kind\":\"snapshot\""));
    assert!(restored.contains("First task"));
    assert!(restored.contains("Relay restored"));
    fixture.finish(TurnOutcome::Complete);
    assert!(prompt!(fixture.runtime, received, "Continue").is_none());
}

#[test]
fn text_latency_ignores_empty_chunks_and_tools_and_diagnostics_survive_restart() {
    let fixture = Fixture::new();
    let (connection, received) = relay_acp::test_connection();
    fixture
        .runtime
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    fixture.ready(false);
    prompt!(fixture.runtime, received, "Measure");
    fixture
        .runtime
        .shared
        .event(ProjectId(1), 0, Event::Text(String::new()));
    fixture.runtime.shared.event(
        ProjectId(1),
        0,
        Event::Tool(ToolActivity {
            id: "tool".into(),
            title: "Reading".into(),
            status: "completed".into(),
        }),
    );
    assert!(
        fixture
            .runtime
            .snapshot(ProjectId(1))
            .messages
            .last()
            .unwrap()
            .metrics
            .as_ref()
            .unwrap()
            .first_text_ms
            .is_none()
    );
    fixture
        .runtime
        .shared
        .event(ProjectId(1), 0, Event::Text("Visible".into()));
    let usage = TokenUsage {
        input_tokens: 100,
        output_tokens: 20,
        cached_read_tokens: Some(900),
        cached_write_tokens: None,
        thought_tokens: Some(10),
    };
    fixture.runtime.shared.event(
        ProjectId(1),
        0,
        Event::TurnEnded {
            outcome: TurnOutcome::Complete,
            usage: Some(usage.clone()),
        },
    );
    let messages = store::load(&fixture.root, ProjectId(1)).unwrap().messages;
    let metrics = messages
        .into_iter()
        .last()
        .unwrap()
        .into_message()
        .metrics
        .unwrap();
    assert_eq!(metrics.usage, Some(usage));
    assert!(metrics.first_text_ms.is_some());
    assert!(metrics.total_ms.unwrap() >= metrics.first_text_ms.unwrap());
    assert_eq!(metrics.context_kind, ContextDeliveryKind::Snapshot);
}

#[test]
fn persistence_failure_prevents_prompt_dispatch_and_another_project_cannot_supply_context() {
    let fixture = Fixture::new();
    fixture.note("PRIVATE PROJECT ONE");
    let other = fixture
        .projects
        .apply(ProjectCommand::Create(ProjectDraft {
            name: "Other".into(),
            ..Default::default()
        }))
        .unwrap();
    assert!(fixture.runtime.snapshot(other).messages.is_empty());
    let (connection, received) = relay_acp::test_connection();
    fixture
        .runtime
        .connections
        .lock()
        .unwrap()
        .insert(ProjectId(1), connection);
    fixture.ready(false);
    let file = fixture.root.join("projects/1/conversation.json");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    fixture
        .runtime
        .dispatch(ProjectId(1), AgentCommand::Send("Must not send".into()))
        .unwrap();
    let started = Instant::now();
    while fixture.runtime.snapshot(ProjectId(1)).status == ConnectionStatus::Running {
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(received.try_recv().is_err());
    assert!(fixture.runtime.snapshot(ProjectId(1)).error.is_some());
    assert!(fixture.runtime.snapshot(other).messages.is_empty());
}
