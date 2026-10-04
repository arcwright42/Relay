//! Main-room voice turns with correlated replies; no audio, UI or speech SDKs.
use relay_core::{ThreadId, agents::*, threads::*, voice::*};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct ThreadVoiceDialogue {
    threads: Arc<dyn ThreadService>,
    agents: Arc<dyn AgentService>,
}
impl ThreadVoiceDialogue {
    pub fn new(threads: Arc<dyn ThreadService>, agents: Arc<dyn AgentService>) -> Self {
        Self { threads, agents }
    }

    fn ready(&self, thread: ThreadId, cancelled: &AtomicBool) -> Result<(), VoiceTurnError> {
        check_cancelled(cancelled)?;
        let state = self.agents.snapshot(thread);
        if matches!(
            state.status,
            ConnectionStatus::Running | ConnectionStatus::Cancelling
        ) {
            return Err(VoiceTurnError::AgentBusy);
        }
        if matches!(
            state.status,
            ConnectionStatus::Disconnected | ConnectionStatus::Failed
        ) {
            self.agents
                .dispatch(thread, AgentCommand::Connect(state.source))
                .map_err(VoiceTurnError::Agent)?;
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            check_cancelled(cancelled)?;
            let state = self.agents.snapshot(thread);
            match state.status {
                ConnectionStatus::Ready if state.pending_config.is_none() => return Ok(()),
                ConnectionStatus::Running | ConnectionStatus::Cancelling => {
                    return Err(VoiceTurnError::AgentBusy);
                }
                ConnectionStatus::NeedsAuthentication | ConnectionStatus::Authenticating => {
                    return Err(VoiceTurnError::AgentAuthentication);
                }
                ConnectionStatus::Disconnected | ConnectionStatus::Failed => {
                    return Err(VoiceTurnError::Agent(
                        state
                            .error
                            .unwrap_or_else(|| "The thread agent is unavailable.".into()),
                    ));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err(VoiceTurnError::Agent(
                    "The thread agent did not become ready in time.".into(),
                ));
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), VoiceTurnError> {
    if cancelled.load(Ordering::Acquire) {
        Err(VoiceTurnError::Cancelled)
    } else {
        Ok(())
    }
}

struct OwnedTurn<'a> {
    agents: &'a dyn AgentService,
    thread: ThreadId,
    response_id: u64,
    complete: bool,
}
impl Drop for OwnedTurn<'_> {
    fn drop(&mut self) {
        if !self.complete {
            let _ = self.agents.dispatch(
                self.thread,
                AgentCommand::CancelTurn {
                    response_id: self.response_id,
                },
            );
        }
    }
}

impl VoicePromptService for ThreadVoiceDialogue {
    fn respond(
        &self,
        prompt: &str,
        cancelled: &AtomicBool,
        progress: &dyn Fn(VoicePromptProgress),
    ) -> Result<VoicePromptResult, VoiceTurnError> {
        check_cancelled(cancelled)?;
        let thread = crate::native::MAIN;
        let definition = self
            .threads
            .thread(thread)
            .ok_or(VoiceTurnError::ThreadUnavailable)?;
        progress(VoicePromptProgress::Thread(VoiceThread {
            id: thread,
            name: definition.name,
        }));
        progress(VoicePromptProgress::Stage(VoiceTurnStage::Connecting));
        self.ready(thread, cancelled)?;
        check_cancelled(cancelled)?;
        let response_id = self
            .agents
            .send_turn(thread, prompt.to_owned())
            .map_err(VoiceTurnError::Agent)?;
        let mut owned = OwnedTurn {
            agents: &*self.agents,
            thread,
            response_id,
            complete: false,
        };
        let mut prior_text = String::new();
        loop {
            check_cancelled(cancelled)?;
            let state = self.agents.snapshot(thread);
            progress(VoicePromptProgress::Stage(
                if state.permissions.is_empty() {
                    VoiceTurnStage::WaitingForAgent
                } else {
                    VoiceTurnStage::NeedsAttention
                },
            ));
            let response = state
                .messages
                .iter()
                .find(|message| message.id == response_id)
                .ok_or_else(|| {
                    VoiceTurnError::Agent(
                        "The thread conversation changed before the voice reply completed.".into(),
                    )
                })?;
            if response.text != prior_text {
                prior_text.clone_from(&response.text);
                progress(VoicePromptProgress::Response(
                    response.text.chars().take(32_000).collect(),
                ));
            }
            match response.status {
                MessageStatus::Complete => {
                    owned.complete = true;
                    if matches!(
                        response
                            .metrics
                            .as_ref()
                            .and_then(|metrics| metrics.outcome),
                        Some(TurnOutcome::Cancelled | TurnOutcome::Failed)
                    ) {
                        return Err(VoiceTurnError::Agent(
                            "The voice turn did not finish successfully.".into(),
                        ));
                    }
                    return Ok(VoicePromptResult::Reply(response.text.clone()));
                }
                MessageStatus::Interrupted => {
                    owned.complete = true;
                    return Err(VoiceTurnError::Agent(
                        state
                            .error
                            .unwrap_or_else(|| "The voice turn was interrupted.".into()),
                    ));
                }
                MessageStatus::Streaming => {}
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
}
