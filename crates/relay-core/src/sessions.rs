//! Read-only native client archives are independent of Relay's execution sessions.
use crate::{ThreadId, agents::MessageRole};
use std::{path::PathBuf, sync::Arc};

pub const CLIENT_SYNC_INTERVAL_SECS: u64 = 30 * 60;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientSessionId(pub String);

#[derive(Clone, Debug)]
pub struct ClientSession {
    pub id: ClientSessionId,
    pub client: String,
    pub native_id: String,
    pub title: String,
    pub working_directory: PathBuf,
    pub source: PathBuf,
    pub thread: Option<ThreadId>,
    pub updated_at: String,
    pub message_count: usize,
    pub available: bool,
}

#[derive(Clone, Debug)]
pub struct ClientMessage {
    /// Byte offset of the complete source record; stable across append-only syncs.
    pub id: u64,
    pub role: MessageRole,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ClientSessionDetail {
    pub session: ClientSession,
    pub messages: Arc<Vec<ClientMessage>>,
}

#[derive(Clone, Debug, Default)]
pub struct ClientSessionsSnapshot {
    pub sessions: Vec<ClientSession>,
    pub syncing: bool,
    pub saving: bool,
    pub last_sync_unix: Option<u64>,
    pub next_sync_unix: Option<u64>,
    pub updated_sessions: usize,
    pub scanned_files: usize,
    pub failed_files: usize,
    pub selected: Option<ClientSessionId>,
    pub detail: Option<ClientSessionDetail>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ClientSessionsCommand {
    Sync,
    Open(ClientSessionId),
    Assign {
        session: ClientSessionId,
        thread: Option<ThreadId>,
    },
}

pub trait ClientSessionsService: Send + Sync {
    fn revision(&self) -> u64;
    fn snapshot(&self) -> ClientSessionsSnapshot;
    fn dispatch(&self, command: ClientSessionsCommand) -> Result<(), String>;
}

#[derive(Default)]
pub struct EmptyClientSessions;
impl ClientSessionsService for EmptyClientSessions {
    fn revision(&self) -> u64 {
        0
    }
    fn snapshot(&self) -> ClientSessionsSnapshot {
        ClientSessionsSnapshot::default()
    }
    fn dispatch(&self, _: ClientSessionsCommand) -> Result<(), String> {
        Err("Local client sessions are unavailable.".into())
    }
}
