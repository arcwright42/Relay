use super::VoicePanel;
use gpui_kit::{AppContext, TestAppContext, component::Root, px, size, test::TestWindowExt};
use relay_core::{settings::*, voice::*};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Settings(Mutex<SettingsSnapshot>);
impl SettingsService for Settings {
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
struct Voice {
    snapshot: Mutex<VoiceSnapshot>,
    ends: AtomicUsize,
    resumes: AtomicUsize,
}
impl Voice {
    fn active() -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(VoiceSnapshot {
                enabled: true,
                status: VoiceStatus::Capturing,
                input_device: Some("Fixture microphone".into()),
                input_level: 65,
                session: Some(VoiceSessionSnapshot {
                    id: 1,
                    transcription: TranscriptionStatus::NotConfigured,
                    transcript: String::new(),
                    transcript_truncated: false,
                    captured_segments: 2,
                    pending_segments: 0,
                    turn: None,
                }),
                ..Default::default()
            }),
            ends: AtomicUsize::new(0),
            resumes: AtomicUsize::new(0),
        })
    }
}
impl VoiceService for Voice {
    fn snapshot(&self) -> VoiceSnapshot {
        self.snapshot.lock().unwrap().clone()
    }
    fn set_enabled(&self, _: bool) {}
    fn retry(&self) {}
    fn start_session(&self) {}
    fn resume_listening(&self, session_id: u64) {
        assert_eq!(session_id, 1);
        self.resumes.fetch_add(1, Ordering::Relaxed);
    }

    fn end_session(&self, session_id: u64) {
        assert_eq!(session_id, 1);
        self.ends.fetch_add(1, Ordering::Relaxed);
        let mut snapshot = self.snapshot.lock().unwrap();
        snapshot.session = None;
        snapshot.status = VoiceStatus::Listening;
    }
}

#[gpui_kit::test]
fn voice_panel_end_button_closes_the_session_in_both_languages(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (language, with_transcript) in [
        (Language::SimplifiedChinese, false),
        (Language::English, false),
        (Language::SimplifiedChinese, true),
        (Language::English, true),
    ] {
        let voice = Voice::active();
        if with_transcript {
            let mut snapshot = voice.snapshot.lock().unwrap();
            let session = snapshot.session.as_mut().unwrap();
            session.transcription = TranscriptionStatus::Ready;
            session.turn = Some(turn(VoiceTurnStage::Speaking));
            let turn = session.turn.as_mut().unwrap();
            turn.prompt = "请总结今天的工作。 A voice request.\n".repeat(20);
            turn.response = "这是很长的一段回复。 A long reply.\n".repeat(100);
        }
        let settings = Arc::new(Settings::default());
        settings.set_language(language);
        let window = cx.add_window(|window, cx| {
            window.resize(size(px(480.), px(540.)));
            window.bounds_changed(cx);
            let panel = cx.new(|cx| VoicePanel::new(voice.clone(), settings, window, cx));
            Root::new(panel, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("end-voice-session").visible());
            assert!(
                window.find("end-voice-session").bounds().bottom() <= window.viewport_size().height
            );
            window.click("end-voice-session", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(voice.ends.load(Ordering::Relaxed), 1);
        assert!(voice.snapshot().session.is_none());
        assert!(cx.update(|cx| cx.windows().is_empty()));
    }
}

#[gpui_kit::test]
fn externally_cancelled_session_closes_the_voice_panel(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let voice = Voice::active();
    cx.add_window(|window, cx| {
        let panel =
            cx.new(|cx| VoicePanel::new(voice.clone(), Arc::new(Settings::default()), window, cx));
        Root::new(panel, window, cx)
    });
    voice.end_session(1);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.windows().is_empty()));
}

fn turn(stage: VoiceTurnStage) -> VoiceTurnSnapshot {
    VoiceTurnSnapshot {
        id: 9,
        stage,
        prompt: "帮我总结工作。".into(),
        response: String::new(),
        thread: None,
        error: None,
    }
}

#[gpui_kit::test]
fn resume_and_end_keep_the_session_identity(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for language in [Language::SimplifiedChinese, Language::English] {
        let voice = Voice::active();
        let turn = turn(VoiceTurnStage::WaitingForAgent);
        voice
            .snapshot
            .lock()
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .turn = Some(turn);
        let settings = Arc::new(Settings::default());
        settings.set_language(language);
        let window = cx.add_window(|window, cx| {
            window.resize(size(px(480.), px(540.)));
            window.bounds_changed(cx);
            let panel = cx.new(|cx| VoicePanel::new(voice.clone(), settings, window, cx));
            Root::new(panel, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("resume-voice-listening", cx);
            window.click("end-voice-session", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(voice.resumes.load(Ordering::Relaxed), 1);
        assert_eq!(voice.ends.load(Ordering::Relaxed), 1);
    }
}
