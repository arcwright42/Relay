//! Project data belongs to Relay, independently of any UI or agent session.
//! This package intentionally has no framework or runtime dependencies.

pub mod agents;
pub mod projects;
pub mod settings;

pub use projects::{ContextId, ContextItem, Project};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(pub u64);
