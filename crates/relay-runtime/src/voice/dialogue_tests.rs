use super::dialogue::ThreadVoiceDialogue;
use crate::{installer, resident::ResidentStore};
use relay_core::{ThreadId, agents::*, voice::*};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Agents {
    view: Mutex<AgentSnapshot>,
    commands: Mutex<Vec<AgentCommand>>,
    sends: Mutex<Vec<(ThreadId, String)>>,
}
impl Agents {
    fn new() -> Self {
        Self {
            view: Mutex::new(AgentSnapshot {
                status: ConnectionStatus::Ready,
                ..Default::default()
            }),
            commands: Mutex::default(),
            sends: Mutex::default(),
        }
    }
}
impl AgentService for Agents {
    fn revision(&self) -> u64 {
        0
    }
    fn snapshot(&self, _: ThreadId) -> AgentSnapshot {
        self.view.lock().unwrap().clone()
    }
    fn dispatch(&self, _: ThreadId, command: AgentCommand) -> Result<(), String> {
        self.commands.lock().unwrap().push(command);
        Ok(())
    }
    fn send_turn(&self, thread: ThreadId, prompt: String) -> Result<u64, String> {
        self.sends.lock().unwrap().push((thread, prompt));
        let mut view = self.view.lock().unwrap();
        view.status = ConnectionStatus::Running;
        view.messages = vec![message(41, "voice reply", MessageStatus::Streaming)];
        Ok(41)
    }
}
fn message(id: u64, text: &str, status: MessageStatus) -> ChatMessage {
    ChatMessage {
        id,
        role: MessageRole::Assistant,
        text: text.into(),
        status,
        tools: vec![],
        metrics: None,
    }
}
struct Fixture {
    root: std::path::PathBuf,
    threads: Arc<ResidentStore>,
    agents: Arc<Agents>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("relay-voice-main-{}", installer::unique_id()));
        let threads = Arc::new(ResidentStore::open(root.clone()).unwrap());
        Self {
            root,
            threads,
            agents: Arc::new(Agents::new()),
        }
    }
    fn dialogue(&self) -> ThreadVoiceDialogue {
        ThreadVoiceDialogue::new(self.threads.clone(), self.agents.clone())
    }
    fn complete(&self, progress: VoicePromptProgress) {
        if matches!(
            progress,
            VoicePromptProgress::Stage(VoiceTurnStage::WaitingForAgent)
        ) {
            let mut view = self.agents.view.lock().unwrap();
            view.messages[0].status = MessageStatus::Complete;
            view.messages.push(message(
                43,
                "a later unrelated reply",
                MessageStatus::Streaming,
            ));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn routed_prompt_uses_the_existing_thread_session_and_only_its_correlated_reply() {
    let fixture = Fixture::new();
    let result = fixture
        .dialogue()
        .respond("summarize", &AtomicBool::new(false), &|progress| {
            fixture.complete(progress)
        })
        .unwrap();
    let VoicePromptResult::Reply(text) = result;
    assert_eq!(text, "voice reply");
    assert_eq!(
        *fixture.agents.sends.lock().unwrap(),
        [(ThreadId(0), "summarize".into())]
    );
    assert!(fixture.agents.commands.lock().unwrap().is_empty());
}

#[test]
fn changed_catalog_and_busy_or_unauthenticated_agents_never_receive_the_prompt() {
    for status in [
        ConnectionStatus::Running,
        ConnectionStatus::Cancelling,
        ConnectionStatus::NeedsAuthentication,
    ] {
        let fixture = Fixture::new();
        fixture.agents.view.lock().unwrap().status = status.clone();
        let result = fixture
            .dialogue()
            .respond("request", &AtomicBool::new(false), &|_| {});
        let expected = if status == ConnectionStatus::NeedsAuthentication {
            VoiceTurnError::AgentAuthentication
        } else {
            VoiceTurnError::AgentBusy
        };
        assert!(matches!(result, Err(error) if error == expected));
        assert!(fixture.agents.sends.lock().unwrap().is_empty());
        assert!(fixture.agents.commands.lock().unwrap().is_empty());
    }
}

#[test]
fn permission_requests_require_the_user_and_cancellation_targets_only_the_owned_turn() {
    let fixture = Fixture::new();
    fixture.agents.view.lock().unwrap().permissions = vec![PermissionRequest {
        id: 1,
        title: "Allow?".into(),
        detail: "Write file".into(),
        choices: vec![],
    }];
    let cancelled = AtomicBool::new(false);
    let result = fixture
        .dialogue()
        .respond("request", &cancelled, &|progress| {
            if matches!(
                progress,
                VoicePromptProgress::Stage(VoiceTurnStage::NeedsAttention)
            ) {
                cancelled.store(true, Ordering::Release);
            }
        });
    assert!(matches!(result, Err(VoiceTurnError::Cancelled)));
    let commands = fixture.agents.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        AgentCommand::CancelTurn { response_id: 41 }
    ));
}

#[test]
fn an_interrupted_or_missing_response_is_not_read_as_a_success() {
    for missing in [false, true] {
        let fixture = Fixture::new();
        let result = fixture
            .dialogue()
            .respond("request", &AtomicBool::new(false), &|progress| {
                if matches!(
                    progress,
                    VoicePromptProgress::Stage(VoiceTurnStage::WaitingForAgent)
                ) {
                    let mut view = fixture.agents.view.lock().unwrap();
                    if missing {
                        view.messages.clear();
                    } else {
                        view.messages[0].status = MessageStatus::Interrupted;
                    }
                }
            });
        assert!(matches!(result, Err(VoiceTurnError::Agent(_))));
    }
}
