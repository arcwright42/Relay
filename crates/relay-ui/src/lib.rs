//! GPUI views over framework-independent agent service contracts.
//! Process ownership, installation and persistence live in relay-runtime / relay-acp.

#[cfg(feature = "devtools")]
mod devtools;
mod preview;
mod workbench;

pub use workbench::{FocusSearch, SendMessage, Workbench};
