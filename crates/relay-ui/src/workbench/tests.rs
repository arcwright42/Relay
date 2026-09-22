use super::{Page, Text, Translate, Workbench};
use gpui_kit::{Entity, TestAppContext};
use relay_core::{ProjectId, agents::*, settings::*};
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
    let window =
        cx.add_window(|window, cx| Workbench::new(agents.clone(), settings.clone(), window, cx));
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
