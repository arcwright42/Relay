//! GPUI views over framework-independent agent service contracts.
//! Process ownership, installation and persistence live in relay-runtime / relay-acp.

#[cfg(feature = "devtools")]
mod devtools;
mod i18n;
mod locale;
mod preview;
mod workbench;

pub use locale::{OpenQuick, OpenWorkspace, Quit, apply_language};
pub use workbench::{FocusSearch, OpenProject, RequestAccessibility, SendMessage, Workbench};
