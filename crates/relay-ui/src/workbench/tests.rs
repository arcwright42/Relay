use super::{Page, Text, Translate, Workbench};
use gpui_kit::{AppContext, Entity, TestAppContext};
use relay_core::{ProjectId, agents::*, projects::*, routing::*, settings::*};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct TestAgents(Mutex<Vec<AgentCommand>>);

impl AgentService for TestAgents {
    fn revision(&self) -> u64 {
        0
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        AgentSnapshot {
            status: ConnectionStatus::Running,
            messages: vec![ChatMessage {
                id: 42,
                role: MessageRole::Assistant,
                text: "Original answer 原始内容".into(),
                status: MessageStatus::Streaming,
                tools: vec![],
                metrics: None,
            }],
            ..Default::default()
        }
    }
    fn dispatch(&self, _: ProjectId, command: AgentCommand) -> Result<(), String> {
        self.0.lock().unwrap().push(command);
        Ok(())
    }
}

#[derive(Default)]
struct TestSettings(Mutex<SettingsSnapshot>);

#[derive(Default)]
struct TestRouting;
impl RoutingService for TestRouting {
    fn snapshot(&self) -> RoutingSnapshot {
        RoutingSnapshot::default()
    }
    fn save_key(&self, _: RoutingProvider, _: String) -> Result<(), RoutingError> {
        Ok(())
    }
    fn remove_key(&self) -> Result<(), RoutingError> {
        Ok(())
    }
    fn decide(&self, _: &str) -> Result<RouteDecision, RoutingError> {
        Err(RoutingError::NotConfigured)
    }
}
impl SettingsService for TestSettings {
    fn snapshot(&self) -> SettingsSnapshot {
        self.0.lock().unwrap().clone()
    }
    fn set_language(&self, language: Language) {
        self.0.lock().unwrap().language = language;
    }
}

#[gpui_kit::test]
fn switching_language_preserves_project_drafts_and_running_conversation(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let agents = Arc::new(TestAgents::default());
    let settings = Arc::new(TestSettings::default());
    let window = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            settings.clone(),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    window
        .update(cx, |view, window, cx| {
            let drafts = ["Unsent first draft", "未发送的第二份草稿", "Third draft"];
            for (draft, value) in view.drafts.iter().zip(drafts) {
                draft.update(cx, |draft, cx| draft.set_value(value, window, cx));
            }
            view.search
                .update(cx, |search, cx| search.set_value("design", window, cx));
            view.navigate(Page::Project(1), window, cx);
            view.navigate(Page::Settings, window, cx);
            let ids: Vec<_> = view.drafts.iter().map(Entity::entity_id).collect();
            for language in [Language::English, Language::SimplifiedChinese] {
                view.set_language(language, window, cx);
                assert_eq!(view.settings_snapshot.language, language);
                assert_eq!(crate::locale::current_language(cx), language);
                assert_eq!(view.text(Text::Settings), language.text(Text::Settings));
                assert!(view.page == Page::Settings);
                assert_eq!(view.selected_project, 1);
                assert_eq!(view.search.read(cx).value().as_ref(), "design");
                for (index, draft) in view.drafts.iter().enumerate() {
                    assert_eq!(draft.entity_id(), ids[index]);
                    assert_eq!(draft.read(cx).value().as_ref(), drafts[index]);
                    assert_eq!(
                        draft.read(cx).presentation().placeholder().as_ref(),
                        language.text(Text::AskRelay)
                    );
                    assert_eq!(view.projects[index].id, ProjectId(index as u64 + 1));
                    assert_eq!(
                        view.projects[index].name,
                        format!("User project {}", index + 1)
                    );
                    assert_eq!(
                        view.projects[index].instructions,
                        "Keep my instructions unchanged"
                    );
                    assert_eq!(view.agent_states[index].status, ConnectionStatus::Running);
                    assert_eq!(
                        view.agent_states[index].messages[0].text,
                        "Original answer 原始内容"
                    );
                    assert_eq!(
                        view.agent_states[index].messages[0].status,
                        MessageStatus::Streaming
                    );
                }
            }
        })
        .unwrap();
    assert!(
        agents.0.lock().unwrap().is_empty(),
        "Switching language must not dispatch or reconnect an agent"
    );
}

struct TestProjects(Mutex<ProjectCatalog>);
impl Default for TestProjects {
    fn default() -> Self {
        Self(Mutex::new(ProjectCatalog {
            revision: 1,
            error: None,
            projects: (1..=3)
                .map(|id| Project {
                    id: ProjectId(id),
                    revision: 1,
                    name: format!("User project {id}"),
                    description: String::new(),
                    instructions: "Keep my instructions unchanged".into(),
                    context: vec![],
                    memory: vec![],
                })
                .collect(),
        }))
    }
}
impl ProjectService for TestProjects {
    fn snapshot(&self) -> ProjectCatalog {
        self.0.lock().unwrap().clone()
    }
    fn apply(&self, command: ProjectCommand) -> Result<ProjectId, String> {
        let mut state = self.0.lock().unwrap();
        if let ProjectCommand::CreateAtRevision {
            expected_catalog_revision,
            ..
        } = &command
            && *expected_catalog_revision != state.revision
        {
            return Err("Catalog changed".into());
        }
        let id = match command {
            ProjectCommand::Create(draft) | ProjectCommand::CreateAtRevision { draft, .. } => {
                let id = ProjectId(state.projects.len() as u64 + 1);
                state.projects.push(Project {
                    id,
                    revision: 1,
                    name: draft.name,
                    description: draft.description,
                    instructions: draft.instructions,
                    context: vec![],
                    memory: vec![],
                });
                id
            }
            ProjectCommand::SaveContext {
                project,
                id,
                name,
                content,
                included,
                ..
            } => {
                let current = state.projects.iter_mut().find(|p| p.id == project).unwrap();
                let id = id.unwrap_or(ContextId(1));
                current.context.retain(|item| item.id != id);
                current.context.push(ContextItem {
                    id,
                    name,
                    content,
                    included,
                });
                current.revision += 1;
                project
            }
            ProjectCommand::SaveMemory {
                project,
                expected_revision,
                id,
                kind,
                name,
                content,
                source,
            } => {
                let current = state.projects.iter_mut().find(|p| p.id == project).unwrap();
                if current.revision != expected_revision {
                    return Err("Project changed".into());
                }
                if let Some(id) = id {
                    let item = current
                        .memory
                        .iter_mut()
                        .find(|item| item.id == id)
                        .unwrap();
                    item.kind = kind;
                    item.name = name;
                    item.content = content;
                } else {
                    let id = MemoryId(
                        current
                            .memory
                            .iter()
                            .map(|item| item.id.0)
                            .max()
                            .unwrap_or(0)
                            + 1,
                    );
                    current.memory.push(MemoryItem {
                        id,
                        kind,
                        name,
                        content,
                        source,
                    });
                }
                current.revision += 1;
                project
            }
            ProjectCommand::RemoveMemory {
                project,
                expected_revision,
                id,
            } => {
                let current = state.projects.iter_mut().find(|p| p.id == project).unwrap();
                if current.revision != expected_revision {
                    return Err("Project changed".into());
                }
                current.memory.retain(|item| item.id != id);
                current.revision += 1;
                project
            }
            _ => return Err("Not used".into()),
        };
        state.revision += 1;
        Ok(id)
    }
}

#[derive(Default)]
struct ReadyAgents(Mutex<Vec<(ProjectId, AgentCommand)>>);
impl AgentService for ReadyAgents {
    fn revision(&self) -> u64 {
        1
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        AgentSnapshot {
            status: ConnectionStatus::Ready,
            ..Default::default()
        }
    }
    fn dispatch(&self, id: ProjectId, command: AgentCommand) -> Result<(), String> {
        self.0.lock().unwrap().push((id, command));
        Ok(())
    }
}

struct DecidingRouter {
    decision: Result<RouteDecision, RoutingError>,
    calls: Mutex<Vec<String>>,
    key: Mutex<Option<(RoutingProvider, String)>>,
    credential_source: RoutingCredentialSource,
}
impl DecidingRouter {
    fn new(target: Option<RouteTarget>) -> Self {
        Self {
            decision: Ok(RouteDecision {
                catalog_revision: 1,
                automatic: target,
                options: vec![RouteOption {
                    target: RouteTarget::Existing(ProjectId(2)),
                    probability: 0.95,
                }],
                confidence: 0.90,
                elapsed_ms: 10,
                model: "jev-1.13.0".into(),
            }),
            calls: Mutex::default(),
            key: Mutex::default(),
            credential_source: RoutingCredentialSource::Keychain,
        }
    }
}
impl RoutingService for DecidingRouter {
    fn snapshot(&self) -> RoutingSnapshot {
        let key = self.key.lock().unwrap();
        RoutingSnapshot {
            provider: key
                .as_ref()
                .map(|(provider, _)| *provider)
                .unwrap_or_default(),
            configured: key.is_some(),
            credential_source: key.as_ref().map(|_| self.credential_source),
            error: None,
        }
    }
    fn save_key(&self, provider: RoutingProvider, key: String) -> Result<(), RoutingError> {
        *self.key.lock().unwrap() = Some((provider, key));
        Ok(())
    }
    fn remove_key(&self) -> Result<(), RoutingError> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }
    fn decide(&self, prompt: &str) -> Result<RouteDecision, RoutingError> {
        self.calls.lock().unwrap().push(prompt.into());
        self.decision.clone()
    }
}

#[gpui_kit::test]
fn home_pointer_and_keyboard_route_once_to_existing_project(cx: &mut TestAppContext) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(gpui_kit::init);
    let projects = Arc::new(TestProjects::default());
    let agents = Arc::new(ReadyAgents::default());
    let router = Arc::new(DecidingRouter::new(Some(RouteTarget::Existing(ProjectId(
        2,
    )))));
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            Workbench::new(
                agents.clone(),
                Arc::new(TestSettings::default()),
                projects.clone(),
                router.clone(),
                window,
                cx,
            )
        });
        Root::new(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("home-prompt", cx);
    })
    .unwrap();
    cx.simulate_input(window.into(), "继续设计 Relay");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("route-prompt", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(&*router.calls.lock().unwrap(), &["继续设计 Relay"]);
    let commands = agents.0.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(&commands[0], (ProjectId(2), AgentCommand::Send(text)) if text == "继续设计 Relay")
    );
    assert_eq!(projects.snapshot().projects.len(), 3);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("route-prompt").is_none());
        assert!(window.try_find("send").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn routing_creates_once_and_explicit_project_messages_bypass_jev(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let projects = Arc::new(TestProjects::default());
    let agents = Arc::new(ReadyAgents::default());
    let router = Arc::new(DecidingRouter::new(Some(RouteTarget::NewProject)));
    let window = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            projects.clone(),
            router.clone(),
            window,
            cx,
        )
    });
    window
        .update(cx, |view, window, cx| {
            view.routing.draft.update(cx, |draft, cx| {
                draft.set_value("Plan my holiday", window, cx)
            });
        })
        .unwrap();
    window
        .update(cx, |view, window, cx| {
            view.route_prompt(window, cx);
            view.route_prompt(window, cx); // double click while deciding
        })
        .unwrap();
    cx.run_until_parked();
    let catalog = projects.snapshot();
    assert_eq!(catalog.projects.len(), 4);
    assert_eq!(catalog.projects[3].name, "Plan my holiday");
    assert!(catalog.projects[3].instructions.is_empty());
    window
        .update(cx, |view, window, cx| {
            assert!(view.page == Page::Project(3));
            assert!(view.routing.draft.read(cx).value().is_empty());
            view.drafts[3].update(cx, |draft, cx| {
                draft.set_value("Make it a weekend", window, cx)
            });
            view.send_message(&super::SendMessage, window, cx);
        })
        .unwrap();
    assert_eq!(router.calls.lock().unwrap().len(), 1);
    let commands = agents.0.lock().unwrap();
    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|(id, _)| *id == ProjectId(4)));
}

#[gpui_kit::test]
fn uncertain_or_failed_routing_preserves_input_and_never_executes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for result in [
        DecidingRouter::new(None).decision,
        Err(RoutingError::Unavailable),
    ] {
        let projects = Arc::new(TestProjects::default());
        let agents = Arc::new(ReadyAgents::default());
        let router = Arc::new(DecidingRouter {
            decision: result,
            calls: Mutex::default(),
            key: Mutex::default(),
            credential_source: RoutingCredentialSource::Keychain,
        });
        let window = cx.add_window(|window, cx| {
            Workbench::new(
                agents.clone(),
                Arc::new(TestSettings::default()),
                projects.clone(),
                router,
                window,
                cx,
            )
        });
        window
            .update(cx, |view, window, cx| {
                view.routing
                    .draft
                    .update(cx, |draft, cx| draft.set_value("继续", window, cx))
            })
            .unwrap();
        window
            .update(cx, |view, window, cx| view.route_prompt(window, cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                assert!(view.page == Page::Home);
                assert_eq!(view.routing.draft.read(cx).value().as_ref(), "继续");
            })
            .unwrap();
        assert_eq!(projects.snapshot().projects.len(), 3);
        assert!(agents.0.lock().unwrap().is_empty());
    }
}

#[gpui_kit::test]
fn leaving_home_during_routing_never_hijacks_navigation_or_creates(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for return_home in [false, true] {
        let projects = Arc::new(TestProjects::default());
        let agents = Arc::new(ReadyAgents::default());
        let router = Arc::new(DecidingRouter::new(Some(RouteTarget::NewProject)));
        let window = cx.add_window(|window, cx| {
            Workbench::new(
                agents.clone(),
                Arc::new(TestSettings::default()),
                projects.clone(),
                router,
                window,
                cx,
            )
        });
        window
            .update(cx, |view, window, cx| {
                view.routing
                    .draft
                    .update(cx, |draft, cx| draft.set_value("A new task", window, cx))
            })
            .unwrap();
        window
            .update(cx, |view, window, cx| {
                view.route_prompt(window, cx);
                view.navigate(Page::Settings, window, cx);
                if return_home {
                    view.navigate(Page::Home, window, cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                assert!(
                    view.page
                        == if return_home {
                            Page::Home
                        } else {
                            Page::Settings
                        }
                );
                assert_eq!(view.routing.draft.read(cx).value().as_ref(), "A new task");
            })
            .unwrap();
        assert_eq!(projects.snapshot().projects.len(), 3);
        assert!(agents.0.lock().unwrap().is_empty());
    }
}

#[gpui_kit::test]
fn jev_settings_save_and_remove_masked_key_without_retaining_input(cx: &mut TestAppContext) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(gpui_kit::init);
    let router = Arc::new(DecidingRouter::new(None));
    let view_cell = std::cell::RefCell::new(None);
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            Workbench::new(
                Arc::new(ReadyAgents::default()),
                Arc::new(TestSettings::default()),
                Arc::new(TestProjects::default()),
                router.clone(),
                window,
                cx,
            )
        });
        view.update(cx, |view, cx| view.navigate(Page::Settings, window, cx));
        *view_cell.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("jev-api-key", cx);
    })
    .unwrap();
    cx.simulate_input(window.into(), "fake-test-key");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("save-jev-key", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(router.snapshot().configured);
    cx.update(|cx| {
        view_cell.borrow().as_ref().unwrap().update(cx, |view, cx| {
            assert!(view.routing.key.read(cx).value().is_empty());
            assert!(view.routing.key.read(cx).presentation().is_masked());
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("typesafe", cx);
        window.click("jev-api-key", cx);
    })
    .unwrap();
    assert_eq!(
        router.snapshot().provider,
        RoutingProvider::OpenRouter,
        "Selecting alone does not change the saved channel"
    );
    cx.simulate_input(window.into(), "different-provider-test-key");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("save-jev-key", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(router.snapshot().provider, RoutingProvider::TypeSafe);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("remove-jev-key", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!router.snapshot().configured);
}

#[gpui_kit::test]
fn environment_jev_credentials_cannot_be_replaced_removed_or_switched_in_settings(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(gpui_kit::init);
    for source in [
        RoutingCredentialSource::Environment,
        RoutingCredentialSource::EnvFile,
    ] {
        let router = Arc::new(DecidingRouter {
            key: Mutex::new(Some((
                RoutingProvider::OpenRouter,
                "externally-managed-key".into(),
            ))),
            credential_source: source,
            ..DecidingRouter::new(None)
        });
        let view_cell = std::cell::RefCell::new(None);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                Workbench::new(
                    Arc::new(ReadyAgents::default()),
                    Arc::new(TestSettings::default()),
                    Arc::new(TestProjects::default()),
                    router.clone(),
                    window,
                    cx,
                )
            });
            view.update(cx, |view, cx| {
                view.navigate(Page::Settings, window, cx);
                // Even a nonempty draft cannot enable saving over an external credential.
                view.routing
                    .key
                    .update(cx, |key, cx| key.set_value("replacement-key", window, cx));
            });
            *view_cell.borrow_mut() = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("typesafe", cx);
            window.click("save-jev-key", cx);
            window.click("remove-jev-key", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(router.snapshot().credential_source, Some(source));
        assert_eq!(
            *router.key.lock().unwrap(),
            Some((RoutingProvider::OpenRouter, "externally-managed-key".into()))
        );
        cx.update(|cx| {
            view_cell.borrow().as_ref().unwrap().update(cx, |view, cx| {
                assert_eq!(
                    view.routing.key.read(cx).value().as_ref(),
                    "replacement-key"
                );
                assert!(!view.routing.saving_key);
            })
        });
    }
}

#[gpui_kit::test]
fn project_and_note_forms_save_through_real_pointer_and_keyboard_events(cx: &mut TestAppContext) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(gpui_kit::init);
    let projects = Arc::new(TestProjects::default());
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            Workbench::new(
                Arc::new(TestAgents::default()),
                Arc::new(TestSettings::default()),
                projects.clone(),
                Arc::new(TestRouting),
                window,
                cx,
            )
        });
        Root::new(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("create-project", cx);
        assert!(window.try_find("save-project-edit").is_some());
        window.click("project-editor-name", cx);
    })
    .unwrap();
    cx.simulate_input(window.into(), "Caching project");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("project-editor-body", cx)
    })
    .unwrap();
    cx.simulate_input(window.into(), "Stable project instructions");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("save-project-edit", cx)
    })
    .unwrap();
    cx.run_until_parked();
    {
        let catalog = projects.snapshot();
        let created = catalog.projects.last().unwrap();
        assert_eq!(created.name, "Caching project");
        assert_eq!(created.instructions, "Stable project instructions");
    }
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("save-project-edit").is_none());
        window.click("composer-context", cx);
        window.click("add-context-note", cx);
        window.click("project-editor-name", cx);
    })
    .unwrap();
    cx.simulate_input(window.into(), "Reference note");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("project-editor-body", cx)
    })
    .unwrap();
    cx.simulate_input(window.into(), "Reference content for this project");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("note-included", cx);
        window.click("save-project-edit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    {
        let catalog = projects.snapshot();
        let note = &catalog.projects.last().unwrap().context[0];
        assert_eq!(note.name, "Reference note");
        assert_eq!(note.content, "Reference content for this project");
        assert!(!note.included);
    }
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("save-project-edit").is_none());
        assert!(window.try_find("add-context-note").is_some());
        window.click(("include-note", 1_u64), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(projects.snapshot().projects.last().unwrap().context[0].included);
}

struct ReplyAgents;
impl AgentService for ReplyAgents {
    fn revision(&self) -> u64 {
        1
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        AgentSnapshot {
            status: ConnectionStatus::Ready,
            messages: vec![
                ChatMessage {
                    id: 12,
                    role: MessageRole::Assistant,
                    text: "## Chosen design\nShare memory through ACP context".into(),
                    status: MessageStatus::Complete,
                    tools: vec![],
                    metrics: None,
                },
                ChatMessage {
                    id: 13,
                    role: MessageRole::Assistant,
                    text: "Unfinished answer".into(),
                    status: MessageStatus::Interrupted,
                    tools: vec![],
                    metrics: None,
                },
            ],
            ..Default::default()
        }
    }
    fn dispatch(&self, _: ProjectId, _: AgentCommand) -> Result<(), String> {
        panic!("Opening project context must not dispatch an agent command")
    }
}

struct LocalSessionFixture {
    state: Mutex<relay_core::sessions::ClientSessionsSnapshot>,
    revision: std::sync::atomic::AtomicU64,
}

impl LocalSessionFixture {
    fn new() -> Self {
        use relay_core::sessions::*;
        let session = ClientSession {
            id: ClientSessionId("codex:outside-relay".into()),
            client: "codex".into(),
            native_id: "outside-relay".into(),
            title: "Native client decision".into(),
            working_directory: "/workspace/native".into(),
            source: "/native/sessions/session.jsonl".into(),
            project: None,
            updated_at: "2026-09-30".into(),
            message_count: 1,
            available: true,
        };
        Self {
            state: Mutex::new(ClientSessionsSnapshot {
                sessions: vec![session],
                ..Default::default()
            }),
            revision: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl relay_core::sessions::ClientSessionsService for LocalSessionFixture {
    fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Acquire)
    }
    fn snapshot(&self) -> relay_core::sessions::ClientSessionsSnapshot {
        self.state.lock().unwrap().clone()
    }
    fn dispatch(&self, command: relay_core::sessions::ClientSessionsCommand) -> Result<(), String> {
        use relay_core::sessions::*;
        let mut state = self.state.lock().unwrap();
        match command {
            ClientSessionsCommand::Open(id) => {
                state.selected = Some(id);
                state.detail = Some(ClientSessionDetail {
                    session: state.sessions[0].clone(),
                    messages: Arc::new(vec![ClientMessage {
                        id: 99,
                        role: MessageRole::Assistant,
                        text: "## Local decision\nReuse the user's own client".into(),
                    }]),
                });
            }
            ClientSessionsCommand::Assign { project, .. } => {
                state.sessions[0].project = project;
                state.detail.as_mut().unwrap().session.project = project;
            }
            ClientSessionsCommand::Sync => {}
        }
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

#[gpui_kit::test]
fn native_session_assignment_and_copy_do_not_offer_manual_memory_saving(cx: &mut TestAppContext) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    for language in [Language::SimplifiedChinese, Language::English] {
        let projects = Arc::new(TestProjects::default());
        projects
            .apply(ProjectCommand::SaveMemory {
                project: ProjectId(2),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Decision,
                name: "Local decision".into(),
                content: "Reuse the user's own client".into(),
                source: Some(MemorySource::ClientSession {
                    client: "codex".into(),
                    session_id: "outside-relay".into(),
                    message_id: 99,
                }),
            })
            .unwrap();
        let saved = projects.snapshot();
        let agents = Arc::new(ReadyAgents::default());
        let sessions = Arc::new(LocalSessionFixture::new());
        let settings = Arc::new(TestSettings::default());
        settings.set_language(language);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = Workbench::new(
                    agents.clone(),
                    settings.clone(),
                    projects.clone(),
                    Arc::new(TestRouting),
                    window,
                    cx,
                );
                view.set_client_session_service(sessions.clone(), cx);
                view.navigate(Page::Sessions, window, cx);
                view
            });
            Root::new(view, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("client-session-codex:outside-relay", cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find(("remember-client-message", 99_u64))
                    .is_none()
            );
            window.click(("copy-client-message", 99_u64), cx);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "## Local decision\nReuse the user's own client"
            );
            window.click("assign-client-session", cx);
            window.click(("client-session-project", 2_u64), cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find(("remember-client-message", 99_u64))
                    .is_none()
            );
            assert!(window.try_find("save-project-edit").is_none());
            assert!(window.try_find(("copy-client-message", 99_u64)).is_some());
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            sessions.state.lock().unwrap().sessions[0].project,
            Some(ProjectId(2))
        );
        assert_eq!(projects.snapshot().projects, saved.projects);
        assert_eq!(projects.snapshot().revision, saved.revision);
        assert!(agents.0.lock().unwrap().is_empty());
    }
}

#[gpui_kit::test]
fn session_keeps_memory_out_of_user_actions_and_preserves_existing_entries(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    for language in [Language::SimplifiedChinese, Language::English] {
        let projects = Arc::new(TestProjects::default());
        projects
            .apply(ProjectCommand::SaveMemory {
                project: ProjectId(1),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Decision,
                name: "Chosen design".into(),
                content: "Share memory through ACP context".into(),
                source: Some(MemorySource::Message { message_id: 12 }),
            })
            .unwrap();
        let saved = projects.snapshot();
        let settings = Arc::new(TestSettings::default());
        settings.set_language(language);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                Workbench::new(
                    Arc::new(ReplyAgents),
                    settings.clone(),
                    projects.clone(),
                    Arc::new(TestRouting),
                    window,
                    cx,
                )
            });
            view.update(cx, |view, cx| view.navigate(Page::Project(0), window, cx));
            Root::new(view, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("project-memory").is_none());
            assert!(window.try_find(("remember-reply", 12_u64)).is_none());
            assert!(window.try_find(("remember-reply", 13_u64)).is_none());
            assert!(window.try_find("send").is_some());
            window.click("composer-context", cx);
            assert!(window.try_find("context-project-memory").is_none());
            assert!(window.try_find("context-project-instructions").is_some());
            window.click("add-context-note", cx);
            assert!(window.try_find("save-project-edit").is_some());
            assert!(window.try_find("note-included").is_some());
            window.click("cancel-project-edit", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(projects.snapshot().projects, saved.projects);
        assert_eq!(projects.snapshot().revision, saved.revision);
    }
}

#[gpui_kit::test]
fn new_projects_appear_without_resetting_other_drafts_and_empty_catalog_is_renderable(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let projects = Arc::new(TestProjects::default());
    let window = cx.add_window(|window, cx| {
        Workbench::new(
            Arc::new(TestAgents::default()),
            Arc::new(TestSettings::default()),
            projects.clone(),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    window
        .update(cx, |view, window, cx| {
            view.drafts[0].update(cx, |draft, cx| {
                draft.set_value("Keep this draft", window, cx)
            });
            let original = view.drafts[0].entity_id();
            {
                let mut state = projects.0.lock().unwrap();
                state.projects.push(Project {
                    id: ProjectId(8),
                    revision: 1,
                    name: "New durable project".into(),
                    description: String::new(),
                    instructions: String::new(),
                    context: vec![],
                    memory: vec![],
                });
                state.revision += 1;
            }
            view.refresh_projects(window, cx);
            assert_eq!(view.projects.len(), 4);
            assert_eq!(view.drafts.len(), 4);
            assert_eq!(view.drafts[0].entity_id(), original);
            assert_eq!(view.drafts[0].read(cx).value().as_ref(), "Keep this draft");
            view.navigate(Page::Project(3), window, cx);
            assert_eq!(view.projects[view.selected_project].id, ProjectId(8));
            {
                let mut state = projects.0.lock().unwrap();
                state.projects.clear();
                state.error = Some("Unreadable catalog".into());
                state.revision += 1;
            }
            view.refresh_projects(window, cx);
            assert!(view.page == Page::Home);
            assert!(view.projects.is_empty());
            view.navigate(Page::Agents, window, cx);
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        use gpui_kit::test::TestWindowExt;
        window.render_frame(cx);
    })
    .unwrap();
}

struct PageFetcher;
impl relay_core::capture::WebFetchService for PageFetcher {
    fn fetch(&self, url: &str) -> Result<String, String> {
        Ok(format!("page-body-for-{url}"))
    }
}

fn resize_quick_in_test(
    event: &super::ResizeQuick,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    let handle = window.window_handle();
    event.apply(handle, cx, move |size, cx| {
        handle
            .update(cx, |_, window, _| window.resize(size))
            .unwrap();
    });
}

#[gpui_kit::test]
fn quick_manage_agents_opens_workspace_settings_for_the_same_project(cx: &mut TestAppContext) {
    use gpui_kit::{px, size, test::TestWindowExt};
    use relay_core::capture::{QuickAction, Selection};
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let workspace = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        );
        view.capture(
            Selection {
                text: "selected text".into(),
                ..Default::default()
            },
            Arc::new(PageFetcher),
            window,
            cx,
        );
        view.open_project(ProjectId(2), window, cx);
        view.agent_states[1].status = ConnectionStatus::Disconnected;
        view.quick_send(QuickAction::Search, window, cx);
        view
    });
    quick
        .update(cx, |view, window, cx| {
            view._subscriptions.push(cx.subscribe(
                &cx.entity(),
                move |_, _, event: &super::OpenAgentSettings, cx| {
                    workspace
                        .update(cx, |view, window, cx| {
                            view.open_agent_settings(event.0, window, cx)
                        })
                        .unwrap();
                },
            ));
            assert!(
                !view.picker_open,
                "connection controls must not obscure the result"
            );
            window.resize(size(px(520.), px(580.)));
            window.bounds_changed(cx);
        })
        .unwrap();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("quick-connect-codex").visible());
        window.click("quick-manage-agents", cx);
    })
    .unwrap();
    workspace
        .update(cx, |view, _, _| {
            assert!(view.page == Page::Agents);
            assert_eq!(view.projects[view.selected_project].id, ProjectId(2));
        })
        .unwrap();
    assert!(matches!(
        &agents.0.lock().unwrap()[0],
        (ProjectId(2), AgentCommand::DiscoverLocal)
    ));
}

#[gpui_kit::test]
fn quick_submission_does_not_wait_for_fetch_or_overwrite_workspace_draft(cx: &mut TestAppContext) {
    use relay_core::capture::{QuickAction, Selection};
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let projects = Arc::new(TestProjects::default());
    let workspace = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            projects.clone(),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    let quick = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            projects.clone(),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    workspace
        .update(cx, |view, window, cx| {
            view.drafts[0].update(cx, |draft, cx| {
                draft.set_value("keep workspace draft", window, cx)
            });
        })
        .unwrap();
    quick
        .update(cx, |view, window, cx| {
            view.capture(
                Selection {
                    text: "selected evidence".into(),
                    url: Some("https://example.com/a".into()),
                    ..Default::default()
                },
                Arc::new(PageFetcher),
                window,
                cx,
            );
            view.quick_send(QuickAction::Search, window, cx);
            view.quick_send(QuickAction::Search, window, cx); // repeated clicks must not send twice
        })
        .unwrap();
    cx.run_until_parked();
    let commands = agents.0.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(&commands[0], (ProjectId(1), AgentCommand::Send(text)) if text.contains("selected evidence") && !text.contains("page-body-for"))
    );
    workspace
        .update(cx, |view, _, cx| {
            assert_eq!(view.drafts[0].read(cx).value(), "keep workspace draft");
        })
        .unwrap();
    quick
        .update(cx, |view, window, cx| {
            view.drafts[0].update(cx, |draft, cx| draft.set_value("follow-up", window, cx));
        })
        .unwrap();
    drop(commands);
    quick
        .update(cx, |view, window, cx| {
            view.quick_send(QuickAction::Ask, window, cx)
        })
        .unwrap();
    assert!(
        matches!(&agents.0.lock().unwrap()[1], (_, AgentCommand::Send(text)) if text == "follow-up")
    );
}

#[gpui_kit::test]
fn quick_recapture_uses_only_latest_page_and_saves_without_agent(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;
    use relay_core::capture::{QuickAction, Selection};
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let projects = Arc::new(TestProjects::default());
    let quick = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            projects.clone(),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    quick
        .update(cx, |view, window, cx| {
            for suffix in ["old", "new"] {
                view.capture(
                    Selection {
                        text: format!("{suffix} selection"),
                        url: Some(format!("https://example.com/{suffix}")),
                        ..Default::default()
                    },
                    Arc::new(PageFetcher),
                    window,
                    cx,
                );
            }
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("quick-more", cx);
        window.render_frame(cx);
        window.click("quick-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(agents.0.lock().unwrap().is_empty());
    let catalog = projects.snapshot();
    assert_eq!(catalog.projects[0].context.len(), 1);
    assert!(
        catalog.projects[0].context[0]
            .content
            .contains("new selection")
    );
    assert!(!catalog.projects[0].context[0].included);
    quick
        .update(cx, |view, window, cx| {
            view.quick_send(QuickAction::Explain, window, cx)
        })
        .unwrap();
    assert!(
        matches!(&agents.0.lock().unwrap()[0], (_, AgentCommand::Send(text)) if text.contains("page-body-for-https://example.com/new") && !text.contains("/old"))
    );
}

#[gpui_kit::test]
fn quick_busy_project_and_failed_fetch_preserve_input(cx: &mut TestAppContext) {
    use relay_core::capture::{QuickAction, Selection};
    struct FailedFetch;
    impl relay_core::capture::WebFetchService for FailedFetch {
        fn fetch(&self, _: &str) -> Result<String, String> {
            Err("timeout".into())
        }
    }
    cx.update(gpui_kit::init);
    let agents = Arc::new(TestAgents::default());
    let quick = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    quick
        .update(cx, |view, window, cx| {
            view.capture(
                Selection {
                    text: "selection".into(),
                    url: Some("https://example.com".into()),
                    ..Default::default()
                },
                Arc::new(FailedFetch),
                window,
                cx,
            );
            view.drafts[0].update(cx, |draft, cx| draft.set_value("keep question", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    quick
        .update(cx, |view, window, cx| {
            view.quick_send(QuickAction::Search, window, cx);
            assert_eq!(view.drafts[0].read(cx).value(), "keep question");
        })
        .unwrap();
    assert!(agents.0.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn quick_pasted_question_still_searches_after_fetch_failure(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;
    use relay_core::capture::Selection;
    struct FailedFetch;
    impl relay_core::capture::WebFetchService for FailedFetch {
        fn fetch(&self, _: &str) -> Result<String, String> {
            Err("unavailable".into())
        }
    }
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let quick = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    cx.simulate_window_resize(
        quick.into(),
        gpui_kit::size(gpui_kit::px(760.), gpui_kit::px(800.)),
    );
    quick
        .update(cx, |view, window, cx| {
            view.capture(
                Selection {
                    url: Some("https://example.com".into()),
                    ..Default::default()
                },
                Arc::new(FailedFetch),
                window,
                cx,
            );
            view.open_project(ProjectId(2), window, cx);
            view.drafts[1].update(cx, |draft, cx| {
                draft.set_value("pasted question", window, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("quick-action", 0_usize), cx);
    })
    .unwrap();
    let commands = agents.0.lock().unwrap();
    assert!(
        matches!(&commands[0], (ProjectId(2), AgentCommand::Send(text)) if text.contains("pasted question") && text.contains("联网搜索") && !text.contains("来源网页正文"))
    );
}

#[gpui_kit::test]
fn quick_toolbar_fits_and_result_hides_previous_conversation(cx: &mut TestAppContext) {
    use gpui_kit::{px, size, test::TestWindowExt};
    use relay_core::capture::Selection;
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        );
        view.capture(
            Selection {
                text: "actual browser selection".into(),
                ..Default::default()
            },
            Arc::new(PageFetcher),
            window,
            cx,
        );
        view.agent_states[0].messages.push(ChatMessage {
            id: 1,
            role: MessageRole::Assistant,
            text: "private earlier reply".into(),
            status: MessageStatus::Complete,
            tools: vec![],
            metrics: None,
        });
        view
    });
    quick
        .update(cx, |view, window, cx| {
            view._subscriptions.push(cx.subscribe_in(
                &cx.entity(),
                window,
                |_, _, event: &super::ResizeQuick, window, cx| {
                    resize_quick_in_test(event, window, cx)
                },
            ));
        })
        .unwrap();
    cx.simulate_window_resize(quick.into(), size(px(640.), px(54.)));
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        for id in ["quick-ask", "quick-more", "quick-close"] {
            let button = window.find(id);
            assert!(button.visible());
            assert!(button.bounds().bottom() <= px(54.));
            assert!(button.bounds().right() <= px(640.));
        }
        window.click(("quick-action", 1_usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.viewport_size(), size(px(480.), px(360.)));
        for id in [
            "quick-title",
            "quick-selection",
            "quick-response",
            "quick-footer",
        ] {
            let element = window.find(id);
            assert!(
                element.visible(),
                "{id} must render after the native resize"
            );
            assert!(element.bounds().bottom() <= px(360.), "{id} below viewport");
            assert!(
                element.bounds().right() <= px(480.),
                "{id} outside viewport"
            );
        }
        assert!(
            window.try_find("quick-copy").is_none(),
            "old replies must not appear"
        );
        assert!(window.find("quick-expand").visible());
        assert!(window.find("quick-close").bounds().right() <= px(480.));
    })
    .unwrap();
    quick
        .update(cx, |view, _, cx| {
            view.agent_states[0].messages.extend([
                ChatMessage {
                    id: 2,
                    role: MessageRole::User,
                    text: "internal context and prompt".into(),
                    status: MessageStatus::Complete,
                    tools: vec![],
                    metrics: None,
                },
                ChatMessage {
                    id: 3,
                    role: MessageRole::Assistant,
                    text: "new answer".into(),
                    status: MessageStatus::Complete,
                    tools: vec![],
                    metrics: None,
                },
            ]);
            cx.notify();
        })
        .unwrap();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("quick-copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "new answer"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn quick_translation_language_changes_real_prompt(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;
    use relay_core::capture::Selection;
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        );
        view.capture(
            Selection {
                text: "selected original".into(),
                ..Default::default()
            },
            Arc::new(PageFetcher),
            window,
            cx,
        );
        view
    });
    quick
        .update(cx, |view, window, cx| {
            view._subscriptions.push(cx.subscribe_in(
                &cx.entity(),
                window,
                |_, _, event: &super::ResizeQuick, window, cx| {
                    resize_quick_in_test(event, window, cx)
                },
            ));
        })
        .unwrap();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("quick-action", 2_usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("quick-language", cx);
        window.render_frame(cx);
        window.click(("quick-target", 1_usize), cx);
    })
    .unwrap();
    let commands = agents.0.lock().unwrap();
    assert_eq!(commands.len(), 2);
    assert!(
        matches!(&commands[1], (_, AgentCommand::Send(text)) if text.contains("English") && text.contains("selected original"))
    );
}

#[gpui_kit::test]
fn quick_empty_action_opens_and_ime_must_commit_before_send(cx: &mut TestAppContext) {
    use gpui_kit::{EntityInputHandler, test::TestWindowExt};
    use relay_core::capture::{QuickAction, Selection};
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        );
        view.capture(Selection::default(), Arc::new(PageFetcher), window, cx);
        view
    });
    cx.update_window(quick.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("quick-action", 2_usize), cx);
    })
    .unwrap();
    quick
        .update(cx, |view, window, cx| {
            assert!(agents.0.lock().unwrap().is_empty());
            view.drafts[0].update(cx, |draft, cx| {
                draft.replace_and_mark_text_in_range(None, "nihao", Some(5..5), window, cx);
            });
            view.quick_send(QuickAction::Ask, window, cx);
            assert!(
                agents.0.lock().unwrap().is_empty(),
                "IME candidates must not be sent"
            );
            view.drafts[0].update(cx, |draft, cx| {
                draft.replace_text_in_range(None, "你好", window, cx);
            });
            view.quick_send(QuickAction::Ask, window, cx);
        })
        .unwrap();
    let commands = agents.0.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(&commands[0], (_, AgentCommand::Send(text)) if text.contains("你好") && text.contains("翻译") && !text.contains("nihao"))
    );
}

#[gpui_kit::test]
fn quick_dismisses_on_focus_loss_without_closing_workspace_or_sending(cx: &mut TestAppContext) {
    use relay_core::capture::Selection;
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let workspace = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        )
    });
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestProjects::default()),
            Arc::new(TestRouting),
            window,
            cx,
        );
        view.capture(
            Selection {
                text: "selected browser text".into(),
                ..Default::default()
            },
            Arc::new(PageFetcher),
            window,
            cx,
        );
        view
    });
    quick
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert!(
        quick.update(cx, |_, _, _| ()).is_ok(),
        "focus within quick panel must keep it open"
    );
    workspace
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert!(
        quick.update(cx, |_, _, _| ()).is_err(),
        "clicking another Relay window must dismiss quick panel"
    );
    assert!(workspace.update(cx, |_, _, _| ()).is_ok());
    assert!(agents.0.lock().unwrap().is_empty());
}
