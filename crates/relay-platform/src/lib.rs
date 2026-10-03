//! The only Relay package containing native FFI. UI stays on GPUI's main thread.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{
    Shortcut, capture_selection, frontmost_process, pointer_position, request_accessibility,
};

#[cfg(target_os = "macos")]
mod panel;
mod placement;
#[cfg(target_os = "macos")]
pub use panel::{
    QUICK_PANEL_TITLE, configure_quick_panel, quick_origin, resize_quick_panel, show_quick_panel,
};

#[cfg(target_os = "macos")]
mod dismiss;
#[cfg(target_os = "macos")]
pub use dismiss::QuickDismissMonitor;

#[cfg(target_os = "macos")]
mod voice;
#[cfg(target_os = "macos")]
pub use voice::{MacAudioOutput, MacSpeechOutput, MacVoiceBackend, probe_wake_file};
