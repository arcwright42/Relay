use super::{Page, Text, Translate, Workbench};
use gpui_kit::{AppContext, Entity, TestAppContext};
use relay_core::{ThreadId, agents::*, settings::*, threads::*};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct TestAgents(Mutex<Vec<AgentCommand>>);

#[gpui_kit::test]
fn files_is_a_standalone_tab_and_preserves_the_conversation_draft(cx: &mut TestAppContext) {
    use gpui_kit::{component::Root, test::TestWindowExt};
    cx.update(gpui_kit::init);
    let mut workbench = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            Workbench::new(
                Arc::new(TestAgents::default()),
                Arc::new(TestSettings::default()),
                Arc::new(TestThreads::default()),
                window,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            view.navigate(Page::Thread(0), window, cx);
            view.drafts[0].update(cx, |draft, cx| draft.set_value("继续这个项目", window, cx));
        });
        workbench = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("files-tab", cx);
    })
    .unwrap();
    let view = workbench.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("files-workspace").is_some());
        assert!(window.try_find("send").is_none());
        view.update(cx, |view, cx| {
            assert!(view.page == Page::Files);
            assert_eq!(view.drafts[0].read(cx).value().as_ref(), "继续这个项目");
            assert_eq!(
                view.agent_states[0].messages[0].text,
                "Original answer 原始内容"
            );
            view.navigate(Page::Thread(0), window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("files-workspace").is_none());
        assert!(window.try_find("stop-response").is_some());
    })
    .unwrap();
}

impl AgentService for TestAgents {
    fn revision(&self) -> u64 {
        0
    }
    fn snapshot(&self, _: ThreadId) -> AgentSnapshot {
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
    fn dispatch(&self, _: ThreadId, command: AgentCommand) -> Result<(), String> {
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
    fn set_voice_wake_enabled(&self, enabled: bool) {
        self.0.lock().unwrap().voice_wake_enabled = enabled;
    }
}

struct TestVoiceWake {
    state: Mutex<relay_core::voice::VoiceSnapshot>,
    settings: Arc<TestSettings>,
    retries: std::sync::atomic::AtomicUsize,
}
impl relay_core::voice::VoiceService for TestVoiceWake {
    fn snapshot(&self) -> relay_core::voice::VoiceSnapshot {
        self.state.lock().unwrap().clone()
    }
    fn set_enabled(&self, enabled: bool) {
        let mut state = self.state.lock().unwrap();
        state.enabled = enabled;
        state.status = if enabled {
            relay_core::voice::VoiceStatus::Listening
        } else {
            relay_core::voice::VoiceStatus::Off
        };
        self.settings.set_voice_wake_enabled(enabled);
    }
    fn retry(&self) {
        self.retries
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.state.lock().unwrap().status = relay_core::voice::VoiceStatus::Listening;
    }
    fn resume_listening(&self, _: u64) {}
    fn start_session(&self) {
        use relay_core::voice::*;
        let mut state = self.state.lock().unwrap();
        state.status = VoiceStatus::Capturing;
        state.session = Some(VoiceSessionSnapshot {
            id: 1,
            transcription: TranscriptionStatus::NotConfigured,
            transcript: String::new(),
            transcript_truncated: false,
            captured_segments: 0,
            pending_segments: 0,
            turn: None,
        });
    }
    fn end_session(&self, _: u64) {
        let mut state = self.state.lock().unwrap();
        state.status = relay_core::voice::VoiceStatus::Listening;
        state.session = None;
    }
}

#[gpui_kit::test]
fn voice_settings_toggle_retry_and_permission_link_preserve_drafts(cx: &mut TestAppContext) {
    use gpui_kit::{component::Root, px, size, test::TestWindowExt};
    use relay_core::voice::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    cx.update(gpui_kit::init);
    for language in [Language::SimplifiedChinese, Language::English] {
        let agents = Arc::new(TestAgents::default());
        let settings = Arc::new(TestSettings::default());
        settings.set_language(language);
        let voice = Arc::new(TestVoiceWake {
            state: Mutex::new(VoiceSnapshot::default()),
            settings: settings.clone(),
            retries: AtomicUsize::new(0),
        });
        let permission_links = Arc::new(AtomicUsize::new(0));
        let view_cell = std::cell::RefCell::new(None);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                Workbench::new(
                    agents.clone(),
                    settings.clone(),
                    Arc::new(TestThreads::default()),
                    window,
                    cx,
                )
            });
            view.update(cx, |view, cx| {
                window.resize(size(px(1_100.), px(1_300.)));
                window.bounds_changed(cx);
                view.drafts[0].update(cx, |draft, cx| draft.set_value("Keep my draft", window, cx));
                view.set_voice_service(voice.clone(), cx);
                view.navigate(Page::Settings, window, cx);
                let links = permission_links.clone();
                view._subscriptions.push(cx.subscribe(
                    &cx.entity(),
                    move |_, _, _: &super::OpenMicrophoneSettings, _| {
                        links.fetch_add(1, Ordering::Relaxed);
                    },
                ));
            });
            *view_cell.borrow_mut() = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.find("voice-wake-toggle").visible(),
                "toggle bounds: {:?}, viewport: {:?}",
                window.find("voice-wake-toggle").bounds(),
                window.viewport_size()
            );
            window.click("voice-wake-toggle", cx);
        })
        .unwrap();
        assert!(settings.snapshot().voice_wake_enabled);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("start-voice-session").visible());
            window.click("start-voice-session", cx);
        })
        .unwrap();
        assert!(voice.snapshot().session.is_some());
        voice.end_session(1);
        voice.state.lock().unwrap().status = VoiceStatus::Failed(VoiceError::new(
            VoiceErrorKind::MicrophoneDenied,
            "native diagnostic",
        ));
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("microphone-settings").visible());
            window.click("microphone-settings", cx);
            window.click("retry-voice-wake", cx);
            window.render_frame(cx);
            window.click("voice-wake-toggle", cx);
        })
        .unwrap();
        assert_eq!(permission_links.load(Ordering::Relaxed), 1);
        assert_eq!(voice.retries.load(Ordering::Relaxed), 1);
        assert!(!settings.snapshot().voice_wake_enabled);
        cx.update(|cx| {
            let cell = view_cell.borrow();
            let view = cell.as_ref().unwrap().read(cx);
            assert_eq!(view.drafts[0].read(cx).value().as_ref(), "Keep my draft");
            assert_eq!(view.settings_snapshot.language, language);
            assert_eq!(view.voice_snapshot.status, VoiceStatus::Off);
        });
        assert!(agents.0.lock().unwrap().is_empty());
    }
}

#[gpui_kit::test]
fn switching_language_preserves_thread_drafts_and_running_conversation(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let agents = Arc::new(TestAgents::default());
    let settings = Arc::new(TestSettings::default());
    let window = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            settings.clone(),
            Arc::new(TestThreads::default()),
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
            view.navigate(Page::Thread(1), window, cx);
            view.navigate(Page::Settings, window, cx);
            let ids: Vec<_> = view.drafts.iter().map(Entity::entity_id).collect();
            for language in [Language::English, Language::SimplifiedChinese] {
                view.set_language(language, window, cx);
                assert_eq!(view.settings_snapshot.language, language);
                assert_eq!(crate::locale::current_language(cx), language);
                assert_eq!(view.text(Text::Settings), language.text(Text::Settings));
                assert!(view.page == Page::Settings);
                assert_eq!(view.selected_thread, 1);
                assert_eq!(view.search.read(cx).value().as_ref(), "design");
                for (index, draft) in view.drafts.iter().enumerate() {
                    assert_eq!(draft.entity_id(), ids[index]);
                    assert_eq!(draft.read(cx).value().as_ref(), drafts[index]);
                    assert_eq!(
                        draft.read(cx).presentation().placeholder().as_ref(),
                        language.text(Text::AskRelay)
                    );
                    assert_eq!(view.threads[index].id, ThreadId(index as u64 + 1));
                    assert_eq!(
                        view.threads[index].name,
                        format!("User thread {}", index + 1)
                    );
                    assert_eq!(
                        view.threads[index].instructions,
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

struct TestThreads(Mutex<ThreadCatalog>);
impl Default for TestThreads {
    fn default() -> Self {
        Self(Mutex::new(ThreadCatalog {
            revision: 1,
            error: None,
            threads: (1..=3)
                .map(|id| Thread {
                    id: ThreadId(id),
                    revision: 1,
                    name: format!("User thread {id}"),
                    description: String::new(),
                    instructions: "Keep my instructions unchanged".into(),
                    context: vec![],
                    memory: vec![],
                })
                .collect(),
        }))
    }
}
impl ThreadService for TestThreads {
    fn snapshot(&self) -> ThreadCatalog {
        self.0.lock().unwrap().clone()
    }
    fn apply(&self, command: ThreadCommand) -> Result<ThreadId, String> {
        let mut state = self.0.lock().unwrap();
        if let ThreadCommand::CreateAtRevision {
            expected_catalog_revision,
            ..
        } = &command
            && *expected_catalog_revision != state.revision
        {
            return Err("Catalog changed".into());
        }
        let id = match command {
            ThreadCommand::Create(draft) | ThreadCommand::CreateAtRevision { draft, .. } => {
                let id = ThreadId(state.threads.len() as u64 + 1);
                state.threads.push(Thread {
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
            ThreadCommand::SaveContext {
                thread,
                id,
                name,
                content,
                included,
                ..
            } => {
                let current = state.threads.iter_mut().find(|p| p.id == thread).unwrap();
                let id = id.unwrap_or(ContextId(1));
                current.context.retain(|item| item.id != id);
                current.context.push(ContextItem {
                    id,
                    name,
                    content,
                    included,
                });
                current.revision += 1;
                thread
            }
            ThreadCommand::SaveMemory {
                thread,
                expected_revision,
                id,
                kind,
                name,
                content,
                source,
            } => {
                let current = state.threads.iter_mut().find(|p| p.id == thread).unwrap();
                if current.revision != expected_revision {
                    return Err("Thread changed".into());
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
                thread
            }
            ThreadCommand::RemoveMemory {
                thread,
                expected_revision,
                id,
            } => {
                let current = state.threads.iter_mut().find(|p| p.id == thread).unwrap();
                if current.revision != expected_revision {
                    return Err("Thread changed".into());
                }
                current.memory.retain(|item| item.id != id);
                current.revision += 1;
                thread
            }
            _ => return Err("Not used".into()),
        };
        state.revision += 1;
        Ok(id)
    }
}

#[derive(Default)]
struct ReadyAgents(Mutex<Vec<(ThreadId, AgentCommand)>>);

#[gpui_kit::test]
fn main_conversation_keeps_its_identity_across_topics_and_explicit_task_navigation(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let threads = Arc::new(TestThreads::default());
    threads.0.lock().unwrap().threads[0].id = ThreadId(0);
    let agents = Arc::new(ReadyAgents::default());
    let window = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            threads.clone(),
            window,
            cx,
        )
    });
    window
        .update(cx, |view, window, cx| {
            for prompt in ["帮我研究 Relay 的记忆", "换个话题，准备旅行计划"] {
                view.drafts[0].update(cx, |draft, cx| draft.set_value(prompt, window, cx));
                view.send_message(&super::SendMessage, window, cx);
                assert!(view.page == Page::Home);
            }
            view.navigate(Page::Thread(1), window, cx);
            view.drafts[1].update(cx, |draft, cx| draft.set_value("继续这个任务", window, cx));
            view.send_message(&super::SendMessage, window, cx);
            view.navigate(Page::Home, window, cx);
            assert_eq!(view.selected_thread, 0);
        })
        .unwrap();
    let commands = agents.0.lock().unwrap();
    assert_eq!(
        commands.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![ThreadId(0), ThreadId(0), ThreadId(2)]
    );
    assert_eq!(threads.snapshot().threads.len(), 3);
}
impl AgentService for ReadyAgents {
    fn revision(&self) -> u64 {
        1
    }
    fn snapshot(&self, _: ThreadId) -> AgentSnapshot {
        AgentSnapshot {
            status: ConnectionStatus::Ready,
            ..Default::default()
        }
    }
    fn dispatch(&self, id: ThreadId, command: AgentCommand) -> Result<(), String> {
        self.0.lock().unwrap().push((id, command));
        Ok(())
    }
}

struct ReplyAgents;
impl AgentService for ReplyAgents {
    fn revision(&self) -> u64 {
        1
    }
    fn snapshot(&self, _: ThreadId) -> AgentSnapshot {
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
    fn dispatch(&self, _: ThreadId, _: AgentCommand) -> Result<(), String> {
        panic!("Opening thread context must not dispatch an agent command")
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
            thread: None,
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
            ClientSessionsCommand::Assign { thread, .. } => {
                state.sessions[0].thread = thread;
                state.detail.as_mut().unwrap().session.thread = thread;
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
        let threads = Arc::new(TestThreads::default());
        threads
            .apply(ThreadCommand::SaveMemory {
                thread: ThreadId(2),
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
        let saved = threads.snapshot();
        let agents = Arc::new(ReadyAgents::default());
        let sessions = Arc::new(LocalSessionFixture::new());
        let settings = Arc::new(TestSettings::default());
        settings.set_language(language);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = Workbench::new(
                    agents.clone(),
                    settings.clone(),
                    threads.clone(),
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
            window.click(("client-session-thread", 2_u64), cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find(("remember-client-message", 99_u64))
                    .is_none()
            );
            assert!(window.try_find("save-thread-edit").is_none());
            assert!(window.try_find(("copy-client-message", 99_u64)).is_some());
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            sessions.state.lock().unwrap().sessions[0].thread,
            Some(ThreadId(2))
        );
        assert_eq!(threads.snapshot().threads, saved.threads);
        assert_eq!(threads.snapshot().revision, saved.revision);
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
        let threads = Arc::new(TestThreads::default());
        threads
            .apply(ThreadCommand::SaveMemory {
                thread: ThreadId(1),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Decision,
                name: "Chosen design".into(),
                content: "Share memory through ACP context".into(),
                source: Some(MemorySource::Message { message_id: 12 }),
            })
            .unwrap();
        let saved = threads.snapshot();
        let settings = Arc::new(TestSettings::default());
        settings.set_language(language);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                Workbench::new(
                    Arc::new(ReplyAgents),
                    settings.clone(),
                    threads.clone(),
                    window,
                    cx,
                )
            });
            view.update(cx, |view, cx| view.navigate(Page::Thread(0), window, cx));
            Root::new(view, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("thread-memory").is_none());
            assert!(window.try_find(("remember-reply", 12_u64)).is_none());
            assert!(window.try_find(("remember-reply", 13_u64)).is_none());
            assert!(window.try_find("send").is_some());
            window.click("composer-context", cx);
            assert!(window.try_find("context-thread-memory").is_none());
            assert!(window.try_find("context-thread-instructions").is_some());
            window.click("add-context-note", cx);
            assert!(window.try_find("save-thread-edit").is_some());
            assert!(window.try_find("note-included").is_some());
            window.click("cancel-thread-edit", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(threads.snapshot().threads, saved.threads);
        assert_eq!(threads.snapshot().revision, saved.revision);
    }
}

#[gpui_kit::test]
fn new_threads_appear_without_resetting_other_drafts_and_empty_catalog_is_renderable(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let threads = Arc::new(TestThreads::default());
    let window = cx.add_window(|window, cx| {
        Workbench::new(
            Arc::new(TestAgents::default()),
            Arc::new(TestSettings::default()),
            threads.clone(),
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
                let mut state = threads.0.lock().unwrap();
                state.threads.push(Thread {
                    id: ThreadId(8),
                    revision: 1,
                    name: "New durable thread".into(),
                    description: String::new(),
                    instructions: String::new(),
                    context: vec![],
                    memory: vec![],
                });
                state.revision += 1;
            }
            view.refresh_threads(window, cx);
            assert_eq!(view.threads.len(), 4);
            assert_eq!(view.drafts.len(), 4);
            assert_eq!(view.drafts[0].entity_id(), original);
            assert_eq!(view.drafts[0].read(cx).value().as_ref(), "Keep this draft");
            view.navigate(Page::Thread(3), window, cx);
            assert_eq!(view.threads[view.selected_thread].id, ThreadId(8));
            {
                let mut state = threads.0.lock().unwrap();
                state.threads.clear();
                state.error = Some("Unreadable catalog".into());
                state.revision += 1;
            }
            view.refresh_threads(window, cx);
            assert!(view.page == Page::Home);
            assert!(view.threads.is_empty());
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
fn quick_manage_agents_opens_workspace_settings_for_the_same_thread(cx: &mut TestAppContext) {
    use gpui_kit::{px, size, test::TestWindowExt};
    use relay_core::capture::{QuickAction, Selection};
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let workspace = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestThreads::default()),
            window,
            cx,
        )
    });
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestThreads::default()),
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
        view.open_thread(ThreadId(2), window, cx);
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
            assert_eq!(view.threads[view.selected_thread].id, ThreadId(2));
        })
        .unwrap();
    assert!(matches!(
        &agents.0.lock().unwrap()[0],
        (ThreadId(2), AgentCommand::DiscoverLocal)
    ));
}

#[gpui_kit::test]
fn quick_submission_does_not_wait_for_fetch_or_overwrite_workspace_draft(cx: &mut TestAppContext) {
    use relay_core::capture::{QuickAction, Selection};
    cx.update(gpui_kit::init);
    let agents = Arc::new(ReadyAgents::default());
    let threads = Arc::new(TestThreads::default());
    let workspace = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            threads.clone(),
            window,
            cx,
        )
    });
    let quick = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            threads.clone(),
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
        matches!(&commands[0], (ThreadId(1), AgentCommand::Send(text)) if text.contains("selected evidence") && !text.contains("page-body-for"))
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
    let threads = Arc::new(TestThreads::default());
    let quick = cx.add_window(|window, cx| {
        Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            threads.clone(),
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
    let catalog = threads.snapshot();
    assert_eq!(catalog.threads[0].context.len(), 1);
    assert!(
        catalog.threads[0].context[0]
            .content
            .contains("new selection")
    );
    assert!(!catalog.threads[0].context[0].included);
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
fn quick_busy_thread_and_failed_fetch_preserve_input(cx: &mut TestAppContext) {
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
            Arc::new(TestThreads::default()),
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
            Arc::new(TestThreads::default()),
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
            view.open_thread(ThreadId(2), window, cx);
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
        matches!(&commands[0], (ThreadId(2), AgentCommand::Send(text)) if text.contains("pasted question") && text.contains("联网搜索") && !text.contains("来源网页正文"))
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
            Arc::new(TestThreads::default()),
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
            Arc::new(TestThreads::default()),
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
            Arc::new(TestThreads::default()),
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
            Arc::new(TestThreads::default()),
            window,
            cx,
        )
    });
    let quick = cx.add_window(|window, cx| {
        let mut view = Workbench::new(
            agents.clone(),
            Arc::new(TestSettings::default()),
            Arc::new(TestThreads::default()),
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
