//! Views and session-local interaction state. No agent processes are started here.

#[cfg(feature = "devtools")]
mod devtools;
mod preview;
mod workbench;

pub use workbench::{FocusSearch, SendMessage, Workbench};
