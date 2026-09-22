//! Durable project knowledge is independent of any harness or execution session.
use crate::ProjectId;

pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_INSTRUCTIONS_CHARS: usize = 8_000;
pub const MAX_ITEM_CHARS: usize = 12_000;
pub const MAX_SELECTED_CONTEXT_CHARS: usize = 32_000;
pub const MAX_CONTEXT_ITEMS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContextId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextItem {
    pub id: ContextId,
    pub name: String,
    pub content: String,
    pub included: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub revision: u64,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub context: Vec<ContextItem>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectCatalog {
    pub revision: u64,
    pub projects: Vec<Project>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectDraft {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Clone, Debug)]
pub enum ProjectCommand {
    Create(ProjectDraft),
    Edit {
        project: ProjectId,
        expected_revision: u64,
        draft: ProjectDraft,
    },
    SaveContext {
        project: ProjectId,
        expected_revision: u64,
        id: Option<ContextId>,
        name: String,
        content: String,
        included: bool,
    },
    RemoveContext {
        project: ProjectId,
        expected_revision: u64,
        id: ContextId,
    },
}

pub trait ProjectService: Send + Sync {
    fn revision(&self) -> u64 {
        self.snapshot().revision
    }
    fn project(&self, id: ProjectId) -> Option<Project> {
        self.snapshot().projects.into_iter().find(|p| p.id == id)
    }
    /// Memory-only, safe during rendering. Published values are already durable.
    fn snapshot(&self) -> ProjectCatalog;
    /// Serialized, version-checked disk transaction. Call on a background executor.
    fn apply(&self, command: ProjectCommand) -> Result<ProjectId, String>;
}
