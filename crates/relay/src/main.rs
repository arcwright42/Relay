use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;
use relay_core::{ProjectId, capture::Selection, settings::SettingsService};
use relay_runtime::{AgentRuntime, JevRouter, MoliFetcher, ProjectStore, SettingsStore};
use relay_ui::{
    FocusSearch, OpenProject, OpenQuick, OpenWorkspace, Quit, RequestAccessibility, SendMessage,
    Workbench, apply_language,
};
use std::sync::Arc;

#[derive(Clone)]
struct Services {
    agents: Arc<AgentRuntime>,
    settings: Arc<SettingsStore>,
    projects: Arc<ProjectStore>,
    routing: Arc<JevRouter>,
    fetcher: Arc<MoliFetcher>,
}
struct Desktop {
    services: Services,
    workspace: Option<(WindowHandle<Root>, Entity<Workbench>)>,
    quick: Option<(WindowHandle<Root>, Entity<Workbench>)>,
    shortcut_error: Option<String>,
}
impl Global for Desktop {}

fn open_workspace(project: Option<ProjectId>, cx: &mut App) {
    if let Some((handle, view)) = cx.global::<Desktop>().workspace.clone()
        && handle
            .update(cx, |_, window, cx| {
                if let Some(project) = project {
                    view.update(cx, |view, cx| view.open_project(project, window, cx));
                }
                window.activate_window();
            })
            .is_ok()
    {
        cx.activate(true);
        return;
    }
    open_window(false, Selection::default(), project, cx);
}
fn open_quick(selection: Selection, cx: &mut App) {
    let project = cx
        .global::<Desktop>()
        .workspace
        .as_ref()
        .and_then(|(_, view)| view.read(cx).current_project());
    // A fresh capture starts a fresh toolbar, at the current selection's location.
    if let Some((handle, _)) = cx.global::<Desktop>().quick.clone() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    open_window(true, selection, project, cx);
}
fn open_window(quick: bool, selection: Selection, project: Option<ProjectId>, cx: &mut App) {
    let services = cx.global::<Desktop>().services.clone();
    let shortcut_error = cx.global::<Desktop>().shortcut_error.clone();
    let mut bounds = Bounds::centered(
        None,
        if quick {
            size(px(640.), px(54.))
        } else {
            size(px(1440.), px(940.))
        },
        cx,
    );
    let mut display_id = None;
    if quick && let Some((x, y)) = relay_platform::pointer_position() {
        let pointer = point(px(x), px(y));
        if let Some(display) = cx
            .displays()
            .into_iter()
            .find(|d| d.bounds().contains(&pointer))
        {
            let screen = display.bounds();
            display_id = Some(display.id());
            // Reserve room below for the result panel and keep the toolbar on screen.
            bounds.origin = point(
                px(x).clamp(
                    screen.left() + px(8.),
                    (screen.right() - bounds.size.width - px(8.)).max(screen.left() + px(8.)),
                ),
                px(y + 16.).clamp(
                    screen.top() + px(30.),
                    (screen.bottom() - px(590.)).max(screen.top() + px(30.)),
                ),
            );
            bounds.origin -= screen.origin;
        }
    }
    let mut view_handle = None;
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            titlebar: (!quick).then(|| TitlebarOptions {
                title: Some("Relay".into()),
                appears_transparent: true,
                ..Default::default()
            }),
            kind: if quick {
                WindowKind::PopUp
            } else {
                WindowKind::Normal
            },
            window_min_size: Some(if quick {
                size(px(320.), px(54.))
            } else {
                size(px(1080.), px(720.))
            }),
            window_background: if quick {
                WindowBackgroundAppearance::Blurred
            } else {
                WindowBackgroundAppearance::Opaque
            },
            focus: !quick,
            is_resizable: !quick,
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| {
                let mut view = Workbench::new(
                    services.agents,
                    services.settings,
                    services.projects,
                    services.routing,
                    window,
                    cx,
                );
                if quick {
                    view.capture(selection, services.fetcher, window, cx);
                }
                if let Some(project) = project {
                    view.open_project(project, window, cx);
                }
                if let Some(error) = shortcut_error {
                    view.set_quick_notice(error, cx);
                }
                view
            });
            cx.subscribe(&view, |_, event: &OpenProject, cx| {
                open_workspace(event.0, cx)
            })
            .detach();
            cx.subscribe(&view, |_, _: &RequestAccessibility, cx| {
                relay_platform::request_accessibility();
                cx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                );
            })
            .detach();
            view_handle = Some(view.clone());
            cx.new(|cx| {
                let root = Root::new(view, window, cx);
                if quick {
                    root.bordered(false).bg(rgba(0x00000000))
                } else {
                    root
                }
            })
        },
    );
    match handle {
        Ok(handle) => {
            let entry = Some((handle, view_handle.expect("window view")));
            let desktop = cx.global_mut::<Desktop>();
            if quick {
                desktop.quick = entry;
            } else {
                desktop.workspace = entry;
            }
            if !quick {
                cx.activate(true);
            }
        }
        Err(error) => eprintln!("Could not open Relay window: {error}"),
    }
}

fn main() {
    let directory = AgentRuntime::default_directory();
    let settings = Arc::new(SettingsStore::new(directory.clone()));
    let projects = Arc::new(ProjectStore::new(directory.clone()));
    let agents = Arc::new(AgentRuntime::new(directory.clone(), projects.clone()));
    let routing = Arc::new(JevRouter::new(&directory, projects.clone(), agents.clone()));
    let services = Services {
        agents: agents.clone(),
        settings: settings.clone(),
        projects,
        routing,
        fetcher: Arc::new(MoliFetcher::new(&directory)),
    };
    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    application.on_reopen(|cx| open_workspace(None, cx));
    application.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        Theme::global_mut(cx).font_size = px(14.);
        apply_language(settings.snapshot().language, cx);
        let shortcut = relay_platform::Shortcut::register();
        let shutdown_fetcher = services.fetcher.clone();
        cx.set_global(Desktop {
            services,
            workspace: None,
            quick: None,
            shortcut_error: shortcut.as_ref().err().cloned(),
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-k", FocusSearch, None),
            KeyBinding::new("cmd-enter", SendMessage, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenWorkspace, cx| open_workspace(None, cx));
        // The menu is a manual/paste entry; it must not capture Relay's own selection.
        cx.on_action(|_: &OpenQuick, cx| open_quick(Selection::default(), cx));
        cx.on_app_quit(move |cx| {
            let fetcher = shutdown_fetcher.clone();
            cx.background_executor().spawn(async move {
                fetcher.shutdown();
            })
        })
        .detach();
        cx.on_app_quit(move |cx| {
            let agents = agents.clone();
            cx.background_executor().spawn(async move {
                agents.shutdown();
            })
        })
        .detach();
        cx.on_app_quit(move |cx| {
            let settings = settings.clone();
            cx.background_executor().spawn(async move {
                settings.shutdown();
            })
        })
        .detach();
        // Deliberately keep the application alive when the last window closes.
        cx.on_window_closed(|cx, closed| {
            let desktop = cx.global_mut::<Desktop>();
            if desktop
                .workspace
                .as_ref()
                .is_some_and(|(handle, _)| handle.window_id() == closed)
            {
                desktop.workspace = None;
            }
            if desktop
                .quick
                .as_ref()
                .is_some_and(|(handle, _)| handle.window_id() == closed)
            {
                desktop.quick = None;
            }
        })
        .detach();
        if let Ok(shortcut) = shortcut {
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                while shortcut.next_trigger().await {
                    let selection = executor
                        .spawn(async { relay_platform::capture_selection() })
                        .await;
                    shortcut.discard_pending();
                    cx.update(|cx| open_quick(selection, cx));
                }
            })
            .detach();
        }
        open_workspace(None, cx);
        if cx.global::<Desktop>().shortcut_error.is_some() {
            open_quick(Selection::default(), cx);
        }
    });
}
