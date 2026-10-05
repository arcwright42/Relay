//! Local client snapshots. Sources are read-only; Relay stores its own archive and cursor.
mod codex;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, bail, ensure};
use relay_core::sessions::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
struct SavedSession {
    client: String,
    native_id: String,
    title: String,
    working_directory: PathBuf,
    source: PathBuf,
    updated_at: String,
    message_count: usize,
    available: bool,
    stamp: codex::FileStamp,
}

impl SavedSession {
    fn id(&self) -> ClientSessionId {
        ClientSessionId(format!("{}:{}", self.client, self.native_id))
    }
    fn view(&self) -> ClientSession {
        ClientSession {
            id: self.id(),
            client: self.client.clone(),
            native_id: self.native_id.clone(),
            title: self.title.clone(),
            working_directory: self.working_directory.clone(),
            source: self.source.clone(),
            updated_at: self.updated_at.clone(),
            message_count: self.message_count,
            available: self.available,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedMessage {
    id: u64,
    role: String,
    text: String,
}

#[derive(Serialize, Deserialize)]
struct Archive {
    version: u32,
    session: SavedSession,
    offset: u64,
    prefix_sha256: String,
    messages: Vec<SavedMessage>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Index {
    version: u32,
    sessions: BTreeMap<String, SavedSession>,
    last_sync_unix: Option<u64>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: BTreeMap::new(),
            last_sync_unix: None,
        }
    }
}

struct Shared {
    view: Mutex<ClientSessionsSnapshot>,
    revision: AtomicU64,
    stopped: AtomicBool,
}

impl Shared {
    fn update(&self, f: impl FnOnce(&mut ClientSessionsSnapshot)) {
        f(&mut self.view.lock().expect("client sessions lock"));
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn publish(&self, index: &Index) {
        let mut sessions: Vec<_> = index.sessions.values().map(SavedSession::view).collect();
        sessions.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        self.update(|view| {
            view.sessions = sessions;
            view.last_sync_unix = index.last_sync_unix;
            if let Some(detail) = &mut view.detail
                && let Some(session) = index.sessions.get(&detail.session.id.0)
            {
                detail.session = session.view();
            }
        });
    }
}

enum Work {
    Command(ClientSessionsCommand),
    Shutdown,
}

pub struct ClientSessionStore {
    shared: Arc<Shared>,
    sender: mpsc::Sender<Work>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ClientSessionStore {
    pub fn new(root: PathBuf) -> Self {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
            });
        Self::with_home(root, home)
    }

    fn with_home(root: PathBuf, home: PathBuf) -> Self {
        let directory = root.join("client-sessions");
        let loaded = load_index(&directory);
        let blocked = loaded.as_ref().err().map(|error| {
            format!("Could not read local session index: {error:#}. The file has been preserved.")
        });
        let index = loaded.unwrap_or_default();
        let shared = Arc::new(Shared {
            view: Mutex::new(ClientSessionsSnapshot {
                error: blocked.clone(),
                syncing: blocked.is_none(),
                ..Default::default()
            }),
            revision: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
        });
        shared.publish(&index);
        let state = shared.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut index = index;
            let mut next_sync = Instant::now();
            while !state.stopped.load(Ordering::Acquire) {
                let work = if next_sync <= Instant::now() {
                    Work::Command(ClientSessionsCommand::Sync)
                } else {
                    match receiver.recv_timeout(next_sync.saturating_duration_since(Instant::now()))
                    {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            Work::Command(ClientSessionsCommand::Sync)
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                };
                let command = match work {
                    Work::Shutdown => break,
                    Work::Command(command) => command,
                };
                if let Some(error) = &blocked {
                    state.update(|v| {
                        v.error = Some(error.clone());
                        v.syncing = false;
                    });
                    next_sync = Instant::now() + Duration::from_secs(CLIENT_SYNC_INTERVAL_SECS);
                    continue;
                }
                let is_sync = matches!(command, ClientSessionsCommand::Sync);
                let result = match command {
                    ClientSessionsCommand::Sync => sync(&directory, &home, &mut index, &state),
                    ClientSessionsCommand::Open(id) => open(&directory, &index, &id, &state),
                };
                if let Err(error) = result {
                    state.update(|v| {
                        v.error = Some(format!("{error:#}"));
                    });
                }
                if is_sync {
                    next_sync = Instant::now() + Duration::from_secs(CLIENT_SYNC_INTERVAL_SECS);
                    state.update(|v| {
                        v.syncing = false;
                        v.next_sync_unix = Some(now() + CLIENT_SYNC_INTERVAL_SECS);
                    });
                }
            }
        });
        Self {
            shared,
            sender,
            worker: Mutex::new(Some(worker)),
        }
    }

    pub fn shutdown(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        let _ = self.sender.send(Work::Shutdown);
        if let Some(worker) = self
            .worker
            .lock()
            .expect("client session worker lock")
            .take()
        {
            let _ = worker.join();
        }
    }
}

impl ClientSessionsService for ClientSessionStore {
    fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }
    fn snapshot(&self) -> ClientSessionsSnapshot {
        self.shared
            .view
            .lock()
            .expect("client sessions lock")
            .clone()
    }
    fn dispatch(&self, command: ClientSessionsCommand) -> std::result::Result<(), String> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err("Local session sync has stopped.".into());
        }
        let mut view = self.shared.view.lock().expect("client sessions lock");
        match &command {
            ClientSessionsCommand::Sync if view.syncing => return Ok(()),
            ClientSessionsCommand::Sync => view.syncing = true,
            ClientSessionsCommand::Open(id) => {
                view.selected = Some(id.clone());
                view.detail = None;
            }
        }
        view.error = None;
        self.shared.revision.fetch_add(1, Ordering::Release);
        self.sender
            .send(Work::Command(command))
            .map_err(|_| "Local session worker has stopped.".into())
    }
}

impl Drop for ClientSessionStore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn archive_path(directory: &Path, id: &ClientSessionId) -> PathBuf {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(id.0.as_bytes());
    directory.join("archives").join(format!(
        "{}.json",
        crate::installer::digest_hex(digest.as_ref())
    ))
}

fn load_index(directory: &Path) -> Result<Index> {
    let bytes = match fs::read(directory.join("index.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Index::default()),
        Err(error) => return Err(error.into()),
    };
    let index: Index = serde_json::from_slice(&bytes)?;
    ensure!(
        index.version == 1,
        "Unsupported client session index version {}",
        index.version
    );
    for (id, session) in &index.sessions {
        ensure!(*id == session.id().0, "Invalid client session identity");
    }
    Ok(index)
}

fn load_archive(directory: &Path, id: &ClientSessionId) -> Result<Archive> {
    let archive: Archive = serde_json::from_slice(&fs::read(archive_path(directory, id))?)?;
    ensure!(
        archive.version == 1 && archive.session.id() == *id,
        "Unsupported or mismatched client session archive"
    );
    ensure!(
        archive
            .messages
            .iter()
            .all(|m| m.id > 0 && matches!(m.role.as_str(), "user" | "assistant")),
        "Invalid client message"
    );
    ensure!(
        archive.prefix_sha256.len() == 64
            && archive.prefix_sha256.bytes().all(|c| c.is_ascii_hexdigit())
            && archive.offset <= archive.session.stamp.bytes,
        "Invalid native snapshot cursor"
    );
    ensure!(
        archive.session.message_count == archive.messages.len()
            && archive.messages.windows(2).all(|m| m[0].id < m[1].id)
            && archive.messages.iter().all(|m| m.id <= archive.offset),
        "Invalid native snapshot messages"
    );
    Ok(archive)
}

fn open(directory: &Path, index: &Index, id: &ClientSessionId, shared: &Shared) -> Result<()> {
    let session = index.sessions.get(&id.0).context("Unknown local session")?;
    let archive = load_archive(directory, id)?;
    let messages = Arc::new(
        archive
            .messages
            .into_iter()
            .map(|m| ClientMessage {
                id: m.id,
                role: if m.role == "user" {
                    relay_core::agents::MessageRole::User
                } else {
                    relay_core::agents::MessageRole::Assistant
                },
                text: m.text,
            })
            .collect(),
    );
    shared.update(|v| {
        if v.selected.as_ref() == Some(id) {
            v.detail = Some(ClientSessionDetail {
                session: session.view(),
                messages,
            });
        }
    });
    Ok(())
}

fn sync(directory: &Path, home: &Path, index: &mut Index, shared: &Shared) -> Result<()> {
    shared.update(|v| {
        v.syncing = true;
        v.error = None;
        v.scanned_files = 0;
        v.updated_sessions = 0;
        v.failed_files = 0;
    });
    let sources = codex::sources(home, &shared.stopped)?;
    let mut updated = index.clone();
    for session in updated.sessions.values_mut() {
        session.available = false;
    }
    let mut changed = 0;
    let mut failed = 0;
    let mut first_error = None;
    let cached_sources: BTreeMap<_, _> = index
        .sessions
        .values()
        .map(|s| (s.source.clone(), s))
        .collect();
    for (scanned, source) in sources.iter().enumerate() {
        if shared.stopped.load(Ordering::Acquire) {
            bail!("Local session sync stopped");
        }
        let result = (|| -> Result<()> {
            let stamp = codex::FileStamp::read(source)?;
            if let Some(cached) = cached_sources
                .get(source)
                .filter(|s| s.stamp == stamp && archive_path(directory, &s.id()).is_file())
            {
                let mut cached = (*cached).clone();
                cached.available = true;
                if updated
                    .sessions
                    .get(&cached.id().0)
                    .is_some_and(|s| s.available && s.source != *source)
                {
                    return Ok(());
                }
                updated.sessions.insert(cached.id().0, cached);
                return Ok(());
            }
            let metadata = codex::metadata(source, stamp)?;
            let id = metadata.id();
            // Prefer a current source over an archived duplicate with the same native ID.
            if updated
                .sessions
                .get(&id.0)
                .is_some_and(|s| s.available && s.source != *source)
            {
                return Ok(());
            }
            let path = archive_path(directory, &id);
            let previous = if path.try_exists()? {
                Some(load_archive(directory, &id)?)
            } else {
                None
            };
            let (archive, content_changed) =
                codex::read(source, metadata, previous, &shared.stopped)?;
            if content_changed {
                crate::store::write_json(&path, &archive)?;
                changed += 1;
            }
            updated.sessions.insert(id.0, archive.session);
            Ok(())
        })();
        if let Err(error) = result {
            failed += 1;
            first_error.get_or_insert_with(|| format!("{}: {error:#}", source.display()));
            // A transient or malformed source does not erase the last readable snapshot.
            for saved in updated
                .sessions
                .values_mut()
                .filter(|s| s.source == *source)
            {
                saved.available = source.exists();
            }
        }
        if scanned % 32 == 0 {
            shared.update(|v| {
                v.scanned_files = scanned + 1;
                v.updated_sessions = changed;
                v.failed_files = failed;
            });
        }
    }
    updated.last_sync_unix = Some(now());
    crate::store::write_json(&directory.join("index.json"), &updated)?;
    *index = updated;
    shared.publish(index);
    shared.update(|v| {
        v.scanned_files = sources.len();
        v.updated_sessions = changed;
        v.failed_files = failed;
        v.error = first_error;
    });
    let selected = shared
        .view
        .lock()
        .expect("client sessions lock")
        .selected
        .clone();
    if let Some(id) = selected {
        open(directory, index, &id, shared)?;
    }
    Ok(())
}
