//! User-facing native memory. Implementations perform I/O off the UI thread.

#[derive(Clone, Debug, Default)]
pub struct MemoryQuery {
    pub text: String,
    pub topic: Option<String>,
    pub before_memory: Option<u64>,
    pub before_source: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryProgress {
    pub sources: u64,
    pub sessions: u64,
    pub pending: u64,
    pub running: u64,
    pub done: u64,
    pub failed: u64,
    pub candidates: u64,
    pub confirmed: u64,
    pub enabled: bool,
    pub hourly_budget: u64,
    pub runs_this_hour: u64,
    pub observer_error: Option<String>,
    pub session_summaries: u64,
    pub embedding_model: Option<String>,
    pub embedding_indexed: u64,
    pub embedding_pending: u64,
    pub embedding_failed: u64,
    pub embedding_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub id: u64,
    pub title: String,
    pub preview: String,
    pub kind: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct MemoryEvidence {
    pub id: u64,
    pub revision: u64,
    pub title: String,
    pub origin: String,
}

#[derive(Clone, Debug)]
pub struct MemoryDetail {
    pub entry: MemoryEntry,
    pub body: String,
    pub topics: Vec<String>,
    pub evidence: Vec<MemoryEvidence>,
}

#[derive(Clone, Debug)]
pub struct MemoryTopic {
    pub name: String,
    pub memories: u64,
    pub sessions: u64,
}

#[derive(Clone, Debug)]
pub struct MemorySourceEntry {
    pub id: u64,
    pub title: String,
    pub origin: String,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MemorySourcePage {
    pub source: MemorySourceEntry,
    pub revision: u64,
    pub body: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub archive: Option<crate::sessions::ClientSessionId>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryOverview {
    pub progress: MemoryProgress,
    pub memories: Vec<MemoryEntry>,
    pub topics: Vec<MemoryTopic>,
    pub sources: Vec<MemorySourceEntry>,
    pub failures: Vec<MemorySourceEntry>,
    pub next_memory: Option<u64>,
    pub next_source: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum MemoryCommand {
    SetEnabled(bool),
    RetryFailed,
    Confirm(u64),
    Revise {
        id: u64,
        title: String,
        body: String,
    },
    ForgetMemory(u64),
    ForgetSource(u64),
}

pub trait MemoryService: Send + Sync {
    fn overview(&self, query: &MemoryQuery) -> Result<MemoryOverview, String>;
    fn detail(&self, id: u64) -> Result<MemoryDetail, String>;
    fn source(&self, id: u64, offset: usize) -> Result<MemorySourcePage, String>;
    /// Returns the current memory ID after confirmation or revision.
    fn apply(&self, command: MemoryCommand) -> Result<Option<u64>, String>;
}

pub struct EmptyMemoryService;
impl MemoryService for EmptyMemoryService {
    fn overview(&self, _: &MemoryQuery) -> Result<MemoryOverview, String> {
        Err("Native memory is unavailable".into())
    }
    fn detail(&self, _: u64) -> Result<MemoryDetail, String> {
        Err("Memory is unavailable".into())
    }
    fn source(&self, _: u64, _: usize) -> Result<MemorySourcePage, String> {
        Err("Source is unavailable".into())
    }
    fn apply(&self, _: MemoryCommand) -> Result<Option<u64>, String> {
        Err("Native memory is unavailable".into())
    }
}
