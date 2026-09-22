//! Framework-free contracts between the desktop and the agent runtime.
use crate::ProjectId;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AgentSource {
    #[default]
    Managed,
    Local(PathBuf),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Preparing(String),
    Connecting,
    NeedsAuthentication,
    Authenticating,
    Ready,
    Running,
    Cancelling,
    Failed,
}

impl ConnectionStatus {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Preparing(_)
                | Self::Connecting
                | Self::Authenticating
                | Self::Running
                | Self::Cancelling
        )
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Disconnected => "Not connected",
            Self::Preparing(step) => step,
            Self::Connecting => "Connecting…",
            Self::NeedsAuthentication => "Sign in to continue",
            Self::Authenticating => "Complete sign-in in your browser…",
            Self::Ready => "Connected",
            Self::Running => "Working…",
            Self::Cancelling => "Stopping…",
            Self::Failed => "Connection failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigChoice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub current: String,
    pub choices: Vec<ConfigChoice>,
}

impl SessionConfig {
    pub fn current_name(&self) -> &str {
        self.choices
            .iter()
            .find(|v| v.id == self.current)
            .map_or(&self.current, |v| &v.name)
    }
}

#[derive(Clone, Debug)]
pub struct AuthenticationMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageStatus {
    Complete,
    Streaming,
    Interrupted,
}

#[derive(Clone, Debug)]
pub struct ToolActivity {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub id: u64,
    pub role: MessageRole,
    pub text: String,
    pub status: MessageStatus,
    pub tools: Vec<ToolActivity>,
    pub metrics: Option<TurnMetrics>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContextDeliveryKind {
    #[default]
    Unchanged,
    Snapshot,
    Delta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnOutcome {
    Complete,
    Cancelled,
    Refused,
    Failed,
}

/// Codex ACP 1.12 reports the last model request, not every request in a tool-using turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_read_tokens: Option<u64>,
    pub cached_write_tokens: Option<u64>,
    pub thought_tokens: Option<u64>,
}

impl TokenUsage {
    /// The pinned Codex adapter reports input excluding cached reads/writes.
    pub fn cache_read_ratio(&self) -> Option<f64> {
        let read = self.cached_read_tokens?;
        let total = self
            .input_tokens
            .checked_add(read)?
            .checked_add(self.cached_write_tokens.unwrap_or(0))?;
        (total > 0).then_some(read as f64 / total as f64)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnMetrics {
    pub model: Option<String>,
    pub first_text_ms: Option<u64>,
    pub total_ms: Option<u64>,
    pub context_kind: ContextDeliveryKind,
    pub context_revision: Option<u64>,
    pub context_bytes: usize,
    pub restored_history: bool,
    pub usage: Option<TokenUsage>,
    pub outcome: Option<TurnOutcome>,
}

#[derive(Clone, Debug)]
pub struct PermissionChoice {
    pub id: String,
    pub name: String,
    pub allows: bool,
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: u64,
    pub title: String,
    pub detail: String,
    pub choices: Vec<PermissionChoice>,
}

#[derive(Clone, Debug, Default)]
pub struct AgentSnapshot {
    pub status: ConnectionStatus,
    pub source: AgentSource,
    pub installed: bool,
    pub runtime_version: Option<String>,
    pub working_directory: PathBuf,
    pub configs: Vec<SessionConfig>,
    pub pending_config: Option<String>,
    pub auth_methods: Vec<AuthenticationMethod>,
    pub messages: Vec<ChatMessage>,
    pub permissions: Vec<PermissionRequest>,
    pub error: Option<String>,
    pub local_installations: Vec<PathBuf>,
    pub discovering: bool,
}

impl AgentSnapshot {
    pub fn model(&self) -> Option<&SessionConfig> {
        self.configs
            .iter()
            .find(|c| c.category.as_deref() == Some("model"))
    }
}

#[derive(Clone, Debug)]
pub enum AgentCommand {
    Connect(AgentSource),
    Disconnect,
    Authenticate(String),
    SetConfig { id: String, value: String },
    Send(String),
    Cancel,
    AnswerPermission { id: u64, choice: Option<String> },
    SetWorkingDirectory(PathBuf),
    DiscoverLocal,
}

/// Implementations perform I/O in background workers. These methods must not block UI rendering.
pub trait AgentService: Send + Sync {
    fn revision(&self) -> u64;
    fn snapshot(&self, project: ProjectId) -> AgentSnapshot;
    fn dispatch(&self, project: ProjectId, command: AgentCommand) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_cache_usage_is_distinct_from_a_measured_zero() {
        let mut usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 10,
            cached_read_tokens: None,
            cached_write_tokens: None,
            thought_tokens: None,
        };
        assert_eq!(usage.cache_read_ratio(), None);
        usage.cached_read_tokens = Some(0);
        assert_eq!(usage.cache_read_ratio(), Some(0.));
        usage.cached_read_tokens = Some(900);
        assert_eq!(usage.cache_read_ratio(), Some(0.9));
        usage.cached_write_tokens = Some(1_000);
        assert_eq!(usage.cache_read_ratio(), Some(0.45));
        usage.input_tokens = u64::MAX;
        assert_eq!(usage.cache_read_ratio(), None);
    }
}
