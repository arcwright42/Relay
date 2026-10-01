//! Project data belongs to Relay, independently of any UI or agent session.
//! This package intentionally has no framework or runtime dependencies.

pub mod agents;
pub mod capture;
pub mod projects;
pub mod routing;
pub mod sessions;
pub mod settings;

pub use projects::{
    ContextId, ContextItem, MemoryId, MemoryItem, MemoryKind, MemorySource, Project,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(pub u64);
