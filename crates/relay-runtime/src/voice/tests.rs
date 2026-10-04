use super::*;
pub(super) use relay_core::settings::Language;
use relay_core::settings::SettingsSnapshot;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Preferences(Mutex<SettingsSnapshot>);
impl SettingsService for Preferences {
    fn snapshot(&self) -> SettingsSnapshot {
        self.0.lock().unwrap().clone()
    }
    fn set_language(&self, language: Language) {
        self.0.lock().unwrap().language = language;
    }
    fn set_voice_wake_enabled(&self, enabled: bool) {
        self.0.lock().unwrap().voice_wake_enabled = enabled;
    }
}
pub(super) struct Resources;
impl WakeResources for Resources {
    fn directory(&self) -> Result<std::path::PathBuf, VoiceError> {
        Ok("/fixture".into())
    }
}
pub(super) enum Signal {
    Event(VoiceInputEvent),
    Fail,
    Inspect(mpsc::Sender<u64>),
}
pub(super) struct Backend(pub mpsc::Sender<mpsc::Sender<Signal>>);
impl VoiceInputBackend for Backend {
    fn listen(
        &self,
        _: &std::path::Path,
        control: &VoiceCaptureControl,
        emit: &dyn Fn(VoiceInputEvent),
    ) -> Result<(), VoiceError> {
        let cancelled = &control.cancelled;
        let (sender, receiver) = mpsc::channel();
        self.0.send(sender).unwrap();
        while !cancelled.load(Ordering::Acquire) {
            match receiver.recv_timeout(Duration::from_millis(5)) {
                Ok(Signal::Event(event)) => emit(event),
                Ok(Signal::Fail) => {
                    return Err(VoiceError::new(VoiceErrorKind::MicrophoneDenied, "denied"));
                }
                Ok(Signal::Inspect(reply)) => {
                    reply.send(control.session.load(Ordering::Acquire)).unwrap();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }
}
pub(super) fn wait(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "voice state transition timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
pub(super) fn fixture() -> (
    VoiceRuntime,
    Arc<Preferences>,
    mpsc::Receiver<mpsc::Sender<Signal>>,
) {
    fixture_with_transcriber(Arc::new(UnconfiguredTranscriber))
}
fn fixture_with_transcriber(
    transcriber: Arc<dyn SpeechTranscriber>,
) -> (
    VoiceRuntime,
    Arc<Preferences>,
    mpsc::Receiver<mpsc::Sender<Signal>>,
) {
    let settings = Arc::new(Preferences::default());
    let (sender, starts) = mpsc::channel();
    let service = VoiceRuntime::with_transcriber(
        settings.clone(),
        Arc::new(Resources),
        Arc::new(Backend(sender)),
        transcriber,
    );
    (service, settings, starts)
}

pub(super) fn active_backend(
    service: &VoiceRuntime,
    starts: &mpsc::Receiver<mpsc::Sender<Signal>>,
) -> mpsc::Sender<Signal> {
    service.set_enabled(true);
    let active = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    active
        .send(Signal::Event(VoiceInputEvent::Listening))
        .unwrap();
    wait(|| service.snapshot().status == VoiceStatus::Listening);
    active
}
fn session_control(active: &mpsc::Sender<Signal>) -> u64 {
    let (reply, response) = mpsc::channel();
    active.send(Signal::Inspect(reply)).unwrap();
    response.recv_timeout(Duration::from_secs(3)).unwrap()
}
fn speech(session_id: u64, contents: &str) -> Signal {
    Signal::Event(VoiceInputEvent::Speech {
        session_id,
        audio: SpeechAudio {
            wav: contents.as_bytes().to_vec(),
        },
    })
}

#[test]
fn wake_starts_continuous_capture_and_unconfigured_asr_does_not_accumulate_audio() {
    let (service, _, starts) = fixture();
    let active = active_backend(&service, &starts);
    active
        .send(Signal::Event(VoiceInputEvent::Detected))
        .unwrap();
    wait(|| service.snapshot().session.is_some());
    let first = service.snapshot().session.unwrap().id;
    assert_eq!(service.snapshot().status, VoiceStatus::Capturing);
    assert_eq!(session_control(&active), first);
    for text in ["first utterance", "second utterance"] {
        active.send(speech(first, text)).unwrap();
    }
    active
        .send(Signal::Event(VoiceInputEvent::Detected))
        .unwrap();
    active
        .send(Signal::Event(VoiceInputEvent::InputLevel {
            session_id: first,
            level: 65,
        }))
        .unwrap();
    wait(|| {
        service
            .snapshot()
            .session
            .as_ref()
            .unwrap()
            .captured_segments
            == 2
    });
    assert_eq!(session_control(&active), first);
    let view = service.snapshot();
    assert_eq!(view.detections, 1);
    assert_eq!(view.input_level, 65);
    let session = view.session.unwrap();
    assert_eq!(session.transcription, TranscriptionStatus::NotConfigured);
    assert_eq!(session.pending_segments, 0);
    assert!(session.transcript.is_empty());
    service.end_session(first);
    assert_eq!(session_control(&active), 0);
    assert_eq!(service.snapshot().status, VoiceStatus::Listening);
    assert!(service.snapshot().session.is_none());
    service.start_session();
    let second = service.snapshot().session.unwrap().id;
    assert_ne!(first, second);
    service.end_session(first);
    assert_eq!(service.snapshot().session.unwrap().id, second);
    active.send(speech(first, "stale audio")).unwrap();
    active
        .send(Signal::Event(VoiceInputEvent::InputLevel {
            session_id: first,
            level: 100,
        }))
        .unwrap();
    assert_eq!(session_control(&active), second);
    assert_eq!(service.snapshot().session.unwrap().captured_segments, 0);
    assert_ne!(service.snapshot().input_level, 100);
    service.shutdown();
}

pub(super) struct TranscriptionRequest {
    pub audio: SpeechAudio,
    pub answer: mpsc::Sender<Result<String, String>>,
}
pub(super) struct Transcriber(pub mpsc::Sender<TranscriptionRequest>);
impl SpeechTranscriber for Transcriber {
    fn available(&self) -> bool {
        true
    }
    fn transcribe(&self, audio: &SpeechAudio, cancelled: &AtomicBool) -> Result<String, String> {
        let (answer, response) = mpsc::channel();
        self.0
            .send(TranscriptionRequest {
                audio: audio.clone(),
                answer,
            })
            .unwrap();
        loop {
            match response.recv_timeout(Duration::from_millis(5)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err("disconnected".into()),
            }
            if cancelled.load(Ordering::Acquire) {
                // Simulate a request finishing successfully as cancellation arrives.
                return Ok("late result from cancelled session".into());
            }
        }
    }
}

#[test]
fn ordered_transcription_stays_in_its_session_and_listening_survives_provider_errors() {
    let (requests, responses) = mpsc::channel();
    let (service, _, starts) = fixture_with_transcriber(Arc::new(Transcriber(requests)));
    let active = active_backend(&service, &starts);
    service.start_session();
    let first = service.snapshot().session.unwrap().id;
    active.send(speech(first, "wav one")).unwrap();
    let one = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(one.audio.wav, b"wav one");
    one.answer.send(Ok("第一句话".into())).unwrap();
    wait(|| {
        service
            .snapshot()
            .session
            .as_ref()
            .unwrap()
            .turn
            .as_ref()
            .unwrap()
            .stage
            == VoiceTurnStage::Complete
    });
    active.send(speech(first, "wav two")).unwrap();
    let two = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(two.audio.wav, b"wav two");
    two.answer.send(Ok("第二句话".into())).unwrap();
    wait(|| {
        service
            .snapshot()
            .session
            .as_ref()
            .unwrap()
            .turn
            .as_ref()
            .unwrap()
            .stage
            == VoiceTurnStage::Complete
    });
    assert_eq!(
        service.snapshot().session.unwrap().transcript,
        "第一句话\n第二句话"
    );
    active.send(speech(first, "failed request")).unwrap();
    responses
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .answer
        .send(Err("provider unavailable".into()))
        .unwrap();
    wait(|| {
        matches!(
            service.snapshot().session.as_ref().unwrap().transcription,
            TranscriptionStatus::Failed(_)
        )
    });
    assert_eq!(service.snapshot().status, VoiceStatus::Capturing);
    active.send(speech(first, "cancelled request")).unwrap();
    let cancelled = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    service.end_session(first);
    service.start_session();
    let second = service.snapshot().session.unwrap().id;
    active.send(speech(second, "new session")).unwrap();
    let fresh = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(fresh.audio.wav, b"new session");
    assert!(cancelled.answer.send(Ok("stale result".into())).is_err());
    fresh.answer.send(Ok("新的会话".into())).unwrap();
    wait(|| {
        service
            .snapshot()
            .session
            .as_ref()
            .unwrap()
            .turn
            .as_ref()
            .unwrap()
            .stage
            == VoiceTurnStage::Complete
    });
    assert_eq!(service.snapshot().session.unwrap().transcript, "新的会话");
    service.shutdown();
}

#[test]
fn slow_asr_suspends_capture_and_retry_cancels_old_jobs() {
    let (requests, responses) = mpsc::channel();
    let (service, _, starts) = fixture_with_transcriber(Arc::new(Transcriber(requests)));
    let active = active_backend(&service, &starts);
    service.start_session();
    let first = service.snapshot().session.unwrap().id;
    active.send(speech(first, "in flight")).unwrap();
    let in_flight = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    for _ in 0..5 {
        active.send(speech(first, "queued")).unwrap();
    }
    assert_eq!(session_control(&active), first);
    let session = service.snapshot().session.unwrap();
    assert_eq!(session.captured_segments, 1);
    assert_eq!(session.pending_segments, 1);
    assert_eq!(session.transcription, TranscriptionStatus::Transcribing);
    assert_eq!(service.snapshot().status, VoiceStatus::Capturing);
    service.retry();
    let restarted = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    restarted
        .send(Signal::Event(VoiceInputEvent::Listening))
        .unwrap();
    wait(|| service.snapshot().status == VoiceStatus::Listening);
    service.start_session();
    let second = service.snapshot().session.unwrap().id;
    restarted.send(speech(second, "after retry")).unwrap();
    let fresh = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(fresh.audio.wav, b"after retry");
    assert!(in_flight.answer.send(Ok("stale".into())).is_err());
    fresh.answer.send(Ok("restarted".into())).unwrap();
    wait(|| service.snapshot().session.as_ref().unwrap().transcript == "restarted");
    service.shutdown();
}

#[test]
fn repeated_wake_candidates_start_one_session_and_cancelled_generations_cannot_wake() {
    let (service, settings, starts) = fixture();
    settings.set_language(Language::English);
    service.set_enabled(true);
    let first = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    first
        .send(Signal::Event(VoiceInputEvent::Listening))
        .unwrap();
    first
        .send(Signal::Event(VoiceInputEvent::Detected))
        .unwrap();
    first
        .send(Signal::Event(VoiceInputEvent::Detected))
        .unwrap();
    wait(|| service.snapshot().detections == 1);
    let first_session = service.snapshot().session.unwrap().id;
    let first_generation = service.shared.state.lock().unwrap().generation;
    service.set_enabled(false);
    let _ = first.send(Signal::Event(VoiceInputEvent::Detected));
    service
        .shared
        .event(first_generation, VoiceInputEvent::Detected);
    assert!(service.snapshot().session.is_none());
    assert_eq!(service.snapshot().status, VoiceStatus::Off);
    service.set_enabled(true);
    let second = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    second
        .send(Signal::Event(VoiceInputEvent::Listening))
        .unwrap();
    second
        .send(Signal::Event(VoiceInputEvent::Detected))
        .unwrap();
    wait(|| service.snapshot().detections == 2);
    assert_ne!(service.snapshot().session.unwrap().id, first_session);
    service.shutdown();
    assert!(settings.snapshot().voice_wake_enabled);
    assert_eq!(settings.snapshot().language, Language::English);
    assert!(
        second
            .send(Signal::Event(VoiceInputEvent::Detected))
            .is_err()
    );
}

#[test]
fn permission_failure_is_retryable() {
    let (service, _, starts) = fixture();
    service.set_enabled(true);
    starts
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .send(Signal::Fail)
        .unwrap();
    wait(|| matches!(service.snapshot().status, VoiceStatus::Failed(_)));
    service.retry();
    let active = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    active
        .send(Signal::Event(VoiceInputEvent::Listening))
        .unwrap();
    wait(|| service.snapshot().status == VoiceStatus::Listening);
    service.shutdown();
}

#[test]
fn enabled_preference_restores_and_permission_wait_can_be_cancelled() {
    let settings = Arc::new(Preferences::default());
    settings.set_voice_wake_enabled(true);
    let (sender, starts) = mpsc::channel();
    let service = VoiceRuntime::new(
        settings.clone(),
        Arc::new(Resources),
        Arc::new(Backend(sender)),
    );
    let active = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    active
        .send(Signal::Event(VoiceInputEvent::RequestingMicrophone))
        .unwrap();
    wait(|| service.snapshot().status == VoiceStatus::RequestingMicrophone);
    service.set_enabled(false);
    service.shutdown();
    assert!(!settings.snapshot().voice_wake_enabled);
    assert_eq!(service.snapshot().status, VoiceStatus::Off);
}
