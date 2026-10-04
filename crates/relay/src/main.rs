use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;
use relay_core::{ThreadId, capture::Selection, settings::SettingsService, voice::VoiceService};
use relay_runtime::{
    AgentRuntime, BundledWakeResources, ClientSessionStore, FileStore, MoliFetcher, SettingsStore,
    ThreadVoiceDialogue, VoiceRuntime,
};
use relay_ui::{
    EndVoiceSession, FocusSearch, OpenAgentSettings, OpenMicrophoneSettings, OpenQuick, OpenThread,
    OpenWorkspace, Quit, RequestAccessibility, ResizeQuick, SaveFile, SendMessage, VoicePanel,
    Workbench, apply_language,
};
use std::sync::Arc;
mod voice_cli;

#[derive(Clone)]
struct Services {
    agents: Arc<AgentRuntime>,
    settings: Arc<SettingsStore>,
    threads: Arc<relay_runtime::native::NativeStore>,
    fetcher: Arc<MoliFetcher>,
    client_sessions: Arc<ClientSessionStore>,
    files: Arc<FileStore>,
    voice: Arc<VoiceRuntime>,
}
struct Desktop {
    services: Services,
    workspace: Option<(WindowHandle<Root>, Entity<Workbench>)>,
    quick: Option<(WindowHandle<Root>, Entity<Workbench>)>,
    voice_panel: Option<(WindowHandle<Root>, u64)>,
    shortcut_error: Option<String>,
    quick_dismiss: Option<relay_platform::QuickDismissMonitor>,
}
impl Global for Desktop {}

fn open_workspace(thread: Option<ThreadId>, cx: &mut App) {
    if let Some((handle, view)) = cx.global::<Desktop>().workspace.clone()
        && handle
            .update(cx, |_, window, cx| {
                if let Some(thread) = thread {
                    view.update(cx, |view, cx| view.open_thread(thread, window, cx));
                }
                window.activate_window();
            })
            .is_ok()
    {
        cx.activate(true);
        return;
    }
    open_window(false, Selection::default(), thread, None, cx);
}
fn open_quick(selection: Selection, anchor: Option<(f32, f32)>, cx: &mut App) {
    eprintln!(
        "quick: opening panel selection_chars={} permission_missing={}",
        selection.text.chars().count(),
        selection.accessibility_missing
    );
    let thread = Some(ThreadId(0));
    // A fresh capture starts a fresh toolbar, at the current selection's location.
    if let Some((handle, _)) = cx.global::<Desktop>().quick.clone() {
        let _ = handle.update(cx, |_, window, _| {
            window.set_window_title("Relay Quick Closing");
            window.remove_window();
        });
    }
    open_window(true, selection, thread, anchor, cx);
}

fn open_voice_session(cx: &mut App) {
    if cx.global::<Desktop>().voice_panel.is_some() {
        return;
    }
    let services = cx.global::<Desktop>().services.clone();
    let Some(session) = services.voice.snapshot().session else {
        return;
    };
    let bounds = Bounds::centered(None, size(px(480.), px(540.)), cx);
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            window_min_size: Some(size(px(480.), px(540.))),
            window_background: WindowBackgroundAppearance::Blurred,
            is_resizable: false,
            focus: true,
            ..Default::default()
        },
        |window, cx| {
            window.set_window_title("Relay Voice");
            let panel = cx.new(|cx| {
                VoicePanel::new(
                    services.voice.clone(),
                    services.settings.clone(),
                    window,
                    cx,
                )
            });
            cx.subscribe(&panel, |_, event: &OpenThread, cx| {
                open_workspace(event.0, cx);
            })
            .detach();
            cx.new(|cx| Root::new(panel, window, cx))
        },
    ) {
        Ok(handle) => {
            cx.global_mut::<Desktop>().voice_panel = Some((handle, session.id));
            cx.activate(true);
        }
        Err(error) => {
            eprintln!("Could not open voice session: {error}");
            services.voice.end_session(session.id);
        }
    }
}
fn open_window(
    quick: bool,
    selection: Selection,
    thread: Option<ThreadId>,
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
            if quick {
                window.set_window_title(relay_platform::QUICK_PANEL_TITLE);
                relay_platform::configure_quick_panel();
            }
            let view = cx.new(|cx| {
                let mut view = Workbench::new(
                    services.agents,
                    services.settings,
                    services.threads,
                    window,
                    cx,
                );
                view.set_client_session_service(services.client_sessions, cx);
                view.set_file_service(services.files, cx);
                view.set_voice_service(services.voice, cx);
                if quick {
                    view.capture(selection, services.fetcher, window, cx);
                }
                if let Some(thread) = thread {
                    view.open_thread(thread, window, cx);
                }
                if let Some(error) = shortcut_error {
                    view.set_quick_notice(error, cx);
                }
                view
            });
            cx.subscribe(&view, |_, event: &OpenThread, cx| {
                open_workspace(event.0, cx)
            })
            .detach();
            cx.subscribe(&view, |_, event: &OpenAgentSettings, cx| {
                open_workspace(Some(event.0), cx);
                if let Some((handle, view)) = cx.global::<Desktop>().workspace.clone() {
                    let _ = handle.update(cx, |_, window, cx| {
                        view.update(cx, |view, cx| view.open_agent_settings(event.0, window, cx));
                    });
                }
            })
            .detach();
            cx.subscribe(&view, |_, _: &RequestAccessibility, cx| {
                relay_platform::request_accessibility();
                cx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                );
            })
            .detach();
            cx.subscribe(&view, |_, _: &OpenMicrophoneSettings, cx| {
                cx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
                );
            })
            .detach();
            let quick_handle = window.window_handle();
            cx.subscribe(&view, move |_, event: &ResizeQuick, cx| {
                event.apply(quick_handle, cx, |size, _| {
                    relay_platform::resize_quick_panel(size.width.as_f32(), size.height.as_f32());
                });
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
                relay_platform::show_quick_panel();
                eprintln!("quick: panel opened {:?}", handle.window_id());
                desktop.quick = entry;
                desktop.quick_dismiss = None;
                if let Some((monitor, receiver)) = relay_platform::QuickDismissMonitor::install() {
                    desktop.quick_dismiss = Some(monitor);
                    cx.spawn(async move |cx| {
                        if receiver.recv().await.is_ok() {
                            eprintln!("quick: outside-click dismissal {:?}", handle.window_id());
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

fn configure_working_directory() -> std::io::Result<()> {
    if let Some(directory) = std::env::var_os("RELAY_WORKING_DIRECTORY") {
        let directory = std::path::Path::new(&directory);
        if !directory.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "RELAY_WORKING_DIRECTORY must be an absolute directory path",
            ));
        }
        std::env::set_current_dir(directory)?;
    }
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "--relay-mcp") {
        let result = (|| -> Result<(), String> {
            let root = args
                .get(2)
                .map(std::path::PathBuf::from)
                .ok_or("Missing MCP data directory")?;
            let id = args
                .get(3)
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .ok_or("Missing MCP scope")?;
            relay_runtime::native::serve_stdio(root, ThreadId(id)).map_err(|e| e.to_string())
        })();
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1)
        }
        return;
    }
    if let Err(error) = configure_working_directory() {
        eprintln!("Could not set Relay working directory: {error}");
        std::process::exit(2);
    }
    let directory = AgentRuntime::default_directory();
    if voice_cli::run(&directory) {
        return;
    }
    let settings = Arc::new(SettingsStore::new(directory.clone()));
    let threads = match relay_runtime::native::NativeStore::open(directory.clone()).and_then(|s| {
        s.migrate()?;
        Ok(Arc::new(s))
    }) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("Could not open Relay's native state: {e:#}. Existing data preserved.");
            return;
        }
    };
    let agents = Arc::new(AgentRuntime::with_native(
        directory.clone(),
        threads.clone(),
    ));
    let resident =
        match relay_runtime::native::ResidentWorker::start(threads.clone(), agents.clone()) {
            Ok(worker) => Arc::new(worker),
            Err(e) => {
                eprintln!("Could not start Relay: {e:#}");
                return;
            }
        };
    let files = Arc::new(FileStore::new(directory.clone()));
    let speech = Arc::new(relay_runtime::QwenSpeech::new(
        &directory,
        Arc::new(relay_platform::MacAudioOutput),
    ));
    let voice = Arc::new(VoiceRuntime::with_pipeline(
        settings.clone(),
        Arc::new(BundledWakeResources::discover()),
        Arc::new(relay_platform::MacVoiceBackend),
        speech.clone(),
        Arc::new(ThreadVoiceDialogue::new(threads.clone(), agents.clone())),
        speech,
    ));
    let client_sessions = Arc::new(ClientSessionStore::new(
        directory.clone(),
        threads.clone(),
        agents.clone(),
    ));
    let services = Services {
        agents: agents.clone(),
        settings: settings.clone(),
        threads,
        fetcher: Arc::new(MoliFetcher::new(&directory)),
        client_sessions: client_sessions.clone(),
        files,
        voice: voice.clone(),
    };
    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    application.on_reopen(|cx| open_workspace(None, cx));
    application.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        Theme::global_mut(cx).font_size = px(16.);
        apply_language(settings.snapshot().language, cx);
        let shortcut = relay_platform::Shortcut::register();
        let shutdown_fetcher = services.fetcher.clone();
        cx.set_global(Desktop {
            services,
            workspace: None,
            quick: None,
            voice_panel: None,
            shortcut_error: shortcut.as_ref().err().cloned(),
            quick_dismiss: None,
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("escape", EndVoiceSession, Some("VoiceSession")),
            KeyBinding::new("cmd-k", FocusSearch, None),
            KeyBinding::new("cmd-enter", SendMessage, None),
            KeyBinding::new("cmd-s", SaveFile, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenWorkspace, cx| open_workspace(None, cx));
        // The menu is a manual/paste entry; it must not capture Relay's own selection.
        cx.on_action(|_: &OpenQuick, cx| {
            open_quick(Selection::default(), relay_platform::pointer_position(), cx)
        });
        let shutdown_resident = resident.clone();
        cx.on_app_quit(move |cx| {
            let worker = shutdown_resident.clone();
            cx.background_executor().spawn(async move {
                worker.shutdown();
            })
        })
        .detach();
        let shutdown_voice = voice.clone();
        cx.on_app_quit(move |cx| {
            let voice = shutdown_voice.clone();
            cx.background_executor().spawn(async move {
                voice.shutdown();
            })
        })
        .detach();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            loop {
                executor.timer(std::time::Duration::from_millis(50)).await;
                if voice.snapshot().session.is_some() {
                    cx.update(open_voice_session);
                }
            }
        })
        .detach();
        cx.on_app_quit(move |cx| {
            let fetcher = shutdown_fetcher.clone();
            cx.background_executor().spawn(async move {
                fetcher.shutdown();
            })
        })
        .detach();
        cx.on_app_quit(move |cx| {
            let client_sessions = client_sessions.clone();
            cx.background_executor().spawn(async move {
                client_sessions.shutdown();
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
            if cx
                .global::<Desktop>()
                .voice_panel
                .is_some_and(|(handle, _)| handle.window_id() == closed)
            {
                let desktop = cx.global_mut::<Desktop>();
                if let Some((_, session_id)) = desktop.voice_panel.take() {
                    desktop.services.voice.end_session(session_id);
                }
            }
            eprintln!("quick: window closed {closed:?}");
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
                    eprintln!("quick: trigger dequeued");
                    let anchor = relay_platform::pointer_position();
                    let source_pid = relay_platform::frontmost_process();
                    let selection = executor
                        .spawn(async move { relay_platform::capture_selection(source_pid) })
                        .await;
                    eprintln!(
                        "quick: capture completed chars={}",
                        selection.text.chars().count()
                    );
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
