use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;
use relay_core::{ProjectId, settings::SettingsService};
use relay_runtime::{AgentRuntime, SettingsStore};
use relay_ui::{FocusSearch, Quit, SendMessage, Workbench, apply_language};
use std::sync::Arc;

fn main() {
    let directory = AgentRuntime::default_directory();
    let settings = Arc::new(SettingsStore::new(directory.clone()));
    let agents = Arc::new(AgentRuntime::new(
        directory,
        [ProjectId(1), ProjectId(2), ProjectId(3)],
    ));
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            Theme::global_mut(cx).font_size = px(14.);
            apply_language(settings.snapshot().language, cx);
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-k", FocusSearch, None),
                KeyBinding::new("cmd-enter", SendMessage, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let shutdown_agents = agents.clone();
            cx.on_app_quit(move |cx| {
                let agents = shutdown_agents.clone();
                cx.background_executor().spawn(async move {
                    agents.shutdown();
                })
            })
            .detach();
            let shutdown_settings = settings.clone();
            cx.on_app_quit(move |cx| {
                let settings = shutdown_settings.clone();
                cx.background_executor().spawn(async move {
                    settings.shutdown();
                })
            })
            .detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1440.), px(940.)), cx);
            cx.spawn(async move |cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Relay".into()),
                            appears_transparent: true,
                            traffic_light_position: Some(point(px(20.), px(20.))),
                        }),
                        window_min_size: Some(size(px(1080.), px(720.))),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| Workbench::new(agents, settings, window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("Relay could not open its workspace window");
            })
            .detach();
            cx.activate(true);
        });
}
