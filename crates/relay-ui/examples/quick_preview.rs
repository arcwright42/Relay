//! Render real views with fixtures through Metal, without desktop automation
//! or a live agent. Add `--workspace` after the output folder for main app pages.
use gpui_kit::{
    assets::AllAssets,
    component::{Root, Theme, ThemeMode},
    test::TestWindowExt,
    *,
};
use relay_core::{Project, ProjectId, agents::*, capture::*, projects::*, routing::*, settings::*};
use relay_ui::{ResizeQuick, Workbench};
use std::sync::{Arc, Mutex};

struct SessionFixtures;
impl relay_core::sessions::ClientSessionsService for SessionFixtures {
    fn revision(&self) -> u64 {
        1
    }
    fn snapshot(&self) -> relay_core::sessions::ClientSessionsSnapshot {
        use relay_core::sessions::*;
        let session = ClientSession {
            id: ClientSessionId("codex:local-design-session".into()),
            client: "codex".into(),
            native_id: "local-design-session".into(),
            title: "统一本地会话与薄记忆的技术方案".into(),
            working_directory: "/Users/example/Projects/Relay".into(),
            source: "/Users/example/.codex/sessions/2026/09/30/rollout.jsonl".into(),
            project: Some(ProjectId(1)),
            updated_at: "2026-09-30T00:00:00Z".into(),
            message_count: 2,
            available: true,
        };
        let mut external = session.clone();
        external.id = ClientSessionId("codex:external-session".into());
        external.native_id = "external-session".into();
        external.title = "在本地 Client 中讨论新项目".into();
        external.project = None;
        ClientSessionsSnapshot {
            sessions: vec![session.clone(), external], selected: Some(session.id.clone()), updated_sessions: 2, scanned_files: 2,
            detail: Some(ClientSessionDetail { session, messages: Arc::new(vec![
                ClientMessage { id: 1, role: MessageRole::User, text: "Relay 不托管 Harness，记忆先保持轻量，外部会话也需要同步。".into() },
                ClientMessage { id: 2, role: MessageRole::Assistant, text: "使用用户本地 Client，复用原生登录和模型配置。\n\n本地会话按工作目录匹配项目，每 30 分钟同步可见文字。项目记忆由主 Agent 整理和维护。".into() },
            ]) }), ..Default::default()
        }
    }
    fn dispatch(&self, _: relay_core::sessions::ClientSessionsCommand) -> Result<(), String> {
        Ok(())
    }
}

struct FixtureAgent(Mutex<AgentSnapshot>, bool);
impl AgentService for FixtureAgent {
    fn revision(&self) -> u64 {
        self.0.lock().unwrap().messages.len() as u64
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        self.0.lock().unwrap().clone()
    }
    fn dispatch(&self, _: ProjectId, command: AgentCommand) -> Result<(), String> {
        if let AgentCommand::Send(prompt) = command {
            let mut state = self.0.lock().unwrap();
            state.messages.extend([
                ChatMessage { id: 1, role: MessageRole::User, text: prompt,
                    status: MessageStatus::Complete, tools: vec![], metrics: None },
                ChatMessage { id: 2, role: MessageRole::Assistant,
                    text: "**GPUI Kit** 是一套 Rust 桌面应用框架。\n\n它提供界面组件、数据管理、布局和文本编辑能力，帮助开发者构建完整的桌面应用。\n\n- **界面**：可直接使用的组件与样式\n- **交互**：输入、选择与编辑\n- **集成**：在同一框架内组织数据和应用逻辑".into(),
                    status: MessageStatus::Complete, tools: vec![], metrics: None },
            ]);
            if self.1 {
                state.status = ConnectionStatus::Running;
                let reply = state.messages.last_mut().unwrap();
                reply.text.clear();
                reply.status = MessageStatus::Streaming;
                reply.tools.push(ToolActivity {
                    id: "search".into(),
                    title: "Web search: GPUI Kit".into(),
                    status: "in_progress".into(),
                });
            }
        }
        Ok(())
    }
}
struct Fixtures;
struct FileFixtures;
impl relay_core::files::FileService for FileFixtures {
    fn list(
        &self,
        directory: std::path::PathBuf,
    ) -> Result<relay_core::files::FileListing, relay_core::files::FileError> {
        use relay_core::files::*;
        Ok(FileListing {
            workspace: "/Users/example/Relay/Personal".into(),
            directory: directory.clone(),
            truncated: false,
            entries: [
                ("资料", FileKind::Directory, 0),
                ("项目方案.md", FileKind::Markdown, 842),
                ("agent-report.md", FileKind::Markdown, 1250),
                ("relay.png", FileKind::Image, 125800),
            ]
            .into_iter()
            .map(|(name, kind, bytes)| FileEntry {
                path: directory.join(name),
                name: name.into(),
                kind,
                stamp: FileStamp {
                    bytes,
                    modified: None,
                },
            })
            .collect(),
        })
    }
    fn read(
        &self,
        location: relay_core::files::FileLocation,
    ) -> Result<relay_core::files::FileDocument, relay_core::files::FileError> {
        use relay_core::files::*;
        let content = if location.path.extension().is_some_and(|ext| ext == "png") {
            FileContent::Image {
                bytes: include_bytes!("../../../assets/relay-icon.png")
                    .as_slice()
                    .into(),
                extension: "png".into(),
            }
        } else {
            FileContent::Text { markdown: true, text: "# 我的项目方案\n\n把资料、想法和智能体生成的文件放在同一个工作空间，让成果随时可以找到和继续编辑。\n\n## 本周安排\n\n- 整理已有资料，建立清晰的文件夹\n- 与智能体讨论方案，生成第一份报告\n- 编辑结果，补充自己的判断\n\n## 项目记录\n\n| 文件 | 用途 |\n| --- | --- |\n| 项目方案.md | 持续完善工作计划 |\n| agent-report.md | 智能体生成的分析成果 |\n\n> 文件保存在个人空间，切换对话后仍然可以回来阅读。\n".into() }
        };
        Ok(FileDocument {
            location,
            stamp: FileStamp {
                bytes: 842,
                modified: None,
            },
            version: "fixture-v1".into(),
            content,
        })
    }
    fn apply(
        &self,
        _: relay_core::files::FileCommand,
    ) -> Result<relay_core::files::FileLocation, relay_core::files::FileError> {
        Err(relay_core::files::FileError::Unavailable)
    }
    fn resolve_link(
        &self,
        link: &str,
    ) -> Result<relay_core::files::FileLocation, relay_core::files::FileError> {
        Ok(relay_core::files::FileLocation {
            workspace: "/Users/example/Relay/Personal".into(),
            path: link.into(),
        })
    }
}
impl SettingsService for Fixtures {
    fn snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot::default()
    }
    fn set_language(&self, _: Language) {}
}
impl ProjectService for Fixtures {
    fn snapshot(&self) -> ProjectCatalog {
        ProjectCatalog {
            revision: 1,
            error: None,
            projects: ["Product Design", "Agent Infra", "Personal"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| Project {
                    id: ProjectId(index as u64 + 1),
                    revision: 1,
                    name: name.into(),
                    description: String::new(),
                    instructions: String::new(),
                    context: vec![],
                    memory: vec![],
                })
                .collect(),
        }
    }
    fn apply(&self, _: ProjectCommand) -> Result<ProjectId, String> {
        Err("fixture".into())
    }
}
impl RoutingService for Fixtures {
    fn snapshot(&self) -> RoutingSnapshot {
        RoutingSnapshot::default()
    }
    fn save_key(&self, _: RoutingProvider, _: String) -> Result<(), RoutingError> {
        Err(RoutingError::NotConfigured)
    }
    fn remove_key(&self) -> Result<(), RoutingError> {
        Ok(())
    }
    fn decide(&self, _: &str) -> Result<RouteDecision, RoutingError> {
        Err(RoutingError::NotConfigured)
    }
}
impl WebFetchService for Fixtures {
    fn fetch(&self, _: &str) -> Result<String, String> {
        Err("fixture".into())
    }
}

fn main() -> gpui_kit::Result<()> {
    let folder = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/relay-quick-preview".into()),
    );
    std::fs::create_dir_all(&folder)?;
    if std::env::args().any(|arg| arg == "--workspace") {
        return render_workspace(&folder);
    }
    for (name, status, action) in [
        ("explain", ConnectionStatus::Ready, 1_usize),
        ("translate", ConnectionStatus::Ready, 2),
        ("search", ConnectionStatus::Ready, 0),
        ("working", ConnectionStatus::Ready, 0),
        ("disconnected", ConnectionStatus::Disconnected, 0),
        ("error", ConnectionStatus::Failed, 0),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(AllAssets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
        });
        let error = (status == ConnectionStatus::Failed)
            .then(|| "示例：无法读取项目对话文件，请检查日志中的具体原因。".into());
        let agents = Arc::new(FixtureAgent(
            Mutex::new(AgentSnapshot {
                status,
                error,
                ..Default::default()
            }),
            name == "working",
        ));
        let handle = cx.open_window(size(px(640.),px(54.)), |window,cx| {
            let view = cx.new(|cx| {
                let mut view = Workbench::new(agents, Arc::new(Fixtures), Arc::new(Fixtures), Arc::new(Fixtures), window,cx);
                view.capture(Selection { text:"GPUI Kit is a comprehensive Rust desktop application framework. It combines a production-ready UI system with application-grade data, layout, and editing capabilities.".into(), ..Default::default() }, Arc::new(Fixtures),window,cx);
                view
            });
            let handle = window.window_handle();
            cx.subscribe(&view, move |_,event:&ResizeQuick,cx| {
                event.apply(handle,cx,move |size,cx| {
                    handle.update(cx,|_,window,_| window.resize(size)).unwrap();
                });
            }).detach();
            cx.new(|cx| Root::new(view,window,cx).bordered(false).bg(rgba(0x00000000)))
        })?;
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("quick-action", action), cx);
        })?;
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find(if name == "working" {
                        "quick-stop"
                    } else {
                        "quick-ask"
                    })
                    .visible()
            );
            assert!(window.find("quick-close").visible());
            if name == "disconnected" || name == "error" {
                assert!(window.find("quick-connect-codex").visible());
            } else if name != "working" {
                assert!(window.try_find("quick-copy").is_some());
            }
        })?;
        cx.capture_screenshot(handle.into())?
            .save(folder.join(format!("{name}.png")))?;
        if name == "working" {
            cx.update_window(handle.into(), |_, window, cx| {
                window.click("quick-options", cx);
                window.render_frame(cx);
                assert!(window.find("agent-picker").visible());
                assert!(window.find("quick-options-settings").visible());
            })?;
            cx.capture_screenshot(handle.into())?
                .save(folder.join("options.png"))?;
        }
    }
    println!("Rendered previews: {}", folder.display());
    Ok(())
}

fn render_workspace(folder: &std::path::Path) -> gpui_kit::Result<()> {
    for (name, width, height) in [("wide", 1440., 900.), ("compact", 960., 640.)] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(AllAssets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
        });
        let agents = Arc::new(FixtureAgent(
            Mutex::new(AgentSnapshot {
                status: ConnectionStatus::Ready,
                ..Default::default()
            }),
            false,
        ));
        let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = Workbench::new(
                    agents,
                    Arc::new(Fixtures),
                    Arc::new(Fixtures),
                    Arc::new(Fixtures),
                    window,
                    cx,
                );
                view.set_client_session_service(Arc::new(SessionFixtures), cx);
                view.set_file_service(Arc::new(FileFixtures), cx);
                view
            });
            cx.new(|cx| Root::new(view, window, cx).bordered(false))
        })?;
        for (page, target) in [
            ("home", ElementId::from("home")),
            ("project", ElementId::from(("project", 0_usize))),
            ("agents", ElementId::from("agents")),
            ("client-sessions", ElementId::from("client-sessions")),
            ("files", ElementId::from("files-tab")),
            ("settings", ElementId::from("settings")),
        ] {
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click(target, cx);
            })?;
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("home").visible());
                assert!(window.find("settings").visible());
                if page == "home" {
                    assert!(window.find("route-prompt").visible());
                    assert!(window.find(("home-project", 2_usize)).visible());
                    assert!(window.try_find("project-members").is_none());
                } else if page == "project" {
                    assert!(window.find("send").visible());
                    assert!(window.find("composer-context").visible());
                }
            })?;
            cx.capture_screenshot(handle.into())?
                .save(folder.join(format!("{page}-{name}.png")))?;
            if page == "files" {
                for (scene, target) in [
                    ("markdown", "files-entry-项目方案.md"),
                    ("editor", "file-toggle-preview"),
                    ("image", "files-entry-relay.png"),
                ] {
                    if scene == "image" {
                        cx.update_window(handle.into(), |_, window, cx| {
                            window.render_frame(cx);
                            window.click("file-back", cx);
                        })?;
                        cx.run_until_parked();
                    }
                    cx.update_window(handle.into(), |_, window, cx| {
                        window.render_frame(cx);
                        window.click(target, cx);
                    })?;
                    cx.run_until_parked();
                    cx.capture_screenshot(handle.into())?
                        .save(folder.join(format!("files-{scene}-{name}.png")))?;
                }
            }
        }
    }
    println!("Rendered workspace previews: {}", folder.display());
    Ok(())
}
