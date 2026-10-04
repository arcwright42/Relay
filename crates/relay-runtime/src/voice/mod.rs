//! Application voice lifecycle; per-utterance processing lives in `turns`.
mod cloud;
mod dialogue;
mod resources;
mod turns;
pub use cloud::{QwenSpeech, save_voice_key};
pub use dialogue::ProjectVoiceDialogue;
pub use resources::BundledWakeResources;
#[cfg(test)]
mod dialogue_tests;
#[cfg(test)]
mod pipeline_tests;
#[cfg(test)]
mod tests;

use relay_core::{settings::SettingsService, voice::*};
use std::{
    sync::{Arc, Mutex, atomic::Ordering, mpsc},
    thread::{self, JoinHandle},
};
use turns::{Shared, State, VoiceJob};

enum Command {
    Start {
        generation: u64,
        control: Arc<VoiceCaptureControl>,
    },
    Stop,
    Shutdown,
}

pub struct VoiceRuntime {
    shared: Arc<Shared>,
    settings: Arc<dyn SettingsService>,
    sender: Mutex<Option<mpsc::Sender<Command>>>,
    manager: Mutex<Option<JoinHandle<()>>>,
    speech_worker: Mutex<Option<JoinHandle<()>>>,
}
impl VoiceRuntime {
    pub fn new(
        settings: Arc<dyn SettingsService>,
        resources: Arc<dyn WakeResources>,
        backend: Arc<dyn VoiceInputBackend>,
    ) -> Self {
        Self::with_transcriber(
            settings,
            resources,
            backend,
            Arc::new(UnconfiguredTranscriber),
        )
    }
    pub fn with_transcriber(
        settings: Arc<dyn SettingsService>,
        resources: Arc<dyn WakeResources>,
        backend: Arc<dyn VoiceInputBackend>,
        transcriber: Arc<dyn SpeechTranscriber>,
    ) -> Self {
        Self::build(settings, resources, backend, transcriber, None, None)
    }
    pub fn with_pipeline(
        settings: Arc<dyn SettingsService>,
        resources: Arc<dyn WakeResources>,
        backend: Arc<dyn VoiceInputBackend>,
        transcriber: Arc<dyn SpeechTranscriber>,
        dialogue: Arc<dyn VoicePromptService>,
        output: Arc<dyn SpeechOutput>,
    ) -> Self {
        Self::build(
            settings,
            resources,
            backend,
            transcriber,
            Some(dialogue),
            Some(output),
        )
    }
    fn build(
        settings: Arc<dyn SettingsService>,
        resources: Arc<dyn WakeResources>,
        backend: Arc<dyn VoiceInputBackend>,
        transcriber: Arc<dyn SpeechTranscriber>,
        dialogue: Option<Arc<dyn VoicePromptService>>,
        output: Option<Arc<dyn SpeechOutput>>,
    ) -> Self {
        let restore = settings.snapshot().voice_wake_enabled;
        let (speech, speech_receiver) = mpsc::sync_channel::<VoiceJob>(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            speech: Mutex::new(Some(speech)),
            transcriber,
            dialogue,
            output,
            playback: Arc::default(),
            settings: settings.clone(),
        });
        let transcribed = shared.clone();
        let speech_worker = thread::spawn(move || {
            while let Ok(job) = speech_receiver.recv() {
                transcribed.run(job);
            }
        });
        let (sender, receiver) = mpsc::channel();
        let managed = shared.clone();
        let manager = thread::spawn(move || {
            let mut active: Option<(Arc<VoiceCaptureControl>, JoinHandle<()>)> = None;
            while let Ok(mut command) = receiver.recv() {
                while !matches!(command, Command::Shutdown) {
                    let Ok(next) = receiver.try_recv() else {
                        break;
                    };
                    command = next;
                }
                if let Some((control, worker)) = active.take() {
                    control.cancelled.store(true, Ordering::Release);
                    let _ = worker.join();
                }
                match command {
                    Command::Start {
                        generation,
                        control,
                    } => {
                        if control.cancelled.load(Ordering::Acquire) || !managed.current(generation)
                        {
                            continue;
                        }
                        let shared = managed.clone();
                        let resources = resources.clone();
                        let backend = backend.clone();
                        let worker_control = control.clone();
                        let worker = thread::spawn(move || {
                            let result = resources.directory().and_then(|directory| {
                                if worker_control.cancelled.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                backend.listen(&directory, &worker_control, &|event| {
                                    shared.event(generation, event)
                                })
                            });
                            if !worker_control.cancelled.load(Ordering::Acquire) {
                                shared.failed(
                                    generation,
                                    result.err().unwrap_or_else(|| {
                                        VoiceError::new(
                                            VoiceErrorKind::DetectionFailed,
                                            "Voice input stopped unexpectedly.",
                                        )
                                    }),
                                );
                            }
                        });
                        active = Some((control, worker));
                    }
                    Command::Stop => {}
                    Command::Shutdown => break,
                }
            }
            if let Some((control, worker)) = active {
                control.cancelled.store(true, Ordering::Release);
                let _ = worker.join();
            }
        });
        let service = Self {
            shared,
            settings,
            sender: Mutex::new(Some(sender)),
            manager: Mutex::new(Some(manager)),
            speech_worker: Mutex::new(Some(speech_worker)),
        };
        if restore {
            service.change(true, false, false);
        }
        service
    }
    fn change(&self, enabled: bool, persist: bool, require_enabled: bool) {
        let sender = self.sender.lock().expect("voice command lock");
        let Some(sender) = sender.as_ref() else {
            return;
        };
        let mut state = self.shared.state.lock().expect("voice lock");
        if require_enabled && !state.view.enabled {
            return;
        }
        state.end_session();
        if let Some(control) = state.control.take() {
            control.cancelled.store(true, Ordering::Release);
        }
        state.generation += 1;
        state.gate = WakeGate::default();
        state.view.enabled = enabled;
        state.view.input_level = 0;
        state.view.input_device = None;
        state.view.status = if enabled {
            VoiceStatus::Starting
        } else {
            VoiceStatus::Off
        };
        let command = if enabled {
            let control = Arc::new(VoiceCaptureControl {
                speaking: self.shared.playback.clone(),
                ..Default::default()
            });
            state.control = Some(control.clone());
            Command::Start {
                generation: state.generation,
                control,
            }
        } else {
            Command::Stop
        };
        if persist {
            self.settings.set_voice_wake_enabled(enabled);
        }
        if sender.send(command).is_err() {
            state.view.status = VoiceStatus::Failed(VoiceError::new(
                VoiceErrorKind::DetectionFailed,
                "Voice manager has stopped.",
            ));
        }
    }
    pub fn shutdown(&self) {
        if let Some(sender) = self.sender.lock().expect("voice command lock").take() {
            let mut state = self.shared.state.lock().expect("voice lock");
            state.end_session();
            if let Some(control) = state.control.take() {
                control.cancelled.store(true, Ordering::Release);
            }
            state.generation += 1;
            state.view.enabled = false;
            state.view.status = VoiceStatus::Off;
            state.view.input_level = 0;
            let _ = sender.send(Command::Shutdown);
        }
        if let Some(manager) = self.manager.lock().expect("voice manager lock").take() {
            let _ = manager.join();
        }
        self.shared.speech.lock().expect("speech queue lock").take();
        if let Some(worker) = self
            .speech_worker
            .lock()
            .expect("speech worker lock")
            .take()
        {
            let _ = worker.join();
        }
    }
}
impl VoiceService for VoiceRuntime {
    fn snapshot(&self) -> VoiceSnapshot {
        self.shared.state.lock().expect("voice lock").view.clone()
    }
    fn set_enabled(&self, enabled: bool) {
        self.change(enabled, true, false);
    }
    fn retry(&self) {
        self.change(true, false, true);
    }
    fn start_session(&self) {
        let mut state = self.shared.state.lock().expect("voice lock");
        state.begin_session(self.shared.transcriber.available(), false);
    }
    fn end_session(&self, session_id: u64) {
        let mut state = self.shared.state.lock().expect("voice lock");
        if state
            .view
            .session
            .as_ref()
            .is_some_and(|session| session.id == session_id)
        {
            state.end_session();
        }
    }
    fn resume_listening(&self, session_id: u64) {
        let mut state = self.shared.state.lock().expect("voice lock");
        if state
            .view
            .session
            .as_ref()
            .is_some_and(|session| session.id == session_id)
        {
            state.resume(self.shared.transcriber.available());
        }
    }
    fn choose_project(
        &self,
        session_id: u64,
        turn_id: u64,
        target: relay_core::routing::RouteTarget,
    ) {
        self.shared.choose_project(session_id, turn_id, target);
    }
}
impl Drop for VoiceRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
