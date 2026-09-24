//! The only Relay package containing native FFI. UI stays on GPUI's main thread.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Shortcut, capture_selection};
