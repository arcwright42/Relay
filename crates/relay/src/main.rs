use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;
use relay_ui::{FocusSearch, SendMessage, Workbench};

actions!(relay, [Quit]);

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            Theme::global_mut(cx).font_size = px(14.);
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-k", FocusSearch, None),
                KeyBinding::new("cmd-enter", SendMessage, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let menus = vec![Menu::new("Relay").items([MenuItem::action("Quit Relay", Quit)])];
            #[cfg(feature = "devtools")]
            let menus = {
                let mut menus = menus;
                menus.push(Menu::new("开发者").items([MenuItem::action(
                    "检查元素",
                    gpui_kit::component::ToggleInspector,
                )]));
                menus
            };
            cx.set_menus(menus);
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
                        let view = cx.new(|cx| Workbench::new(window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("Relay could not open its workspace window");
            })
            .detach();
            cx.activate(true);
        });
}
