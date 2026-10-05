//! Thread data belongs to Relay, independently of any UI or agent session.
//! This package intentionally has no framework or runtime dependencies.

pub mod agents;
pub mod capture;
pub mod files;
pub mod memory;
pub mod sessions;
pub mod settings;
pub mod threads;
pub mod voice;

pub use threads::{ContextId, ContextItem, MemoryId, MemoryItem, MemoryKind, MemorySource, Thread};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u64);
