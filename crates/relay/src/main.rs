use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;
use relay_core::{ProjectId, capture::Selection, settings::SettingsService};
use relay_runtime::{AgentRuntime, JevRouter, MoliFetcher, ProjectStore, SettingsStore};
use relay_ui::{
    FocusSearch, OpenProject, OpenQuick, OpenWorkspace, Quit, RequestAccessibility, ResizeQuick,
    SendMessage, Workbench, apply_language,
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
    quick_dismiss: Option<relay_platform::QuickDismissMonitor>,
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
    open_window(false, Selection::default(), project, None, cx);
}
fn open_quick(selection: Selection, anchor: Option<(f32, f32)>, cx: &mut App) {
    let project = cx
        .global::<Desktop>()
        .workspace
        .as_ref()
        .and_then(|(_, view)| view.read(cx).current_project());
    // A fresh capture starts a fresh toolbar, at the current selection's location.
    if let Some((handle, _)) = cx.global::<Desktop>().quick.clone() {
        let _ = handle.update(cx, |_, window, _| {
            window.set_window_title("Relay Quick Closing");
            window.remove_window();
        });
    }
    open_window(true, selection, project, anchor, cx);
}
fn open_window(
    quick: bool,
    selection: Selection,
    project: Option<ProjectId>,
    anchor: Option<(f32, f32)>,
    cx: &mut App,
) {
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
    if quick
        && let Some(placement) =
            anchor.and_then(|pointer| relay_platform::quick_origin(pointer, 640., 54.))
    {
        // GPUI's MacDisplay::bounds drops every display origin. Use the native
        // display ID and display-local coordinates together; never infer a screen
        // from those origin-less bounds.
        display_id = Some(DisplayId::new(placement.display_id as u64));
        bounds.origin = point(px(placement.left), px(placement.top));
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
                WindowKind::Floating
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
            if quick {
                window.set_window_title(relay_platform::QUICK_PANEL_TITLE);
                relay_platform::configure_quick_panel();
            }
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
            cx.subscribe(&view, |_, event: &ResizeQuick, _| {
                relay_platform::resize_quick_panel(event.0.width.as_f32(), event.0.height.as_f32());
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
                desktop.quick_dismiss = None;
                if let Some((monitor, receiver)) = relay_platform::QuickDismissMonitor::install() {
                    desktop.quick_dismiss = Some(monitor);
                    cx.spawn(async move |cx| {
                        if receiver.recv().await.is_ok() {
                            // Bound to this handle: a delayed click cannot close a newer panel.
                            let _ = handle.update(cx, |_, window, _| window.remove_window());
                        }
                    })
                    .detach();
                } else {
                    eprintln!("Could not install quick-panel outside-click monitors");
                }
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
            quick_dismiss: None,
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-k", FocusSearch, None),
            KeyBinding::new("cmd-enter", SendMessage, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenWorkspace, cx| open_workspace(None, cx));
        // The menu is a manual/paste entry; it must not capture Relay's own selection.
        cx.on_action(|_: &OpenQuick, cx| {
            open_quick(Selection::default(), relay_platform::pointer_position(), cx)
        });
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
                desktop.quick_dismiss = None;
            }
        })
        .detach();
        if let Ok(shortcut) = shortcut {
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                while shortcut.next_trigger().await {
                    let anchor = relay_platform::pointer_position();
                    let source_pid = relay_platform::frontmost_process();
                    let selection = executor
                        .spawn(async move { relay_platform::capture_selection(source_pid) })
                        .await;
                    shortcut.discard_pending();
                    cx.update(|cx| open_quick(selection, anchor, cx));
                }
            })
            .detach();
        }
        open_workspace(None, cx);
        if cx.global::<Desktop>().shortcut_error.is_some() {
            open_quick(Selection::default(), relay_platform::pointer_position(), cx);
        }
    });
}
