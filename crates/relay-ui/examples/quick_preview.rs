//! Render real quick-entry views with fixture replies through Metal, without
//! desktop automation or a live agent. PNGs are written to the supplied folder.
use gpui_kit::{
    assets::Assets,
    component::{Root, Theme, ThemeMode},
    test::TestWindowExt,
    *,
};
use relay_core::{Project, ProjectId, agents::*, capture::*, projects::*, routing::*, settings::*};
use relay_ui::{ResizeQuick, Workbench};
use std::sync::{Arc, Mutex};

struct FixtureAgent(Mutex<AgentSnapshot>);
impl AgentService for FixtureAgent {
    fn revision(&self) -> u64 {
        self.0.lock().unwrap().messages.len() as u64
    }
    fn snapshot(&self, _: ProjectId) -> AgentSnapshot {
        self.0.lock().unwrap().clone()
    }
    fn dispatch(&self, _: ProjectId, command: AgentCommand) -> Result<(), String> {
        if let AgentCommand::Send(prompt) = command {
            self.0.lock().unwrap().messages.extend([
                ChatMessage { id: 1, role: MessageRole::User, text: prompt,
                    status: MessageStatus::Complete, tools: vec![], metrics: None },
                ChatMessage { id: 2, role: MessageRole::Assistant,
                    text: "**GPUI Kit** 是一套 Rust 桌面应用框架。\n\n它提供界面组件、数据管理、布局和文本编辑能力，帮助开发者构建完整的桌面应用。\n\n- **界面**：可直接使用的组件与样式\n- **交互**：输入、选择与编辑\n- **集成**：在同一框架内组织数据和应用逻辑".into(),
                    status: MessageStatus::Complete, tools: vec![], metrics: None },
            ]);
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
            projects: vec![Project {
                id: ProjectId(1),
                revision: 1,
                name: "Personal".into(),
                description: String::new(),
                instructions: String::new(),
                context: vec![],
            }],
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
    for (name, status, action) in [
        ("explain", ConnectionStatus::Ready, 1_usize),
        ("translate", ConnectionStatus::Ready, 2),
        ("search", ConnectionStatus::Ready, 0),
        ("disconnected", ConnectionStatus::Disconnected, 0),
        ("error", ConnectionStatus::Failed, 0),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
        });
        let error = (status == ConnectionStatus::Failed)
            .then(|| "示例：无法读取项目对话文件，请检查日志中的具体原因。".into());
        let agents = Arc::new(FixtureAgent(Mutex::new(AgentSnapshot {
            status,
            error,
            ..Default::default()
        })));
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
            assert!(window.find("quick-ask").visible());
            assert!(window.find("quick-close").visible());
            if name == "disconnected" || name == "error" {
                assert!(window.find("quick-connect-codex").visible());
            } else {
                assert!(window.try_find("quick-copy").is_some());
            }
        })?;
        cx.capture_screenshot(handle.into())?
            .save(folder.join(format!("{name}.png")))?;
    }
    println!("Rendered previews: {}", folder.display());
    Ok(())
}
