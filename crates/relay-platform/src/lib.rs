//! The only Relay package containing native FFI. UI stays on GPUI's main thread.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Shortcut, capture_selection, pointer_position, request_accessibility};

#[cfg(target_os = "macos")]
mod panel;
mod placement;
#[cfg(target_os = "macos")]
pub use panel::{QUICK_PANEL_TITLE, quick_origin, resize_quick_panel};
