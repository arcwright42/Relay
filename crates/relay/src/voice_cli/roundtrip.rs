//! Opt-in real ASR → resident → thread Agent → TTS, using only disposable thread data.
use relay_core::{
    agents::AgentService,
    settings::Language,
    threads::{ThreadCommand, ThreadDraft, ThreadService},
    voice::{
        SpeechAudio, SpeechOutput, SpeechTranscriber, VoicePromptProgress, VoicePromptResult,
        VoicePromptService, VoiceTurnStage,
    },
};
use relay_runtime::{AgentRuntime, QwenSpeech, ThreadVoiceDialogue};
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Sandbox(PathBuf);
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn check(credential_root: &Path, wav: Vec<u8>) -> Result<(), String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Invalid system clock.")?
        .as_nanos();
    let sandbox = Sandbox(std::env::temp_dir().join(format!(
        "relay-voice-roundtrip-{}-{unique}",
        std::process::id()
    )));
    fs::create_dir(&sandbox.0).map_err(|_| "Could not create isolated voice check directory.")?;
    let components = credential_root.join("components");
    if !components.is_dir() {
        return Err("Install the local Agent adapter through Relay first.".into());
    }
    std::os::unix::fs::symlink(components, sandbox.0.join("components"))
        .map_err(|_| "Could not reuse the installed Agent adapter.")?;
    let threads = Arc::new(
        relay_runtime::native::NativeStore::open(sandbox.0.clone()).map_err(|e| e.to_string())?,
    );
    let thread = threads
        .snapshot()
        .threads
        .first()
        .cloned()
        .ok_or("Missing isolated thread.")?;
    threads.apply(ThreadCommand::Edit {
        thread: thread.id,
        expected_revision: thread.revision,
        draft: ThreadDraft {
            name: "Relay 语音联调".into(),
            description: "验证 Relay 语音链路：语音识别、主对话 Agent 回复与语音播报。".into(),
            instructions:
                "这是独立的语音集成测试。只回复一句语音链路测试成功；不要调用工具、读取或修改文件。"
                    .into(),
        },
    })?;
    let agents = Arc::new(AgentRuntime::new(sandbox.0.clone(), threads.clone()));
    let result = (|| {
        if !agents.snapshot(thread.id).installed {
            return Err("Install the current Agent adapter through Relay first.".into());
        }
        let speech = QwenSpeech::new(credential_root, Arc::new(relay_platform::MacAudioOutput));
        let cancelled = AtomicBool::new(false);
        let transcript = speech.transcribe(&SpeechAudio { wav }, &cancelled)?;
        if transcript.trim().is_empty() {
            return Err("The check audio produced no transcript.".into());
        }
        println!("ASR: {transcript}");
        let dialogue = ThreadVoiceDialogue::new(threads, agents.clone());
        let last_stage = RefCell::new(None);
        let (stop, stopped) = mpsc::channel();
        let reply = thread::scope(|scope| {
            let cancellation = &cancelled;
            scope.spawn(move || {
                if stopped.recv_timeout(Duration::from_secs(120)).is_err() {
                    cancellation.store(true, Ordering::Release);
                }
            });
            let result = dialogue
                .respond(&transcript, &cancelled, &|progress| match progress {
                    VoicePromptProgress::Stage(stage) => {
                        if *last_stage.borrow() != Some(stage) {
                            println!("Voice stage: {stage:?}");
                            *last_stage.borrow_mut() = Some(stage);
                        }
                        if stage == VoiceTurnStage::NeedsAttention {
                            cancelled.store(true, Ordering::Release);
                        }
                    }
                    VoicePromptProgress::Thread(thread) => {
                        println!("resident selected: {}", thread.name)
                    }
                    VoicePromptProgress::Response(_) => {}
                })
                .map_err(|error| format!("Voice roundtrip failed: {error:?}"));
            let _ = stop.send(());
            result
        })?;
        let VoicePromptResult::Reply(reply) = reply;
        if !reply.contains("语音链路测试成功") {
            return Err("Agent did not return the expected voice check response.".into());
        }
        if agents
            .snapshot(thread.id)
            .messages
            .iter()
            .any(|message| !message.tools.is_empty())
        {
            return Err("The check Agent unexpectedly invoked a tool.".into());
        }
        println!("Agent: {reply}");
        speech.speak(&reply, Language::SimplifiedChinese, &cancelled)?;
        println!("PASS: real ASR → resident → thread Agent → TTS playback.");
        Ok(())
    })();
    agents.shutdown();
    result
}
