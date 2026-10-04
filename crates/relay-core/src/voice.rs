//! Voice wake, continuous capture and transcription contracts, independent of UI.
mod speech;
use crate::{ThreadId, settings::Language};
pub use speech::spoken_text;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64},
    },
    time::{Duration, Instant},
};

pub const WAKE_PHRASE: &str = "嘿 relay / Hey Relay";
pub const WAKE_COOLDOWN: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceErrorKind {
    ResourcesUnavailable,
    MicrophoneDenied,
    MicrophoneUnavailable,
    DetectionFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceError {
    pub kind: VoiceErrorKind,
    pub detail: String,
}

impl VoiceError {
    pub fn new(kind: VoiceErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum VoiceStatus {
    #[default]
    Off,
    Starting,
    RequestingMicrophone,
    Listening,
    Capturing,
    Failed(VoiceError),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoiceSnapshot {
    pub enabled: bool,
    pub status: VoiceStatus,
    pub detections: u64,
    pub input_device: Option<String>,
    pub input_level: u8,
    pub session: Option<VoiceSessionSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptionStatus {
    NotConfigured,
    Ready,
    Transcribing,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceSessionSnapshot {
    pub id: u64,
    pub transcription: TranscriptionStatus,
    pub transcript: String,
    pub transcript_truncated: bool,
    pub captured_segments: u64,
    pub pending_segments: u32,
    pub turn: Option<VoiceTurnSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceTurnStage {
    Transcribing,
    Connecting,
    WaitingForAgent,
    NeedsAttention,
    Speaking,
    Complete,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceTurnError {
    AsrNotConfigured,
    Transcription(String),
    ThreadUnavailable,
    AgentBusy,
    AgentAuthentication,
    Agent(String),
    SpeechOutput(String),
    InputTooLong,
    QueueFull,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceThread {
    pub id: ThreadId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceTurnSnapshot {
    pub id: u64,
    pub stage: VoiceTurnStage,
    pub prompt: String,
    pub response: String,
    pub thread: Option<VoiceThread>,
    pub error: Option<VoiceTurnError>,
}

pub enum VoicePromptProgress {
    Stage(VoiceTurnStage),
    Thread(VoiceThread),
    Response(String),
}

pub enum VoicePromptResult {
    Reply(String),
}

/// Main-conversation execution is independent of microphone and ASR providers.
pub trait VoicePromptService: Send + Sync {
    fn respond(
        &self,
        prompt: &str,
        cancelled: &AtomicBool,
        progress: &dyn Fn(VoicePromptProgress),
    ) -> Result<VoicePromptResult, VoiceTurnError>;
}

pub trait SpeechOutput: Send + Sync {
    /// Return only when playback completes, fails, or has stopped after cancellation.
    fn speak(&self, text: &str, language: Language, cancelled: &AtomicBool) -> Result<(), String>;
}

/// Native playback accepts provider-independent mono PCM16, without files or URLs.
pub trait SpeechAudioOutput: Send + Sync {
    fn open(&self, sample_rate: u32) -> Result<Box<dyn SpeechAudioStream>, String>;
}

/// Owned by the speech worker. Dropping the stream immediately stops playback.
pub trait SpeechAudioStream {
    /// Nonblocking backpressure: false means retry this entire chunk later.
    fn try_write(&mut self, samples: &[i16]) -> Result<bool, String>;
    fn is_drained(&self) -> Result<bool, String>;
}

/// Provider-independent audio payload: mono 16 kHz PCM16 in a WAV container.
/// One utterance is bounded to sixty seconds and is never persisted by the voice domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechAudio {
    pub wav: Vec<u8>,
}

pub trait SpeechTranscriber: Send + Sync {
    fn available(&self) -> bool;
    /// Runs on a bounded background queue. Respect cancellation and release
    /// any network request promptly; stale results cannot enter another session.
    fn transcribe(&self, audio: &SpeechAudio, cancelled: &AtomicBool) -> Result<String, String>;
}

pub struct UnconfiguredTranscriber;
impl SpeechTranscriber for UnconfiguredTranscriber {
    fn available(&self) -> bool {
        false
    }
    fn transcribe(&self, _: &SpeechAudio, _: &AtomicBool) -> Result<String, String> {
        Err("Speech recognition is not configured.".into())
    }
}

#[derive(Default)]
pub struct VoiceCaptureControl {
    pub cancelled: AtomicBool,
    /// Zero means keyword detection; a nonzero value selects a continuous session.
    pub session: AtomicU64,
    /// Keep the wake phrase's trailing audio only for a keyword-triggered session.
    pub wake_preroll: AtomicBool,
    /// Processing and playback suspend utterance capture without closing the session.
    pub paused: AtomicBool,
    /// Playback and its short acoustic tail remain gated even if the user resumes early.
    pub speaking: Arc<AtomicBool>,
    /// Discard buffered audio even when a brief pause ends between capture callbacks.
    pub capture_epoch: AtomicU64,
}

/// Commands return immediately. The application observes the active session.
pub trait VoiceService: Send + Sync {
    fn snapshot(&self) -> VoiceSnapshot;
    fn set_enabled(&self, enabled: bool);
    fn retry(&self);
    fn start_session(&self);
    /// A stale window may only end the session it was opened for.
    fn end_session(&self, session_id: u64);
    fn resume_listening(&self, session_id: u64);
}

/// Resource and native ports carry no third-party model handles.
pub trait WakeResources: Send + Sync {
    fn directory(&self) -> Result<PathBuf, VoiceError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceInputEvent {
    RequestingMicrophone,
    Listening,
    InputDevice(String),
    InputLevel { session_id: u64, level: u8 },
    Detected,
    Speech { session_id: u64, audio: SpeechAudio },
    SpeechTooLong { session_id: u64 },
}

/// Blocking capture and detection run on an owned background worker.
/// Implementations release the microphone and return promptly when cancelled.
pub trait VoiceInputBackend: Send + Sync {
    fn listen(
        &self,
        model_directory: &std::path::Path,
        control: &VoiceCaptureControl,
        emit: &dyn Fn(VoiceInputEvent),
    ) -> Result<(), VoiceError>;
}

/// One utterance may produce several candidates; accept at most one per cooldown.
#[derive(Default)]
pub struct WakeGate {
    last_accepted: Option<Instant>,
}

impl WakeGate {
    pub fn accept(&mut self, now: Instant) -> bool {
        if self
            .last_accepted
            .is_some_and(|last| now.duration_since(last) < WAKE_COOLDOWN)
        {
            return false;
        }
        self.last_accepted = Some(now);
        true
    }
}

/// Used by standalone UI previews before the application injects its service.
pub struct EmptyVoiceService;
impl VoiceService for EmptyVoiceService {
    fn snapshot(&self) -> VoiceSnapshot {
        VoiceSnapshot::default()
    }
    fn set_enabled(&self, _: bool) {}
    fn retry(&self) {}
    fn start_session(&self) {}
    fn end_session(&self, _: u64) {}
    fn resume_listening(&self, _: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_candidates_do_not_extend_the_cooldown() {
        let mut gate = WakeGate::default();
        let start = Instant::now();
        assert!(gate.accept(start));
        assert!(!gate.accept(start + Duration::from_millis(100)));
        assert!(!gate.accept(start + WAKE_COOLDOWN - Duration::from_millis(1)));
        assert!(gate.accept(start + WAKE_COOLDOWN));
    }
}
