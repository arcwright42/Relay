//! A versioned ACP v1 boundary. Protocol types never escape into the desktop.
mod mapping;

use agent_client_protocol::schema::{ProtocolVersion, v1 as acp};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo};
use async_channel::{Receiver, Sender};
use relay_core::agents::{AuthenticationMethod, PermissionRequest, SessionConfig, ToolActivity};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub enum Command {
    Authenticate(String),
    SetConfig {
        id: String,
        value: String,
    },
    Prompt {
        text: String,
        context: Option<String>,
    },
    Cancel,
    Permission {
        id: u64,
        choice: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum Event {
    AuthenticationRequired(Vec<AuthenticationMethod>),
    Ready {
        session_id: String,
        configs: Vec<SessionConfig>,
        resumed: bool,
    },
    Configs {
        configs: Vec<SessionConfig>,
        confirmed: Option<String>,
    },
    Text(String),
    Tool(ToolActivity),
    Permission(PermissionRequest),
    PermissionResolved(u64),
    TurnEnded {
        cancelled: bool,
    },
    Error {
        message: String,
        fatal: bool,
    },
}

pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Default)]
pub struct SessionOptions {
    pub saved_session: Option<String>,
    pub preferences: BTreeMap<String, String>,
}

pub struct ConnectionHandle {
    commands: Sender<Command>,
    stop: Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ConnectionHandle {
    pub fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|_| "The agent connection is closed. Reconnect to continue.".to_owned())
    }
    pub fn stop(&self) {
        let _ = self.stop.try_send(());
    }
    /// Wait for the owned process to exit. Call from a background shutdown worker.
    pub fn shutdown(mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for ConnectionHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts one connection on a dedicated I/O worker. Dropping the handle also stops its process group.
pub fn connect(
    spec: LaunchSpec,
    cwd: PathBuf,
    options: SessionOptions,
    sink: EventSink,
) -> Result<ConnectionHandle, String> {
    let (commands, receiver) = async_channel::unbounded();
    let (stop, stopped) = async_channel::bounded(1);
    let worker = std::thread::Builder::new()
        .name("relay-acp".into())
        .spawn(move || {
            let result = async_io::block_on(futures_lite::future::race(
                run(spec, cwd, options, receiver, sink.clone()),
                async move {
                    let _ = stopped.recv().await;
                    Ok(())
                },
            ));
            if let Err(error) = result {
                sink(Event::Error {
                    message: error.to_string(),
                    fatal: true,
                });
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(ConnectionHandle {
        commands,
        stop,
        worker: Some(worker),
    })
}

type PendingPermissions = Arc<Mutex<BTreeMap<u64, Sender<Option<String>>>>>;

async fn run(
    spec: LaunchSpec,
    cwd: PathBuf,
    options: SessionOptions,
    commands: Receiver<Command>,
    sink: EventSink,
) -> Result<(), acp::Error> {
    let agent = AcpAgent::new(
        AcpAgentConfig::new(spec.command)
            .args(spec.args)
            .envs(spec.env),
    );
    let live = Arc::new(AtomicBool::new(false));
    let pending: PendingPermissions = Arc::default();
    let permission_ids = Arc::new(AtomicU64::new(1));
    let notifications = sink.clone();
    let notification_live = live.clone();
    let permissions = pending.clone();
    let permission_sink = sink.clone();

    Client.builder().name("relay")
        .on_receive_notification(async move |message: acp::SessionNotification, _cx| {
            // Relay owns its visible transcript; session/load replays must not duplicate it.
            if notification_live.load(Ordering::Acquire)
                && let Some(event) = mapping::notification(message.update) {
                    notifications(event);
            }
            Ok(())
        }, agent_client_protocol::on_receive_notification!())
        .on_receive_request(async move |request: acp::RequestPermissionRequest, responder, cx| {
            let id = permission_ids.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = async_channel::bounded(1);
            let choices: Vec<_> = request.options.iter().map(|option| option.option_id.to_string()).collect();
            permissions.lock().expect("permission lock").insert(id, tx);
            permission_sink(Event::Permission(mapping::permission(id, &request)));
            let permissions = permissions.clone();
            let sink = permission_sink.clone();
            // Waiting in the handler itself would block notifications and cancellation.
            cx.spawn(async move {
                let choice = rx.recv().await.ok().flatten().filter(|id| choices.contains(id));
                permissions.lock().expect("permission lock").remove(&id);
                sink(Event::PermissionResolved(id));
                let outcome = choice.map_or(acp::RequestPermissionOutcome::Cancelled, |id| {
                    acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(id))
                });
                responder.respond(acp::RequestPermissionResponse::new(outcome))
            })
        }, agent_client_protocol::on_receive_request!())
        .connect_with(agent, async move |connection: ConnectionTo<Agent>| {
            // The SDK allows a client task to outlive EOF. Relay must end its command loop
            // when the peer exits, otherwise the UI can incorrectly remain connected.
            let transport = connection.clone();
            connection.spawn(async move {
                transport.incoming_closed().await;
                Err(acp::Error::internal_error().data("The Codex connection closed. Reconnect to continue; the last turn may be incomplete."))
            })?;
            let init = timeout(connection.send_request(acp::InitializeRequest::new(ProtocolVersion::V1)
                .client_info(acp::Implementation::new("relay", env!("CARGO_PKG_VERSION"))))
                .block_task(), 30).await?;
            if init.protocol_version != ProtocolVersion::V1 {
                return Err(acp::Error::internal_error().data("This agent did not negotiate ACP v1."));
            }
            let methods: Vec<_> = init.auth_methods.iter().filter_map(|method| {
                if matches!(method, acp::AuthMethod::Agent(_)) {
                    Some(AuthenticationMethod { id: method.id().to_string(), name: method.name().to_owned(), description: method.description().map(str::to_owned) })
                } else { None }
            }).collect();
            let mut session = open_session(&connection, &cwd, &options, init.agent_capabilities.load_session, &sink, &live).await;
            if let Err(error) = &session {
                if error.code == acp::ErrorCode::AuthRequired { sink(Event::AuthenticationRequired(methods.clone())); }
                else { return Err(error.clone()); }
            }
            let running = Arc::new(AtomicBool::new(false));
            let turn = Arc::new(AtomicU64::new(0));
            while let Ok(command) = commands.recv().await {
                match command {
                    Command::Authenticate(method) => {
                        if !methods.iter().any(|m| m.id == method) { continue; }
                        match timeout(connection.send_request(acp::AuthenticateRequest::new(method)).block_task(), 300).await {
                            Ok(_) => {
                                session = open_session(&connection, &cwd, &options, init.agent_capabilities.load_session, &sink, &live).await;
                                if let Err(error) = &session { sink(Event::Error { message: error.to_string(), fatal: false }); sink(Event::AuthenticationRequired(methods.clone())); }
                            }
                            Err(error) => { sink(Event::Error { message: error.to_string(), fatal: false }); sink(Event::AuthenticationRequired(methods.clone())); }
                        }
                    }
                    Command::Permission { id, choice } => {
                        if let Some(sender) = pending.lock().expect("permission lock").remove(&id) { let _ = sender.try_send(choice); }
                    }
                    Command::SetConfig { id, value } => {
                        let Ok(session_id) = &session else { continue; };
                        if running.load(Ordering::Acquire) { continue; }
                        match timeout(connection.send_request(acp::SetSessionConfigOptionRequest::new(session_id.clone(), id.clone(), acp::SessionConfigOptionValue::value_id(value))).block_task(), 20).await {
                            Ok(response) => sink(Event::Configs { configs: mapping::configs(&response.config_options), confirmed: Some(id) }),
                            Err(error) => sink(Event::Error { message: error.to_string(), fatal: false }),
                        }
                    }
                    Command::Prompt { text, context } => {
                        let Ok(session_id) = &session else { continue; };
                        if running.swap(true, Ordering::AcqRel) { continue; }
                        turn.fetch_add(1, Ordering::AcqRel);
                        let mut content = Vec::new();
                        if let Some(context) = context { content.push(acp::ContentBlock::Text(acp::TextContent::new(context))); }
                        content.push(acp::ContentBlock::Text(acp::TextContent::new(text)));
                        let request = acp::PromptRequest::new(session_id.clone(), content);
                        let connection = connection.clone();
                        let running = running.clone();
                        let sink = sink.clone();
                        let pending = pending.clone();
                        connection.clone().spawn(async move {
                            let result = connection.send_request(request).block_task().await;
                            running.store(false, Ordering::Release);
                            cancel_permissions(&pending);
                            match result {
                                Ok(response) => sink(Event::TurnEnded { cancelled: response.stop_reason == acp::StopReason::Cancelled }),
                                Err(error) => { sink(Event::Error { message: error.to_string(), fatal: false }); sink(Event::TurnEnded { cancelled: true }); }
                            }
                            Ok(())
                        })?;
                    }
                    Command::Cancel => {
                        cancel_permissions(&pending);
                        if let Ok(id) = &session {
                            connection.send_notification(acp::CancelNotification::new(id.clone()))?;
                            let running = running.clone();
                            let turn = turn.clone();
                            let current_turn = turn.load(Ordering::Acquire);
                            connection.spawn(async move {
                                async_io::Timer::after(Duration::from_secs(10)).await;
                                if running.load(Ordering::Acquire) && turn.load(Ordering::Acquire) == current_turn {
                                    return Err(acp::Error::internal_error().data("Codex did not confirm cancellation. The connection was stopped; the last turn may be incomplete."));
                                }
                                Ok(())
                            })?;
                        }
                    }
                }
            }
            Ok(())
        }).await
}

fn cancel_permissions(pending: &PendingPermissions) {
    for (_, sender) in std::mem::take(&mut *pending.lock().expect("permission lock")) {
        let _ = sender.try_send(None);
    }
}

async fn open_session(
    connection: &ConnectionTo<Agent>,
    cwd: &std::path::Path,
    options: &SessionOptions,
    supports_load: bool,
    sink: &EventSink,
    live: &AtomicBool,
) -> Result<acp::SessionId, acp::Error> {
    live.store(false, Ordering::Release);
    if let Some(saved) = options.saved_session.as_deref().filter(|_| supports_load) {
        match timeout(
            connection
                .send_request(acp::LoadSessionRequest::new(saved.to_owned(), cwd))
                .block_task(),
            45,
        )
        .await
        {
            Ok(response) => {
                let configs = restore_preferences(
                    connection,
                    saved,
                    mapping::configs(&response.config_options.unwrap_or_default()),
                    &options.preferences,
                    sink,
                )
                .await;
                sink(Event::Ready {
                    session_id: saved.to_owned(),
                    configs,
                    resumed: true,
                });
                live.store(true, Ordering::Release);
                return Ok(acp::SessionId::new(saved.to_owned()));
            }
            Err(error) if error.code == acp::ErrorCode::AuthRequired => return Err(error),
            Err(_) => {} // Restore from Relay's own transcript if native history is unavailable.
        }
    }
    let response = timeout(
        connection
            .send_request(acp::NewSessionRequest::new(cwd))
            .block_task(),
        45,
    )
    .await?;
    let configs = restore_preferences(
        connection,
        &response.session_id.to_string(),
        mapping::configs(&response.config_options.unwrap_or_default()),
        &options.preferences,
        sink,
    )
    .await;
    sink(Event::Ready {
        session_id: response.session_id.to_string(),
        configs,
        resumed: false,
    });
    live.store(true, Ordering::Release);
    Ok(response.session_id)
}

async fn restore_preferences(
    connection: &ConnectionTo<Agent>,
    session: &str,
    mut configs: Vec<SessionConfig>,
    preferences: &BTreeMap<String, String>,
    sink: &EventSink,
) -> Vec<SessionConfig> {
    for (id, value) in preferences {
        // A saved ID is only valid if the current agent still advertises it.
        if !configs.iter().any(|config| {
            config.id == *id
                && config.current != *value
                && config.choices.iter().any(|choice| choice.id == *value)
        }) {
            continue;
        }
        match timeout(
            connection
                .send_request(acp::SetSessionConfigOptionRequest::new(
                    session.to_owned(),
                    id.clone(),
                    acp::SessionConfigOptionValue::value_id(value.clone()),
                ))
                .block_task(),
            20,
        )
        .await
        {
            Ok(response) => configs = mapping::configs(&response.config_options),
            Err(error) => sink(Event::Error {
                message: format!("Could not restore {id}: {error}"),
                fatal: false,
            }),
        }
    }
    configs
}

async fn timeout<T>(
    future: impl Future<Output = Result<T, acp::Error>>,
    seconds: u64,
) -> Result<T, acp::Error> {
    futures_lite::future::race(future, async move {
        async_io::Timer::after(Duration::from_secs(seconds)).await;
        Err(acp::Error::internal_error()
            .data("The agent request timed out. Please reconnect and try again."))
    })
    .await
}
