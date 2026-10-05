//! Memory engines are adapters, not owners of Relay's task scheduler or identity.
//! Lifecycle capture, context, tools, UI and background work select the SAME provider.
mod claude_mem;
mod config;
mod protocol;
#[cfg(test)]
mod tests;

use anyhow::Result;
pub use config::{ContextMode, MemoryConfig, ProviderKind, run_cli};
pub use protocol::serve_stdio;
use relay_core::{ThreadId, agents::*, memory::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryToolEvent {
    pub id: String,
    pub title: String,
    pub status: String,
    pub input: String,
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryMessage {
    pub id: u64,
    pub role: String,
    pub text: String,
    pub complete: bool,
    pub tools: Vec<MemoryToolEvent>,
}

impl From<&ChatMessage> for MemoryMessage {
    fn from(message: &ChatMessage) -> Self {
        Self {
            id: message.id,
            role: match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            }
            .into(),
            text: message.text.clone(),
            complete: message.status == MessageStatus::Complete,
            tools: message
                .tools
                .iter()
                .map(|t| MemoryToolEvent {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    status: t.status.clone(),
                    input: t.input.clone(),
                    output: t.output.clone(),
                })
                .collect(),
        }
    }
}

/// Domain events contain neither SQL identities nor Claude-Mem HTTP parameters.
pub enum MemoryEvent {
    User {
        thread: ThreadId,
        message: u64,
        text: String,
        directory: std::path::PathBuf,
    },
    Tool {
        thread: ThreadId,
        response: u64,
        request: String,
        tool: MemoryToolEvent,
        directory: std::path::PathBuf,
    },
    Turn {
        thread: ThreadId,
        messages: Vec<MemoryMessage>,
        directory: std::path::PathBuf,
    },
    SessionEnd {
        thread: ThreadId,
        last_message: u64,
    },
    Archive {
        client: String,
        session: String,
        title: String,
        messages: Vec<(u64, String, String)>,
        directory: Option<std::path::PathBuf>,
    },
}

pub struct ContextRequest<'a> {
    pub thread: ThreadId,
    pub query: &'a str,
    pub fresh_session: bool,
}

pub trait MemoryWorker: Send + Sync {
    fn shutdown(&self);
}

pub trait MemoryProvider: MemoryService {
    /// Changes invalidate execution checkpoints and MCP connections.
    fn identity(&self) -> String;
    fn context(&self, request: &ContextRequest<'_>) -> Result<String>;
    fn record(&self, event: &MemoryEvent) -> Result<()>;
    /// Tool schemas and execution share one capability boundary. Task tools stay in Relay.
    fn tools(&self, caller: ThreadId) -> Vec<Value>;
    fn call(&self, caller: ThreadId, name: &str, args: &Value) -> Result<Value>;
    fn start(self: Arc<Self>) -> Result<Box<dyn MemoryWorker>>;
    /// Historical import is explicit for external engines; live delivery recovery is internal.
    fn imports_history(&self) -> bool;
}

pub fn open(root: &Path) -> Result<Arc<dyn MemoryProvider>> {
    std::fs::create_dir_all(root)?;
    let config = MemoryConfig::load(root)?;
    Ok(Arc::new(claude_mem::ClaudeMemProvider::open(root, config)?))
}
