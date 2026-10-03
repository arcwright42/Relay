use super::dialogue::ProjectVoiceDialogue;
use crate::{ProjectStore, installer};
use relay_core::{ProjectId, agents::*, projects::*, routing::*, voice::*};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Router {
    decision: RouteDecision,
    calls: AtomicUsize,
}
impl RoutingService for Router {
    fn snapshot(&self) -> RoutingSnapshot {
        RoutingSnapshot::default()
    }
    fn save_key(&self, _: RoutingProvider, _: String) -> Result<(), RoutingError> {
        unreachable!()
    }
    fn remove_key(&self) -> Result<(), RoutingError> {
        unreachable!()
    }
    fn decide(&self, _: &str) -> Result<RouteDecision, RoutingError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.decision.clone())
    }
}
struct Agents {
    view: Mutex<AgentSnapshot>,
    commands: Mutex<Vec<AgentCommand>>,
    sends: Mutex<Vec<(ProjectId, String)>>,
}
impl Agents {
    fn new() -> Self {
        Self {
            view: Mutex::new(AgentSnapshot {
                status: ConnectionStatus::Ready,
                ..Default::default()
            }),
            commands: Mutex::default(),
            sends: Mutex::default(),
        }
    }
}
impl AgentService for Agents {
    fn revision(&self) -> u64 {
        0
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        self.view.lock().unwrap().clone()
    }
    fn dispatch(&self, _: ProjectId, command: AgentCommand) -> Result<(), String> {
        self.commands.lock().unwrap().push(command);
        Ok(())
    }
    fn send_turn(&self, project: ProjectId, prompt: String) -> Result<u64, String> {
        self.sends.lock().unwrap().push((project, prompt));
        let mut view = self.view.lock().unwrap();
        view.status = ConnectionStatus::Running;
        view.messages = vec![message(41, "voice reply", MessageStatus::Streaming)];
        Ok(41)
    }
}
fn message(id: u64, text: &str, status: MessageStatus) -> ChatMessage {
    ChatMessage {
        id,
        role: MessageRole::Assistant,
        text: text.into(),
        status,
        tools: vec![],
        metrics: None,
    }
}
struct Fixture {
    root: std::path::PathBuf,
    projects: Arc<ProjectStore>,
    router: Arc<Router>,
    agents: Arc<Agents>,
}
impl Fixture {
    fn new(automatic: Option<RouteTarget>) -> Self {
        let root =
            std::env::temp_dir().join(format!("relay-voice-routing-{}", installer::unique_id()));
        let projects = Arc::new(ProjectStore::new(root.clone()));
        let router = Arc::new(Router {
            decision: RouteDecision {
                catalog_revision: projects.revision(),
                automatic,
                options: vec![
                    RouteOption {
                        target: RouteTarget::Existing(ProjectId(1)),
                        probability: 0.6,
                    },
                    RouteOption {
                        target: RouteTarget::NewProject,
                        probability: 0.4,
                    },
                ],
                confidence: 0.6,
                elapsed_ms: 0,
                model: "fixture".into(),
            },
            calls: AtomicUsize::new(0),
        });
        Self {
            root,
            projects,
            router,
            agents: Arc::new(Agents::new()),
        }
    }
    fn dialogue(&self) -> ProjectVoiceDialogue {
        ProjectVoiceDialogue::new(
            self.router.clone(),
            self.projects.clone(),
            self.agents.clone(),
        )
    }
    fn complete(&self, progress: VoicePromptProgress) {
        if matches!(
            progress,
            VoicePromptProgress::Stage(VoiceTurnStage::WaitingForAgent)
        ) {
            let mut view = self.agents.view.lock().unwrap();
            view.messages[0].status = MessageStatus::Complete;
            view.messages.push(message(
                43,
                "a later unrelated reply",
                MessageStatus::Streaming,
            ));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn routed_prompt_uses_the_existing_project_session_and_only_its_correlated_reply() {
    let fixture = Fixture::new(Some(RouteTarget::Existing(ProjectId(1))));
    let result = fixture
        .dialogue()
        .respond("summarize", None, &AtomicBool::new(false), &|progress| {
            fixture.complete(progress)
        })
        .unwrap();
    let VoicePromptResult::Reply(text) = result else {
        panic!("expected reply")
    };
    assert_eq!(text, "voice reply");
    assert_eq!(
        *fixture.agents.sends.lock().unwrap(),
        [(ProjectId(1), "summarize".into())]
    );
    assert!(fixture.agents.commands.lock().unwrap().is_empty());
}

#[test]
fn ambiguous_route_executes_nothing_until_a_revision_bound_selection_is_made() {
    let fixture = Fixture::new(None);
    let dialogue = fixture.dialogue();
    let cancelled = AtomicBool::new(false);
    let result = dialogue
        .respond("new task", None, &cancelled, &|_| {})
        .unwrap();
    let VoicePromptResult::ChooseProject {
        catalog_revision,
        choices,
    } = result
    else {
        panic!("expected choices")
    };
    assert_eq!(choices.len(), 2);
    assert!(choices[0].project_name.is_some());
    assert!(fixture.agents.sends.lock().unwrap().is_empty());
    let before = fixture.projects.snapshot().projects.len();
    let result = dialogue
        .respond(
            "new task",
            Some(VoiceRouteSelection {
                catalog_revision,
                target: RouteTarget::NewProject,
            }),
            &cancelled,
            &|progress| fixture.complete(progress),
        )
        .unwrap();
    assert!(matches!(result, VoicePromptResult::Reply(_)));
    assert_eq!(fixture.router.calls.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.projects.snapshot().projects.len(), before + 1);
    let sends = fixture.agents.sends.lock().unwrap();
    assert_eq!(sends.len(), 1);
    assert_eq!(
        fixture.projects.project(sends[0].0).unwrap().name,
        "new task"
    );
}

#[test]
fn changed_catalog_and_busy_or_unauthenticated_agents_never_receive_the_prompt() {
    for status in [
        ConnectionStatus::Running,
        ConnectionStatus::Cancelling,
        ConnectionStatus::NeedsAuthentication,
    ] {
        let fixture = Fixture::new(Some(RouteTarget::Existing(ProjectId(1))));
        fixture.agents.view.lock().unwrap().status = status.clone();
        let result = fixture
            .dialogue()
            .respond("request", None, &AtomicBool::new(false), &|_| {});
        let expected = if status == ConnectionStatus::NeedsAuthentication {
            VoiceTurnError::AgentAuthentication
        } else {
            VoiceTurnError::AgentBusy
        };
        assert!(matches!(result, Err(error) if error == expected));
        assert!(fixture.agents.sends.lock().unwrap().is_empty());
        assert!(fixture.agents.commands.lock().unwrap().is_empty());
    }
    let fixture = Fixture::new(Some(RouteTarget::Existing(ProjectId(1))));
    let revision = fixture.projects.revision();
    fixture
        .projects
        .apply(ProjectCommand::Create(ProjectDraft {
            name: "Added later".into(),
            ..Default::default()
        }))
        .unwrap();
    for selection in [
        None,
        Some(VoiceRouteSelection {
            catalog_revision: revision,
            target: RouteTarget::NewProject,
        }),
    ] {
        assert!(matches!(
            fixture
                .dialogue()
                .respond("request", selection, &AtomicBool::new(false), &|_| {}),
            Err(VoiceTurnError::CatalogChanged)
        ));
    }
    assert!(fixture.agents.sends.lock().unwrap().is_empty());
}

#[test]
fn permission_requests_require_the_user_and_cancellation_targets_only_the_owned_turn() {
    let fixture = Fixture::new(Some(RouteTarget::Existing(ProjectId(1))));
    fixture.agents.view.lock().unwrap().permissions = vec![PermissionRequest {
        id: 1,
        title: "Allow?".into(),
        detail: "Write file".into(),
        choices: vec![],
    }];
    let cancelled = AtomicBool::new(false);
    let result = fixture
        .dialogue()
        .respond("request", None, &cancelled, &|progress| {
            if matches!(
                progress,
                VoicePromptProgress::Stage(VoiceTurnStage::NeedsAttention)
            ) {
                cancelled.store(true, Ordering::Release);
            }
        });
    assert!(matches!(result, Err(VoiceTurnError::Cancelled)));
    let commands = fixture.agents.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        AgentCommand::CancelTurn { response_id: 41 }
    ));
}

#[test]
fn an_interrupted_or_missing_response_is_not_read_as_a_success() {
    for missing in [false, true] {
        let fixture = Fixture::new(Some(RouteTarget::Existing(ProjectId(1))));
        let result =
            fixture
                .dialogue()
                .respond("request", None, &AtomicBool::new(false), &|progress| {
                    if matches!(
                        progress,
                        VoicePromptProgress::Stage(VoiceTurnStage::WaitingForAgent)
                    ) {
                        let mut view = fixture.agents.view.lock().unwrap();
                        if missing {
                            view.messages.clear();
                        } else {
                            view.messages[0].status = MessageStatus::Interrupted;
                        }
                    }
                });
        assert!(matches!(result, Err(VoiceTurnError::Agent(_))));
    }
}
