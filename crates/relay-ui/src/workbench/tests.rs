use super::{Page, Text, Translate, Workbench};
use gpui_kit::{AppContext, Entity, TestAppContext};
use relay_core::{ProjectId, agents::*, projects::*, settings::*};
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
        let id = match command {
            ProjectCommand::Create(draft) => {
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
