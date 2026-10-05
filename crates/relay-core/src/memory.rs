//! Provider-independent memory presentation. Implementations perform I/O off the UI thread.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryCapabilities {
    pub forget_memory: bool,
    pub retry: bool,
}

#[derive(Clone, Debug)]
pub struct MemoryProviderInfo {
    pub id: String,
    pub name: String,
    pub capabilities: MemoryCapabilities,
    pub description: String,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryQuery {
    pub text: String,
    /// Opaque provider cursor; the UI must not interpret it as an observation ID.
    pub before_memory: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryProgress {
    pub provider_status: Option<String>,
    /// Delivery counters are separate from extraction and indexing progress.
    pub pending: u64,
    pub running: u64,
    pub accepted: u64,
    pub failed: u64,
    pub uncertain: u64,
    pub skipped: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub id: u64,
    pub title: String,
    pub preview: String,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct MemoryDetail {
    pub entry: MemoryEntry,
    pub body: String,
    pub concepts: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryOverview {
    pub progress: MemoryProgress,
    pub memories: Vec<MemoryEntry>,
    pub next_memory: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum MemoryCommand {
    RetryFailed,
    ForgetMemory(u64),
}

pub trait MemoryService: Send + Sync {
    /// No I/O. Capabilities describe operations the provider actually implements.
    fn provider(&self) -> MemoryProviderInfo;
    fn overview(&self, query: &MemoryQuery) -> Result<MemoryOverview, String>;
    fn detail(&self, id: u64) -> Result<MemoryDetail, String>;
    fn apply(&self, command: MemoryCommand) -> Result<(), String>;
}

pub struct EmptyMemoryService;
impl MemoryService for EmptyMemoryService {
    fn provider(&self) -> MemoryProviderInfo {
        MemoryProviderInfo {
            id: "unavailable".into(),
            name: "Memory unavailable".into(),
            capabilities: MemoryCapabilities::default(),
            description: String::new(),
        }
    }
    fn overview(&self, _: &MemoryQuery) -> Result<MemoryOverview, String> {
        Err("Memory is unavailable".into())
    }
    fn detail(&self, _: u64) -> Result<MemoryDetail, String> {
        Err("Memory is unavailable".into())
    }
    fn apply(&self, _: MemoryCommand) -> Result<(), String> {
        Err("Memory is unavailable".into())
    }
}
