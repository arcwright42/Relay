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
                Workbench::new(
                    agents,
                    Arc::new(Fixtures),
                    Arc::new(Fixtures),
                    Arc::new(Fixtures),
                    window,
                    cx,
                )
            });
            cx.new(|cx| Root::new(view, window, cx).bordered(false))
        })?;
        for (page, target) in [
            ("home", ElementId::from("home")),
            ("project", ElementId::from(("project", 0_usize))),
            ("agents", ElementId::from("agents")),
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
        }
    }
    println!("Rendered workspace previews: {}", folder.display());
    Ok(())
}
