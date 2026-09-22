//! Agent orchestration and storage, with no dependency on GPUI.
mod installer;
mod settings;
mod store;

pub use settings::SettingsStore;

use installer::{Installer, RELEASE};
use relay_acp::{Command as AcpCommand, ConnectionHandle, Event};
use relay_core::{ProjectId, agents::*};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

struct ProjectState {
    view: AgentSnapshot,
    generation: u64,
    session_id: Option<String>,
    session_key: Option<String>,
    preferences: BTreeMap<String, String>,
    needs_context: bool,
    last_checkpoint: Instant,
    storage_error: bool,
}

impl ProjectState {
    fn saved(&self) -> store::SavedProject {
        store::SavedProject {
            version: 1,
            local_codex: match &self.view.source {
                AgentSource::Managed => None,
                AgentSource::Local(path) => Some(path.clone()),
            },
            cwd: Some(self.view.working_directory.clone()),
            session_id: self.session_id.clone(),
            session_key: self.session_key.clone(),
            preferences: self.preferences.clone(),
            messages: self
                .view
                .messages
                .iter()
                .map(store::SavedMessage::from_message)
                .collect(),
        }
    }
}

struct Shared {
    projects: Mutex<BTreeMap<ProjectId, ProjectState>>,
    root: PathBuf,
    revision: AtomicU64,
    shutdown: AtomicBool,
    // Serializes file replacement without holding a UI snapshot lock during disk I/O.
    persistence: Mutex<()>,
}

impl Shared {
    fn valid(&self, project: ProjectId, generation: u64) -> bool {
        !self.shutdown.load(Ordering::Acquire)
            && self
                .projects
                .lock()
                .expect("project lock")
                .get(&project)
                .is_some_and(|p| p.generation == generation)
    }

    fn update(
        &self,
        project: ProjectId,
        generation: u64,
        f: impl FnOnce(&mut ProjectState),
    ) -> bool {
        let mut projects = self.projects.lock().expect("project lock");
        if self.shutdown.load(Ordering::Acquire) {
            return false;
        }
        let Some(state) = projects
            .get_mut(&project)
            .filter(|p| p.generation == generation)
        else {
            return false;
        };
        f(state);
        self.revision.fetch_add(1, Ordering::Release);
        true
    }

    fn persist(&self, project: ProjectId) {
        let _write = self.persistence.lock().expect("persistence lock");
        let saved = {
            let projects = self.projects.lock().expect("project lock");
            let Some(state) = projects.get(&project) else {
                return;
            };
            if state.storage_error {
                return;
            } // Never overwrite a file we could not read.
            state.saved()
        };
        if let Err(error) = store::save(&self.root, project, &saved) {
            if let Some(state) = self
                .projects
                .lock()
                .expect("project lock")
                .get_mut(&project)
            {
                state.view.error = Some(format!("Could not save this conversation: {error}"));
            }
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    fn event(&self, project: ProjectId, generation: u64, event: Event) {
        let mut persist = false;
        if !self.update(project, generation, |state| match event {
            Event::AuthenticationRequired(methods) => {
                state.view.status = ConnectionStatus::NeedsAuthentication;
                state.view.auth_methods = methods;
            }
            Event::Ready {
                session_id,
                configs,
                resumed,
            } => {
                state.view.status = ConnectionStatus::Ready;
                state.view.configs = configs;
                state.view.auth_methods.clear();
                state.session_id = Some(session_id);
                state.session_key = Some(session_key(&state.view));
                state.needs_context = !resumed && !state.view.messages.is_empty();
                persist = true;
            }
            Event::Configs { configs, confirmed } => {
                if let Some(id) = confirmed
                    && state.view.pending_config.as_ref() == Some(&id)
                {
                    state.view.pending_config = None;
                    if let Some(config) = configs.iter().find(|c| c.id == id) {
                        state.preferences.insert(id, config.current.clone());
                    }
                }
                state.view.configs = configs;
                persist = true;
            }
            Event::Text(text) => {
                if let Some(message) = streaming_message(state) {
                    message.text.push_str(&text);
                }
                if state.last_checkpoint.elapsed() > Duration::from_secs(1) {
                    state.last_checkpoint = Instant::now();
                    persist = true;
                }
            }
            Event::Tool(tool) => {
                if let Some(message) = streaming_message(state) {
                    if let Some(existing) = message.tools.iter_mut().find(|t| t.id == tool.id) {
                        if !tool.title.is_empty() {
                            existing.title = tool.title;
                        }
                        if !tool.status.is_empty() {
                            existing.status = tool.status;
                        }
                    } else {
                        message.tools.push(tool);
                    }
                }
            }
            Event::Permission(permission) => state.view.permissions.push(permission),
            Event::PermissionResolved(id) => state.view.permissions.retain(|p| p.id != id),
            Event::TurnEnded { cancelled } => {
                if let Some(message) = streaming_message(state) {
                    message.status = if cancelled {
                        MessageStatus::Interrupted
                    } else {
                        MessageStatus::Complete
                    };
                }
                state.view.permissions.clear();
                state.view.status = ConnectionStatus::Ready;
                persist = true;
            }
            Event::Error { message, fatal } => {
                state.view.error = Some(message);
                state.view.pending_config = None;
                if fatal {
                    state.view.status = ConnectionStatus::Failed;
                    state.view.permissions.clear();
                    if let Some(message) = streaming_message(state) {
                        message.status = MessageStatus::Interrupted;
                    }
                }
                persist = true;
            }
        }) {
            return;
        }
        if persist {
            self.persist(project);
        }
    }
}

fn streaming_message(state: &mut ProjectState) -> Option<&mut ChatMessage> {
    state
        .view
        .messages
        .last_mut()
        .filter(|m| m.role == MessageRole::Assistant && m.status == MessageStatus::Streaming)
}

type Connections = Arc<Mutex<BTreeMap<ProjectId, ConnectionHandle>>>;

pub struct AgentRuntime {
    shared: Arc<Shared>,
    connections: Connections,
    installer: Arc<Installer>,
    preparations: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl AgentRuntime {
    pub fn default_directory() -> PathBuf {
        if let Some(directory) = std::env::var_os("RELAY_DATA_DIR") {
            return PathBuf::from(directory);
        }
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/Relay")
    }

    pub fn new(root: PathBuf, project_ids: impl IntoIterator<Item = ProjectId>) -> Self {
        let installer = Arc::new(Installer::new(root.clone()));
        let installed = installer.installed();
        let mut projects = BTreeMap::new();
        for id in project_ids {
            let loaded = store::load(&root, id);
            let error = loaded.as_ref().err().map(|e| format!("Could not read this project's saved conversation: {e}. The existing file has been preserved."));
            let saved = loaded.unwrap_or_default();
            let view = AgentSnapshot {
                source: saved
                    .local_codex
                    .map_or(AgentSource::Managed, AgentSource::Local),
                installed,
                working_directory: saved.cwd.unwrap_or_else(|| {
                    root.join("projects")
                        .join(id.0.to_string())
                        .join("workspace")
                }),
                messages: saved
                    .messages
                    .into_iter()
                    .map(store::SavedMessage::into_message)
                    .collect(),
                error: error.clone(),
                runtime_version: None,
                ..Default::default()
            };
            projects.insert(
                id,
                ProjectState {
                    view,
                    generation: 0,
                    session_id: saved.session_id,
                    session_key: saved.session_key,
                    preferences: saved.preferences,
                    needs_context: false,
                    last_checkpoint: Instant::now(),
                    storage_error: error.is_some(),
                },
            );
        }
        Self {
            shared: Arc::new(Shared {
                root,
                projects: Mutex::new(projects),
                revision: AtomicU64::new(1),
                shutdown: AtomicBool::new(false),
                persistence: Mutex::new(()),
            }),
            connections: Arc::default(),
            installer,
            preparations: Mutex::new(Vec::new()),
        }
    }

    fn connect(&self, project: ProjectId, source: AgentSource) -> Result<(), String> {
        let (generation, cwd, saved, preferences) = {
            let mut projects = self.shared.projects.lock().expect("project lock");
            let state = projects.get_mut(&project).ok_or("Unknown project")?;
            if state.storage_error {
                return Err("The saved conversation could not be read. Resolve the storage error before connecting.".into());
            }
            if state.view.status.is_busy() {
                return Err("Wait for the current operation to finish, or stop it first.".into());
            }
            state.generation += 1;
            state.view.source = source.clone();
            let saved = (state.session_key.as_ref() == Some(&session_key(&state.view)))
                .then(|| state.session_id.clone())
                .flatten();
            state.view.status = ConnectionStatus::Preparing("Preparing Codex…".into());
            state.view.error = None;
            state.view.configs.clear();
            state.view.permissions.clear();
            state.view.pending_config = None;
            (
                state.generation,
                state.view.working_directory.clone(),
                saved,
                state.preferences.clone(),
            )
        };
        self.shared.revision.fetch_add(1, Ordering::Release);
        self.connections
            .lock()
            .expect("connection lock")
            .remove(&project);
        let shared = self.shared.clone();
        let installer = self.installer.clone();
        let connections = self.connections.clone();
        let worker = std::thread::Builder::new()
            .name("relay-prepare".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    std::fs::create_dir_all(&cwd)?;
                    let prepared = installer.prepare(
                        &source,
                        |step| {
                            shared.update(project, generation, |s| {
                                s.view.status = ConnectionStatus::Preparing(step.into())
                            });
                        },
                        || !shared.valid(project, generation),
                    )?;
                    if !shared.valid(project, generation) {
                        return Ok(());
                    }
                    shared.update(project, generation, |s| {
                        s.view.status = ConnectionStatus::Connecting;
                        s.view.installed = true;
                        s.view.runtime_version = Some(prepared.version);
                    });
                    let callback_shared = shared.clone();
                    let handle = relay_acp::connect(
                        prepared.launch,
                        cwd,
                        relay_acp::SessionOptions {
                            saved_session: saved,
                            preferences,
                        },
                        Arc::new(move |event| callback_shared.event(project, generation, event)),
                    )
                    .map_err(anyhow::Error::msg)?;
                    let mut handle = Some(handle);
                    {
                        // Lock order is always project state, then connection handles.
                        let projects = shared.projects.lock().expect("project lock");
                        if !shared.shutdown.load(Ordering::Acquire)
                            && projects
                                .get(&project)
                                .is_some_and(|p| p.generation == generation)
                        {
                            connections
                                .lock()
                                .expect("connection lock")
                                .insert(project, handle.take().expect("new connection"));
                        }
                    }
                    if let Some(handle) = handle {
                        handle.shutdown();
                    }
                    shared.persist(project);
                    Ok(())
                })();
                if let Err(error) = result
                    && shared.valid(project, generation)
                {
                    shared.event(
                        project,
                        generation,
                        Event::Error {
                            message: format!("{error:#}"),
                            fatal: true,
                        },
                    );
                }
            })
            .map_err(|e| e.to_string())?;
        let mut preparations = self.preparations.lock().expect("preparation lock");
        preparations.retain(|worker| !worker.is_finished());
        preparations.push(worker);
        Ok(())
    }

    /// Flush project history and wait for child processes to exit, outside the UI thread.
    pub fn shutdown(&self) {
        if self.shared.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }
        let connections = std::mem::take(&mut *self.connections.lock().expect("connection lock"));
        for connection in connections.values() {
            connection.stop();
        }
        // Persist before waiting for installer cleanup: the native application has a
        // bounded quit grace period, and a large staging directory may take longer.
        let ids: Vec<_> = {
            let mut projects = self.shared.projects.lock().expect("project lock");
            for state in projects.values_mut() {
                if let Some(message) = streaming_message(state) {
                    message.status = MessageStatus::Interrupted;
                }
            }
            projects.keys().copied().collect()
        };
        for project in ids {
            self.shared.persist(project);
        }
        for (_, connection) in connections {
            connection.shutdown();
        }
        let preparations =
            std::mem::take(&mut *self.preparations.lock().expect("preparation lock"));
        for worker in preparations {
            let _ = worker.join();
        }
    }

    fn send_command(&self, project: ProjectId, command: AcpCommand) -> Result<(), String> {
        self.connections
            .lock()
            .expect("connection lock")
            .get(&project)
            .ok_or("Connect Codex first.")?
            .send(command)
    }
}

impl AgentService for AgentRuntime {
    fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }
    fn snapshot(&self, project: ProjectId) -> AgentSnapshot {
        self.shared
            .projects
            .lock()
            .expect("project lock")
            .get(&project)
            .map(|p| p.view.clone())
            .unwrap_or_default()
    }

    fn dispatch(&self, project: ProjectId, command: AgentCommand) -> Result<(), String> {
        if let AgentCommand::Connect(source) = command {
            return self.connect(project, source);
        }
        if matches!(command, AgentCommand::DiscoverLocal) {
            let shared = self.shared.clone();
            {
                let mut projects = shared.projects.lock().expect("project lock");
                let state = projects.get_mut(&project).ok_or("Unknown project")?;
                if state.view.discovering {
                    return Ok(());
                }
                state.view.discovering = true;
            }
            shared.revision.fetch_add(1, Ordering::Release);
            std::thread::spawn(move || {
                let paths = installer::discover_local();
                let mut projects = shared.projects.lock().expect("project lock");
                if let Some(state) = projects.get_mut(&project) {
                    state.view.local_installations = paths;
                    state.view.discovering = false;
                    shared.revision.fetch_add(1, Ordering::Release);
                }
            });
            return Ok(());
        }
        let mut persist = false;
        let mut projects = self.shared.projects.lock().expect("project lock");
        let state = projects.get_mut(&project).ok_or("Unknown project")?;
        if state.storage_error {
            return Err(
                "Resolve the project storage error before changing this conversation.".into(),
            );
        }
        match command {
            AgentCommand::Disconnect => {
                state.generation += 1;
                state.view.status = ConnectionStatus::Disconnected;
                state.view.permissions.clear();
                state.view.pending_config = None;
                if let Some(message) = streaming_message(state) {
                    message.status = MessageStatus::Interrupted;
                }
                self.connections
                    .lock()
                    .expect("connection lock")
                    .remove(&project);
                persist = true;
            }
            AgentCommand::Authenticate(method) => {
                if state.view.status != ConnectionStatus::NeedsAuthentication {
                    return Err("This connection is not waiting for sign-in.".into());
                }
                self.send_command(project, AcpCommand::Authenticate(method))?;
                state.view.status = ConnectionStatus::Authenticating;
                state.view.error = None;
            }
            AgentCommand::SetConfig { id, value } => {
                if state.view.status != ConnectionStatus::Ready
                    || state.view.pending_config.is_some()
                {
                    return Err(
                        "Wait for the current operation to finish before changing the model."
                            .into(),
                    );
                }
                if !state
                    .view
                    .configs
                    .iter()
                    .any(|c| c.id == id && c.choices.iter().any(|v| v.id == value))
                {
                    return Err(
                        "This option is no longer offered by Codex. Reconnect to refresh it."
                            .into(),
                    );
                }
                self.send_command(
                    project,
                    AcpCommand::SetConfig {
                        id: id.clone(),
                        value,
                    },
                )?;
                state.view.pending_config = Some(id);
                state.view.error = None;
            }
            AgentCommand::Send(text) => {
                if state.view.status != ConnectionStatus::Ready
                    || state.view.pending_config.is_some()
                {
                    return Err("Connect Codex and wait until it is ready before sending.".into());
                }
                let text = text.trim().to_owned();
                if text.is_empty() {
                    return Ok(());
                }
                let context = state
                    .needs_context
                    .then(|| history_context(&state.view.messages));
                self.send_command(
                    project,
                    AcpCommand::Prompt {
                        text: text.clone(),
                        context,
                    },
                )?;
                state.needs_context = false;
                let id = state.view.messages.last().map_or(1, |m| m.id + 1);
                state.view.messages.push(ChatMessage {
                    id,
                    role: MessageRole::User,
                    text,
                    status: MessageStatus::Complete,
                    tools: vec![],
                });
                state.view.messages.push(ChatMessage {
                    id: id + 1,
                    role: MessageRole::Assistant,
                    text: String::new(),
                    status: MessageStatus::Streaming,
                    tools: vec![],
                });
                state.view.status = ConnectionStatus::Running;
                state.view.error = None;
                persist = true;
            }
            AgentCommand::Cancel => {
                if state.view.status == ConnectionStatus::Running {
                    self.send_command(project, AcpCommand::Cancel)?;
                    state.view.status = ConnectionStatus::Cancelling;
                }
            }
            AgentCommand::AnswerPermission { id, choice } => {
                if !state.view.permissions.iter().any(|p| {
                    p.id == id
                        && choice
                            .as_ref()
                            .is_none_or(|c| p.choices.iter().any(|v| &v.id == c))
                }) {
                    return Err("That permission request is no longer active.".into());
                }
                self.send_command(project, AcpCommand::Permission { id, choice })?;
            }
            AgentCommand::SetWorkingDirectory(path) => {
                if state.view.status.is_busy() {
                    return Err(
                        "Stop the current operation before changing the working folder.".into(),
                    );
                }
                let path = path.canonicalize().map_err(|e| e.to_string())?;
                if !path.is_dir() {
                    return Err("Choose an existing folder.".into());
                }
                state.generation += 1;
                state.view.working_directory = path;
                state.view.status = ConnectionStatus::Disconnected;
                state.session_id = None;
                state.session_key = None;
                state.view.configs.clear();
                state.view.permissions.clear();
                state.view.pending_config = None;
                self.connections
                    .lock()
                    .expect("connection lock")
                    .remove(&project);
                persist = true;
            }
            AgentCommand::Connect(_) | AgentCommand::DiscoverLocal => unreachable!(),
        }
        drop(projects);
        self.shared.revision.fetch_add(1, Ordering::Release);
        if persist {
            let shared = self.shared.clone();
            std::thread::spawn(move || shared.persist(project));
        }
        Ok(())
    }
}

impl Drop for AgentRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn session_key(view: &AgentSnapshot) -> String {
    format!(
        "{:?}|{}|{}",
        view.source,
        view.working_directory.display(),
        RELEASE
    )
}

fn history_context(messages: &[ChatMessage]) -> String {
    let mut history = Vec::new();
    let mut remaining = 24_000;
    for message in messages.iter().rev() {
        if message.text.is_empty() {
            continue;
        }
        let text: String = message.text.chars().take(remaining).collect();
        remaining = remaining.saturating_sub(text.chars().count());
        history.push(format!(
            "{}: {}",
            if message.role == MessageRole::User {
                "User"
            } else {
                "Assistant"
            },
            text
        ));
        if remaining == 0 {
            break;
        }
    }
    history.reverse();
    format!(
        "Relay restored this project's recent visible conversation because the previous agent session could not be reused. Treat this as historical context; answer the new user message that follows. Earlier material may be omitted.\n\n{}",
        history.join("\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime() -> AgentRuntime {
        AgentRuntime::new(
            std::env::temp_dir().join(format!("relay-state-test-{}", installer::unique_id())),
            [ProjectId(1), ProjectId(2)],
        )
    }
    #[test]
    fn late_events_cannot_mutate_a_replaced_session_or_another_project() {
        let runtime = runtime();
        runtime
            .shared
            .projects
            .lock()
            .unwrap()
            .get_mut(&ProjectId(1))
            .unwrap()
            .generation = 2;
        runtime.shared.event(
            ProjectId(1),
            1,
            Event::Error {
                message: "old connection".into(),
                fatal: true,
            },
        );
        assert!(runtime.snapshot(ProjectId(1)).error.is_none());
        assert!(runtime.snapshot(ProjectId(2)).error.is_none());
    }
    #[test]
    fn model_choice_is_not_optimistically_committed() {
        let runtime = runtime();
        assert!(
            runtime
                .dispatch(
                    ProjectId(1),
                    AgentCommand::SetConfig {
                        id: "model".into(),
                        value: "invented".into()
                    }
                )
                .is_err()
        );
        assert!(runtime.snapshot(ProjectId(1)).model().is_none());
    }

    #[test]
    fn projects_restore_independently_and_unfinished_turns_remain_interrupted() {
        let runtime = runtime();
        let root = runtime.shared.root.clone();
        for (project, text, status) in [
            (ProjectId(1), "Partial", MessageStatus::Streaming),
            (ProjectId(2), "Other project", MessageStatus::Complete),
        ] {
            runtime.shared.update(project, 0, |state| {
                state.view.messages.push(ChatMessage {
                    id: 1,
                    role: MessageRole::Assistant,
                    text: text.into(),
                    status,
                    tools: vec![],
                })
            });
        }
        drop(runtime);
        let restored = AgentRuntime::new(root.clone(), [ProjectId(1), ProjectId(2)]);
        assert_eq!(restored.snapshot(ProjectId(1)).messages[0].text, "Partial");
        assert_eq!(
            restored.snapshot(ProjectId(1)).messages[0].status,
            MessageStatus::Interrupted
        );
        assert_eq!(
            restored.snapshot(ProjectId(2)).messages[0].text,
            "Other project"
        );
        assert_eq!(
            restored.snapshot(ProjectId(2)).messages[0].status,
            MessageStatus::Complete
        );
        drop(restored);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_conversation_is_preserved_and_cannot_be_overwritten() {
        let runtime = runtime();
        let root = runtime.shared.root.clone();
        drop(runtime);
        let path = root.join("projects/1/conversation.json");
        std::fs::write(&path, "damaged original").unwrap();
        let restored = AgentRuntime::new(root.clone(), [ProjectId(1)]);
        assert!(restored.snapshot(ProjectId(1)).error.is_some());
        assert!(
            restored
                .dispatch(ProjectId(1), AgentCommand::Connect(AgentSource::Managed))
                .is_err()
        );
        drop(restored);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "damaged original");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unrelated_config_notifications_do_not_confirm_a_pending_selection() {
        let runtime = runtime();
        runtime.shared.update(ProjectId(1), 0, |state| {
            state.view.pending_config = Some("model".into())
        });
        runtime.shared.event(
            ProjectId(1),
            0,
            Event::Configs {
                configs: vec![],
                confirmed: None,
            },
        );
        assert_eq!(
            runtime.snapshot(ProjectId(1)).pending_config.as_deref(),
            Some("model")
        );
        runtime.shared.event(
            ProjectId(1),
            0,
            Event::Error {
                message: "Unavailable".into(),
                fatal: false,
            },
        );
        assert!(runtime.snapshot(ProjectId(1)).pending_config.is_none());
        assert!(
            runtime.shared.projects.lock().unwrap()[&ProjectId(1)]
                .preferences
                .is_empty()
        );
    }
}
