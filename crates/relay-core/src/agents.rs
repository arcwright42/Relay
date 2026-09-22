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
