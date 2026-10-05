//! DashScope adapters only: no microphone, task decisions, GPUI or Agent ownership.
mod asr;
mod config;
#[cfg(test)]
mod tests;
mod transport;
mod tts;
pub use config::save_voice_key;

use relay_core::{
    settings::Language,
    voice::{SpeechAudio, SpeechAudioOutput, SpeechOutput, SpeechTranscriber, spoken_text},
};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub struct QwenSpeech {
    config: Result<Option<config::Config>, String>,
    output: Arc<dyn SpeechAudioOutput>,
}
impl QwenSpeech {
    pub fn new(root: &Path, output: Arc<dyn SpeechAudioOutput>) -> Self {
        Self {
            config: config::load(root),
            output,
        }
    }
    fn configuration(&self) -> Result<&config::Config, String> {
        self.config
            .as_ref()
            .map_err(Clone::clone)?
            .as_ref()
            .ok_or_else(|| "Configure a DashScope voice API key first.".into())
    }
}
impl SpeechTranscriber for QwenSpeech {
    fn available(&self) -> bool {
        !matches!(self.config, Ok(None))
    }
    fn transcribe(&self, audio: &SpeechAudio, cancelled: &AtomicBool) -> Result<String, String> {
        let config = self.configuration()?;
        transport::execute(
            cancelled,
            Duration::from_secs(90),
            asr::transcribe(config, &audio.wav),
        )
    }
}
impl SpeechOutput for QwenSpeech {
    fn speak(&self, text: &str, language: Language, cancelled: &AtomicBool) -> Result<(), String> {
        let config = self.configuration()?;
        let text = spoken_text(text, language);
        transport::execute(
            cancelled,
            Duration::from_secs(330),
            tts::speak(config, &text, self.output.as_ref()),
        )
    }
}
