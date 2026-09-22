//! Project data belongs to Relay, independently of any UI or agent session.
//! This package intentionally has no framework or runtime dependencies.

pub mod agents;
pub mod settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextKind {
    Web,
    Document,
    Image,
}

#[derive(Clone, Debug)]
pub struct ContextItem {
    pub name: String,
    pub kind: ContextKind,
}

#[derive(Debug)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: String,
    pub context: Vec<ContextItem>,
}

impl Project {
    pub fn context_count(&self, kind: ContextKind) -> usize {
        self.context.iter().filter(|item| item.kind == kind).count()
    }
}
