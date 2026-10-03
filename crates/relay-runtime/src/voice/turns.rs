//! One spoken utterance becomes one routed prompt; all callbacks carry a turn identity.
use relay_core::{settings::SettingsService, voice::*};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_TRANSCRIPT_CHARS: usize = 32_000;

#[derive(Default)]
pub(super) struct State {
    pub view: VoiceSnapshot,
    pub generation: u64,
    pub control: Option<Arc<VoiceCaptureControl>>,
    turn_cancelled: Option<Arc<AtomicBool>>,
    next_session: u64,
    next_turn: u64,
    first_wake_utterance: bool,
    pub gate: WakeGate,
}
impl State {
    pub fn pause(&self, paused: bool) {
        if let Some(control) = &self.control {
            control.capture_epoch.fetch_add(1, Ordering::AcqRel);
            control.paused.store(paused, Ordering::Release);
        }
    }
    pub fn end_session(&mut self) {
        if let Some(cancelled) = self.turn_cancelled.take() {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(control) = &self.control {
            control.session.store(0, Ordering::Release);
        }
        self.pause(false);
        self.view.session = None;
        self.view.input_level = 0;
        if self.view.status == VoiceStatus::Capturing {
            self.view.status = VoiceStatus::Listening;
        }
        self.gate = WakeGate::default();
        self.gate.accept(Instant::now());
    }
    pub fn begin_session(&mut self, configured: bool, from_wake: bool) {
        if self.view.status != VoiceStatus::Listening || self.view.session.is_some() {
            return;
        }
        self.next_session += 1;
        self.first_wake_utterance = from_wake;
        self.view.session = Some(VoiceSessionSnapshot {
            id: self.next_session,
            transcription: if configured {
                TranscriptionStatus::Ready
            } else {
                TranscriptionStatus::NotConfigured
            },
            transcript: String::new(),
            transcript_truncated: false,
            captured_segments: 0,
            pending_segments: 0,
            turn: None,
        });
        self.view.status = VoiceStatus::Capturing;
        self.view.input_level = 0;
        if let Some(control) = &self.control {
            control.paused.store(false, Ordering::Release);
            control.wake_preroll.store(from_wake, Ordering::Release);
            control.session.store(self.next_session, Ordering::Release);
        }
    }
    fn begin_turn(&mut self) -> Arc<AtomicBool> {
        if let Some(old) = self.turn_cancelled.take() {
            old.store(true, Ordering::Release);
        }
        self.next_turn += 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.turn_cancelled = Some(cancelled.clone());
        let session = self.view.session.as_mut().expect("active voice session");
        session.turn = Some(VoiceTurnSnapshot {
            id: self.next_turn,
            stage: VoiceTurnStage::Transcribing,
            prompt: String::new(),
            response: String::new(),
            project: None,
            choices: vec![],
            catalog_revision: None,
            error: None,
        });
        session.pending_segments = 1;
        self.pause(true);
        self.view.input_level = 0;
        cancelled
    }
    pub fn resume(&mut self, configured: bool) {
        if let Some(cancelled) = self.turn_cancelled.take() {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(session) = &mut self.view.session {
            session.pending_segments = 0;
            session.transcription = if configured {
                TranscriptionStatus::Ready
            } else {
                TranscriptionStatus::NotConfigured
            };
            if let Some(turn) = &mut session.turn {
                turn.stage = VoiceTurnStage::Complete;
                turn.error = None;
                turn.choices.clear();
            }
        }
        self.pause(false);
    }
}

pub(super) enum VoiceJobKind {
    Audio(SpeechAudio),
    Prompt {
        text: String,
        selection: VoiceRouteSelection,
    },
}
pub(super) struct VoiceJob {
    pub generation: u64,
    pub session_id: u64,
    pub turn_id: u64,
    pub cancelled: Arc<AtomicBool>,
    pub control: Arc<VoiceCaptureControl>,
    pub strip_wake: bool,
    pub kind: VoiceJobKind,
}

pub(super) struct Shared {
    pub state: Mutex<State>,
    pub speech: Mutex<Option<mpsc::SyncSender<VoiceJob>>>,
    pub transcriber: Arc<dyn SpeechTranscriber>,
    pub dialogue: Option<Arc<dyn VoicePromptService>>,
    pub output: Option<Arc<dyn SpeechOutput>>,
    pub playback: Arc<AtomicBool>,
    pub settings: Arc<dyn SettingsService>,
}
impl Shared {
    pub fn choose_project(
        &self,
        session_id: u64,
        turn_id: u64,
        target: relay_core::routing::RouteTarget,
    ) {
        let mut state = self.state.lock().expect("voice lock");
        let Some(session) = state
            .view
            .session
            .as_mut()
            .filter(|session| session.id == session_id)
        else {
            return;
        };
        let Some(turn) = session
            .turn
            .as_mut()
            .filter(|turn| turn.id == turn_id && turn.stage == VoiceTurnStage::ChoosingProject)
        else {
            return;
        };
        if !turn.choices.iter().any(|choice| choice.target == target) {
            return;
        }
        let Some(catalog_revision) = turn.catalog_revision else {
            return;
        };
        let text = turn.prompt.clone();
        turn.choices.clear();
        turn.stage = VoiceTurnStage::Routing;
        let Some(cancelled) = state.turn_cancelled.clone() else {
            return;
        };
        let Some(control) = state.control.clone() else {
            return;
        };
        let job = VoiceJob {
            generation: state.generation,
            session_id,
            turn_id,
            cancelled,
            control,
            strip_wake: false,
            kind: VoiceJobKind::Prompt {
                text,
                selection: VoiceRouteSelection {
                    target,
                    catalog_revision,
                },
            },
        };
        self.enqueue(&mut state, job);
    }
    pub fn current(&self, generation: u64) -> bool {
        let state = self.state.lock().expect("voice lock");
        state.generation == generation && state.view.enabled
    }
    fn accepts(state: &State, job: &VoiceJob) -> bool {
        state.view.enabled
            && state.generation == job.generation
            && !job.cancelled.load(Ordering::Acquire)
            && state.view.session.as_ref().is_some_and(|session| {
                session.id == job.session_id
                    && session
                        .turn
                        .as_ref()
                        .is_some_and(|turn| turn.id == job.turn_id)
            })
    }
    fn update(&self, job: &VoiceJob, update: impl FnOnce(&mut State)) -> bool {
        let mut state = self.state.lock().expect("voice lock");
        if !Self::accepts(&state, job) {
            return false;
        }
        update(&mut state);
        true
    }
    pub fn enqueue(&self, state: &mut State, job: VoiceJob) {
        if self
            .speech
            .lock()
            .expect("speech queue lock")
            .as_ref()
            .is_none_or(|sender| sender.try_send(job).is_err())
        {
            let session = state.view.session.as_mut().expect("active session");
            session.pending_segments = 0;
            let turn = session.turn.as_mut().expect("voice turn");
            turn.stage = VoiceTurnStage::Failed;
            turn.error = Some(VoiceTurnError::QueueFull);
            state.pause(false);
        }
    }
    pub fn event(&self, generation: u64, event: VoiceInputEvent) {
        let mut state = self.state.lock().expect("voice lock");
        if state.generation != generation || !state.view.enabled {
            return;
        }
        match event {
            VoiceInputEvent::RequestingMicrophone => {
                state.view.status = VoiceStatus::RequestingMicrophone
            }
            VoiceInputEvent::Listening => {
                if state.view.session.is_none() {
                    state.view.status = VoiceStatus::Listening;
                }
            }
            VoiceInputEvent::InputDevice(device) => state.view.input_device = Some(device),
            VoiceInputEvent::InputLevel { session_id, level } => {
                if state.view.session.as_ref().map_or(0, |session| session.id) == session_id {
                    state.view.input_level = if state.control.as_ref().is_some_and(|control| {
                        control.paused.load(Ordering::Acquire)
                            || control.speaking.load(Ordering::Acquire)
                    }) {
                        0
                    } else {
                        level.min(100)
                    };
                }
            }
            VoiceInputEvent::Detected => {
                if state.view.status == VoiceStatus::Listening
                    && !self.playback.load(Ordering::Acquire)
                    && state.gate.accept(Instant::now())
                {
                    state.view.detections += 1;
                    state.begin_session(self.transcriber.available(), true);
                }
            }
            VoiceInputEvent::Speech { session_id, audio } => {
                if state
                    .view
                    .session
                    .as_ref()
                    .is_none_or(|session| session.id != session_id)
                    || state.control.as_ref().is_none_or(|control| {
                        control.paused.load(Ordering::Acquire)
                            || control.speaking.load(Ordering::Acquire)
                    })
                {
                    return;
                }
                state.view.session.as_mut().unwrap().captured_segments += 1;
                let cancelled = state.begin_turn();
                if !self.transcriber.available() {
                    let session = state.view.session.as_mut().unwrap();
                    session.transcription = TranscriptionStatus::NotConfigured;
                    session.pending_segments = 0;
                    let turn = session.turn.as_mut().unwrap();
                    turn.stage = VoiceTurnStage::Failed;
                    turn.error = Some(VoiceTurnError::AsrNotConfigured);
                    state.pause(false);
                    return;
                }
                state.view.session.as_mut().unwrap().transcription =
                    TranscriptionStatus::Transcribing;
                let job = VoiceJob {
                    generation,
                    session_id,
                    turn_id: state.next_turn,
                    cancelled,
                    control: state.control.as_ref().unwrap().clone(),
                    strip_wake: std::mem::take(&mut state.first_wake_utterance),
                    kind: VoiceJobKind::Audio(audio),
                };
                self.enqueue(&mut state, job);
            }
            VoiceInputEvent::SpeechTooLong { session_id } => {
                if state
                    .view
                    .session
                    .as_ref()
                    .is_some_and(|session| session.id == session_id)
                    && state.control.as_ref().is_some_and(|control| {
                        !control.paused.load(Ordering::Acquire)
                            && !control.speaking.load(Ordering::Acquire)
                    })
                {
                    state.begin_turn();
                    let session = state.view.session.as_mut().unwrap();
                    session.pending_segments = 0;
                    let turn = session.turn.as_mut().unwrap();
                    turn.stage = VoiceTurnStage::Failed;
                    turn.error = Some(VoiceTurnError::InputTooLong);
                    // Resume explicitly so the remainder of an oversized utterance
                    // cannot become a partial, unintended command.
                }
            }
        }
    }
    pub fn failed(&self, generation: u64, error: VoiceError) {
        let mut state = self.state.lock().expect("voice lock");
        if state.generation == generation && state.view.enabled {
            state.end_session();
            state.view.status = VoiceStatus::Failed(error);
        }
    }
    pub fn run(&self, job: VoiceJob) {
        if !self.update(&job, |_| {}) {
            return;
        }
        let result = self.process(&job);
        self.update(&job, |state| {
            let session = state.view.session.as_mut().unwrap();
            session.pending_segments = 0;
            let turn = session.turn.as_mut().unwrap();
            match result {
                Ok(Some((revision, choices))) => {
                    turn.stage = VoiceTurnStage::ChoosingProject;
                    turn.catalog_revision = Some(revision);
                    turn.choices = choices;
                }
                Ok(None) => {
                    turn.stage = VoiceTurnStage::Complete;
                    state.pause(false);
                }
                Err(error) => {
                    if let VoiceTurnError::Transcription(detail) = &error {
                        session.transcription = TranscriptionStatus::Failed(detail.clone());
                    }
                    turn.stage = VoiceTurnStage::Failed;
                    turn.error = Some(error);
                    state.pause(false);
                }
            }
        });
    }
    fn process(
        &self,
        job: &VoiceJob,
    ) -> Result<Option<(u64, Vec<VoiceRouteChoice>)>, VoiceTurnError> {
        let (prompt, selection) = match &job.kind {
            VoiceJobKind::Audio(audio) => {
                let text = self
                    .transcriber
                    .transcribe(audio, &job.cancelled)
                    .map_err(VoiceTurnError::Transcription)?;
                let text = if job.strip_wake {
                    strip_wake_prefix(&text)
                } else {
                    text.trim().to_owned()
                };
                (text, None)
            }
            VoiceJobKind::Prompt { text, selection } => (text.clone(), Some(*selection)),
        };
        if !self.update(job, |state| {
            let session = state.view.session.as_mut().unwrap();
            session.transcription = TranscriptionStatus::Ready;
            session.pending_segments = 0;
            session.turn.as_mut().unwrap().prompt.clone_from(&prompt);
            if matches!(job.kind, VoiceJobKind::Audio(_)) && !prompt.is_empty() {
                if !session.transcript.is_empty() {
                    session.transcript.push('\n');
                }
                session.transcript.push_str(&prompt);
                if session.transcript.chars().count() > MAX_TRANSCRIPT_CHARS {
                    session.transcript = session
                        .transcript
                        .chars()
                        .rev()
                        .take(MAX_TRANSCRIPT_CHARS)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    session.transcript_truncated = true;
                }
            }
        }) {
            return Err(VoiceTurnError::Cancelled);
        }
        if prompt.is_empty() {
            return Ok(None);
        }
        let Some(dialogue) = &self.dialogue else {
            return Ok(None);
        };
        let result = dialogue.respond(&prompt, selection, &job.cancelled, &|progress| {
            self.update(job, |state| {
                let turn = state.view.session.as_mut().unwrap().turn.as_mut().unwrap();
                match progress {
                    VoicePromptProgress::Stage(stage) => turn.stage = stage,
                    VoicePromptProgress::Project(project) => turn.project = Some(project),
                    VoicePromptProgress::Response(text) => turn.response = text,
                }
            });
        })?;
        if job.cancelled.load(Ordering::Acquire) {
            return Err(VoiceTurnError::Cancelled);
        }
        match result {
            VoicePromptResult::ChooseProject {
                catalog_revision,
                choices,
            } => Ok(Some((catalog_revision, choices))),
            VoicePromptResult::Reply(text) => {
                if !self.update(job, |state| {
                    state
                        .view
                        .session
                        .as_mut()
                        .unwrap()
                        .turn
                        .as_mut()
                        .unwrap()
                        .response = text.chars().take(32_000).collect()
                }) {
                    return Err(VoiceTurnError::Cancelled);
                }
                if let Some(output) = &self.output
                    && !text.trim().is_empty()
                {
                    self.update(job, |state| {
                        state
                            .view
                            .session
                            .as_mut()
                            .unwrap()
                            .turn
                            .as_mut()
                            .unwrap()
                            .stage = VoiceTurnStage::Speaking
                    });
                    job.control.speaking.store(true, Ordering::Release);
                    let result =
                        output.speak(&text, self.settings.snapshot().language, &job.cancelled);
                    // Drain the acoustic tail before capture can become a new user prompt.
                    thread::sleep(Duration::from_millis(300));
                    job.control.speaking.store(false, Ordering::Release);
                    result.map_err(VoiceTurnError::SpeechOutput)?;
                }
                Ok(None)
            }
        }
    }
}

fn strip_wake_prefix(text: &str) -> String {
    let text = text.trim();
    let lower = text.to_lowercase();
    for prefix in [
        "hey relay",
        "hey, relay",
        "嘿 relay",
        "嘿，relay",
        "嘿relay",
        "嗨 relay",
        "嗨relay",
    ] {
        if lower.starts_with(prefix) {
            return text[prefix.len()..]
                .trim_start_matches(|c: char| c.is_whitespace() || ",，.。!！:：".contains(c))
                .to_owned();
        }
    }
    if lower.trim_matches(|c: char| !c.is_alphanumeric()) == "relay" {
        String::new()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wake_only_audio_is_not_a_prompt_and_following_speech_is_preserved() {
        assert_eq!(
            strip_wake_prefix("嘿 Relay，帮我总结今天的工作。"),
            "帮我总结今天的工作。"
        );
        assert_eq!(strip_wake_prefix("Hey Relay!"), "");
        assert_eq!(strip_wake_prefix("relay。"), "");
        assert_eq!(strip_wake_prefix("relay 是什么？"), "relay 是什么？");
    }
}
