//! Durable thread knowledge is independent of any harness or execution session.
use crate::ThreadId;

pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_INSTRUCTIONS_CHARS: usize = 8_000;
pub const MAX_ITEM_CHARS: usize = 12_000;
pub const MAX_SELECTED_CONTEXT_CHARS: usize = 32_000;
pub const MAX_CONTEXT_ITEMS: usize = 64;
pub const MAX_MEMORY_ITEMS: usize = 32;
pub const MAX_MEMORY_CHARS: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContextId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextItem {
    pub id: ContextId,
    pub name: String,
    pub content: String,
    pub included: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemoryId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MemoryKind {
    #[default]
    Fact,
    Decision,
}

impl MemoryKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Decision => "decision",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "fact" => Some(Self::Fact),
            "decision" => Some(Self::Decision),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemorySource {
    /// A visible message in this thread's Relay conversation, not a native ACP ID.
    Message { message_id: u64 },
    ClientSession {
        client: String,
        session_id: String,
        message_id: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryItem {
    pub id: MemoryId,
    pub kind: MemoryKind,
    pub name: String,
    pub content: String,
    /// None means no specific visible message is linked. Edits retain the original source.
    pub source: Option<MemorySource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: ThreadId,
    pub revision: u64,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub context: Vec<ContextItem>,
    pub memory: Vec<MemoryItem>,
}

#[derive(Clone, Debug, Default)]
pub struct ThreadCatalog {
    pub revision: u64,
    pub threads: Vec<Thread>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ThreadDraft {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Clone, Debug)]
pub enum ThreadCommand {
    Create(ThreadDraft),
    CreateAtRevision {
        expected_catalog_revision: u64,
        draft: ThreadDraft,
    },
    Edit {
        thread: ThreadId,
        expected_revision: u64,
        draft: ThreadDraft,
    },
    SaveContext {
        thread: ThreadId,
        expected_revision: u64,
        id: Option<ContextId>,
        name: String,
        content: String,
        included: bool,
    },
    RemoveContext {
        thread: ThreadId,
        expected_revision: u64,
        id: ContextId,
    },
    SaveMemory {
        thread: ThreadId,
        expected_revision: u64,
        id: Option<MemoryId>,
        kind: MemoryKind,
        name: String,
        content: String,
        source: Option<MemorySource>,
    },
    RemoveMemory {
        thread: ThreadId,
        expected_revision: u64,
        id: MemoryId,
    },
}

pub trait ThreadService: Send + Sync {
    fn activity(&self) -> Vec<TaskActivity> {
        Vec::new()
    }
    fn revision(&self) -> u64 {
        self.snapshot().revision
    }
    fn thread(&self, id: ThreadId) -> Option<Thread> {
        self.snapshot().threads.into_iter().find(|p| p.id == id)
    }
    /// Memory-only, safe during rendering. Published values are already durable.
    fn snapshot(&self) -> ThreadCatalog;
    /// Serialized, version-checked disk transaction. Call on a background executor.
    fn apply(&self, command: ThreadCommand) -> Result<ThreadId, String>;
}

#[derive(Clone, Debug)]
pub struct TaskActivity {
    pub thread: ThreadId,
    pub name: String,
    pub state: String,
    pub summary: String,
}
