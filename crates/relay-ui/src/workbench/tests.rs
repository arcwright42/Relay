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
        RoutingProvider::Vercel,
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
