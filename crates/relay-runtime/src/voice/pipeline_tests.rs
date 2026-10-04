//! Utterance lifecycle tests use controlled providers, without recording or network calls.
use super::{tests::*, *};
use relay_core::{ProjectId, routing::RouteTarget};
use std::{sync::atomic::AtomicBool, time::Duration};

struct Output(mpsc::Sender<mpsc::Sender<()>>);
impl SpeechOutput for Output {
    fn speak(&self, text: &str, _: Language, cancelled: &AtomicBool) -> Result<(), String> {
        assert_eq!(text, "项目回复");
        let (answer, response) = mpsc::channel();
        self.0.send(answer).unwrap();
        while !cancelled.load(Ordering::Acquire) {
            match response.recv_timeout(Duration::from_millis(5)) {
                Ok(()) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("output closed".into()),
            }
        }
        Ok(())
    }
}
struct Dialogue {
    prompts: Mutex<Vec<String>>,
    choose: bool,
}
impl VoicePromptService for Dialogue {
    fn respond(
        &self,
        prompt: &str,
        selection: Option<VoiceRouteSelection>,
        cancelled: &AtomicBool,
        progress: &dyn Fn(VoicePromptProgress),
    ) -> Result<VoicePromptResult, VoiceTurnError> {
        assert!(!cancelled.load(Ordering::Acquire));
        self.prompts.lock().unwrap().push(prompt.to_owned());
        if self.choose && selection.is_none() {
            return Ok(VoicePromptResult::ChooseProject {
                catalog_revision: 7,
                choices: vec![VoiceRouteChoice {
                    target: RouteTarget::Existing(ProjectId(1)),
                    project_name: Some("Project".into()),
                }],
            });
        }
        if self.choose {
            assert_eq!(selection.unwrap().catalog_revision, 7);
        }
        progress(VoicePromptProgress::Stage(VoiceTurnStage::WaitingForAgent));
        Ok(VoicePromptResult::Reply("项目回复".into()))
    }
}
fn emit(service: &VoiceRuntime, event: VoiceInputEvent) {
    let generation = service.shared.state.lock().unwrap().generation;
    service.shared.event(generation, event);
}
fn utterance(service: &VoiceRuntime, id: u64) {
    emit(
        service,
        VoiceInputEvent::Speech {
            session_id: id,
            audio: SpeechAudio {
                wav: b"wav".to_vec(),
            },
        },
    );
}
fn stage(service: &VoiceRuntime) -> Option<VoiceTurnStage> {
    service
        .snapshot()
        .session
        .and_then(|session| session.turn.map(|turn| turn.stage))
}
fn control(service: &VoiceRuntime) -> Arc<VoiceCaptureControl> {
    service
        .shared
        .state
        .lock()
        .unwrap()
        .control
        .clone()
        .unwrap()
}

#[test]
fn one_utterance_routes_once_then_playback_gates_capture_until_complete() {
    let settings = Arc::new(Preferences::default());
    let (sender, starts) = mpsc::channel();
    let (requests, responses) = mpsc::channel();
    let (plays, playback) = mpsc::channel();
    let dialogue = Arc::new(Dialogue {
        prompts: Mutex::default(),
        choose: false,
    });
    let service = VoiceRuntime::with_pipeline(
        settings,
        Arc::new(Resources),
        Arc::new(Backend(sender)),
        Arc::new(Transcriber(requests)),
        dialogue.clone(),
        Arc::new(Output(plays)),
    );
    let _active = active_backend(&service, &starts);
    emit(&service, VoiceInputEvent::Detected);
    let id = service.snapshot().session.unwrap().id;
    utterance(&service, id);
    let request = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(control(&service).paused.load(Ordering::Acquire));
    utterance(&service, id); // Speech and meters arriving during ASR are ignored.
    emit(
        &service,
        VoiceInputEvent::InputLevel {
            session_id: id,
            level: 100,
        },
    );
    assert_eq!(service.snapshot().input_level, 0);
    request
        .answer
        .send(Ok("嘿 relay，帮我总结今天的工作".into()))
        .unwrap();
    let finish = playback.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(stage(&service), Some(VoiceTurnStage::Speaking));
    assert!(control(&service).speaking.load(Ordering::Acquire));
    utterance(&service, id); // TTS must not become its own next prompt.
    assert_eq!(*dialogue.prompts.lock().unwrap(), ["帮我总结今天的工作"]);
    assert_eq!(service.snapshot().session.unwrap().captured_segments, 1);
    finish.send(()).unwrap();
    wait(|| stage(&service) == Some(VoiceTurnStage::Complete));
    assert!(!control(&service).paused.load(Ordering::Acquire));
    assert!(!control(&service).speaking.load(Ordering::Acquire));
    utterance(&service, id); // Same conversation accepts the next turn without a wake word.
    let request = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    request.answer.send(Ok("".into())).unwrap();
    wait(|| stage(&service) == Some(VoiceTurnStage::Complete));
    assert_eq!(dialogue.prompts.lock().unwrap().len(), 1);
    service.shutdown();
}

#[test]
fn low_confidence_choice_is_explicit_and_cannot_dispatch_twice() {
    let settings = Arc::new(Preferences::default());
    let (sender, starts) = mpsc::channel();
    let (requests, responses) = mpsc::channel();
    let dialogue = Arc::new(Dialogue {
        prompts: Mutex::default(),
        choose: true,
    });
    let service = VoiceRuntime::build(
        settings,
        Arc::new(Resources),
        Arc::new(Backend(sender)),
        Arc::new(Transcriber(requests)),
        Some(dialogue.clone()),
        None,
    );
    let _active = active_backend(&service, &starts);
    service.start_session();
    let id = service.snapshot().session.unwrap().id;
    utterance(&service, id);
    responses
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .answer
        .send(Ok("request".into()))
        .unwrap();
    wait(|| stage(&service) == Some(VoiceTurnStage::ChoosingProject));
    assert!(control(&service).paused.load(Ordering::Acquire));
    let turn = service.snapshot().session.unwrap().turn.unwrap().id;
    service.choose_project(id + 1, turn, RouteTarget::Existing(ProjectId(1)));
    service.choose_project(id, turn + 1, RouteTarget::Existing(ProjectId(1)));
    service.choose_project(id, turn, RouteTarget::NewProject);
    assert_eq!(dialogue.prompts.lock().unwrap().len(), 1);
    service.choose_project(id, turn, RouteTarget::Existing(ProjectId(1)));
    service.choose_project(id, turn, RouteTarget::Existing(ProjectId(1)));
    wait(|| stage(&service) == Some(VoiceTurnStage::Complete));
    assert_eq!(*dialogue.prompts.lock().unwrap(), ["request", "request"]);
    assert!(!control(&service).paused.load(Ordering::Acquire));
    service.shutdown();
}

#[test]
fn wake_only_and_cancelled_asr_never_dispatch_a_project_prompt() {
    let settings = Arc::new(Preferences::default());
    let (sender, starts) = mpsc::channel();
    let (requests, responses) = mpsc::channel();
    let dialogue = Arc::new(Dialogue {
        prompts: Mutex::default(),
        choose: false,
    });
    let service = VoiceRuntime::build(
        settings,
        Arc::new(Resources),
        Arc::new(Backend(sender)),
        Arc::new(Transcriber(requests)),
        Some(dialogue.clone()),
        None,
    );
    let _active = active_backend(&service, &starts);
    emit(&service, VoiceInputEvent::Detected);
    let id = service.snapshot().session.unwrap().id;
    utterance(&service, id);
    responses
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .answer
        .send(Ok("Hey Relay!".into()))
        .unwrap();
    wait(|| stage(&service) == Some(VoiceTurnStage::Complete));
    assert!(service.snapshot().session.unwrap().transcript.is_empty());
    assert!(dialogue.prompts.lock().unwrap().is_empty());
    utterance(&service, id);
    let request = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    service.resume_listening(id);
    utterance(&service, id);
    let next = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(request.answer.send(Ok("must not be sent".into())).is_err());
    next.answer.send(Ok("current".into())).unwrap();
    wait(|| stage(&service) == Some(VoiceTurnStage::Complete));
    assert_eq!(*dialogue.prompts.lock().unwrap(), ["current"]);
    service.shutdown();
}

#[test]
fn cancelling_playback_keeps_the_acoustic_tail_gated_and_long_input_needs_resume() {
    let settings = Arc::new(Preferences::default());
    let (sender, starts) = mpsc::channel();
    let (requests, responses) = mpsc::channel();
    let (plays, playback) = mpsc::channel();
    let service = VoiceRuntime::with_pipeline(
        settings,
        Arc::new(Resources),
        Arc::new(Backend(sender)),
        Arc::new(Transcriber(requests)),
        Arc::new(Dialogue {
            prompts: Mutex::default(),
            choose: false,
        }),
        Arc::new(Output(plays)),
    );
    let _active = active_backend(&service, &starts);
    service.start_session();
    let id = service.snapshot().session.unwrap().id;
    utterance(&service, id);
    responses
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .answer
        .send(Ok("request".into()))
        .unwrap();
    let _playing = playback.recv_timeout(Duration::from_secs(3)).unwrap();
    service.resume_listening(id);
    assert!(!control(&service).paused.load(Ordering::Acquire));
    assert!(control(&service).speaking.load(Ordering::Acquire));
    utterance(&service, id);
    assert_eq!(service.snapshot().session.unwrap().captured_segments, 1);
    wait(|| !control(&service).speaking.load(Ordering::Acquire));
    emit(&service, VoiceInputEvent::SpeechTooLong { session_id: id });
    let turn = service.snapshot().session.unwrap().turn.unwrap();
    assert_eq!(turn.error, Some(VoiceTurnError::InputTooLong));
    assert!(control(&service).paused.load(Ordering::Acquire));
    utterance(&service, id);
    assert_eq!(service.snapshot().session.unwrap().captured_segments, 1);
    service.resume_listening(id);
    assert!(!control(&service).paused.load(Ordering::Acquire));
    service.shutdown();
}

#[test]
fn restarting_the_microphone_preserves_the_playback_gate() {
    let (service, _, starts) = fixture();
    let _active = active_backend(&service, &starts);
    let previous = control(&service);
    previous.speaking.store(true, Ordering::Release);
    service.retry();
    let active = starts.recv_timeout(Duration::from_secs(3)).unwrap();
    active
        .send(Signal::Event(VoiceInputEvent::Listening))
        .unwrap();
    wait(|| service.snapshot().status == VoiceStatus::Listening);
    assert!(Arc::ptr_eq(&previous.speaking, &control(&service).speaking));
    emit(&service, VoiceInputEvent::Detected);
    assert!(service.snapshot().session.is_none());
    service.start_session();
    let id = service.snapshot().session.unwrap().id;
    utterance(&service, id);
    assert_eq!(service.snapshot().session.unwrap().captured_segments, 0);
    previous.speaking.store(false, Ordering::Release);
    utterance(&service, id);
    assert_eq!(service.snapshot().session.unwrap().captured_segments, 1);
    service.shutdown();
}
