//! Silero VAD endpoints complete utterances after a pause, before any ASR request.
use relay_core::voice::{SpeechAudio, VoiceError, VoiceErrorKind};
use sherpa_onnx::{LinearResampler, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::path::Path;

const RATE: usize = 16_000;
const WINDOW: usize = 512;
const MAX_SAMPLES: usize = RATE * 60;

pub(super) enum Utterance {
    Complete(SpeechAudio),
    TooLong,
}

pub(super) struct SpeechCapture {
    resampler: LinearResampler,
    vad: VoiceActivityDetector,
    pending: Vec<f32>,
    seen: usize,
    started: Option<usize>,
}
impl SpeechCapture {
    pub(super) fn new(rate: u32, directory: &Path) -> Result<Self, VoiceError> {
        let fail = || {
            VoiceError::new(
                VoiceErrorKind::DetectionFailed,
                "Could not initialize voice activity detection.",
            )
        };
        let resampler = LinearResampler::create(rate as i32, RATE as i32).ok_or_else(fail)?;
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(
                    directory
                        .join("silero_vad.onnx")
                        .to_str()
                        .ok_or_else(fail)?
                        .to_owned(),
                ),
                threshold: 0.5,
                min_silence_duration: 0.8,
                min_speech_duration: 0.25,
                window_size: WINDOW as i32,
                max_speech_duration: 64.,
            },
            sample_rate: RATE as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&config, 65.).ok_or_else(fail)?;
        Ok(Self {
            resampler,
            vad,
            pending: Vec::new(),
            seen: 0,
            started: None,
        })
    }
    pub(super) fn restart(&mut self) {
        self.resampler.reset();
        self.vad.reset();
        self.pending.clear();
        self.seen = 0;
        self.started = None;
    }
    pub(super) fn accept(&mut self, samples: &[f32]) -> Vec<Utterance> {
        let samples = self.resampler.resample(samples, false);
        self.accept_resampled(&samples)
    }
    fn accept_resampled(&mut self, samples: &[f32]) -> Vec<Utterance> {
        self.pending.extend_from_slice(samples);
        let consumed = self.pending.len() / WINDOW * WINDOW;
        let mut utterances = Vec::new();
        for frame in self.pending[..consumed].chunks_exact(WINDOW) {
            self.vad.accept_waveform(frame);
            self.seen += frame.len();
            if self.vad.detected() && self.started.is_none() {
                self.started = Some(self.seen.saturating_sub(RATE / 4));
            }
            while let Some(segment) = self.vad.front() {
                utterances.push(if segment.samples().len() > MAX_SAMPLES {
                    Utterance::TooLong
                } else {
                    Utterance::Complete(wav(segment.samples()))
                });
                self.vad.pop();
                self.started = None;
            }
            if self
                .started
                .is_some_and(|start| self.seen - start >= MAX_SAMPLES)
            {
                utterances.push(Utterance::TooLong);
                self.vad.reset();
                self.started = None;
                self.seen = 0;
            } else if self.started.is_none() && self.seen >= RATE * 60 {
                self.vad.reset();
                self.seen = 0;
            }
        }
        self.pending.drain(..consumed);
        utterances
    }
}

fn wav(samples: &[f32]) -> SpeechAudio {
    debug_assert!(samples.len() <= MAX_SAMPLES);
    let count = samples.len();
    let bytes = (count * 2) as u32;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&(RATE as u32).to_le_bytes());
    wav.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for sample in samples.iter().take(count) {
        let pcm = (sample.clamp(-1., 1.) * 32767.).round() as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    SpeechAudio { wav }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resources() -> std::path::PathBuf {
        std::env::var_os("RELAY_TEST_VOICE_RESOURCES")
            .map(Into::into)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/relay-resources/voice")
            })
    }
    #[test]
    fn silence_and_low_background_noise_do_not_submit_a_turn() {
        let mut capture = SpeechCapture::new(48_000, &resources()).unwrap();
        assert!(capture.accept(&vec![0.; 48_000 * 65]).is_empty());
        let noise: Vec<f32> = (0..48_000 * 5)
            .map(|index| ((index * 7919 % 1009) as f32 / 504. - 1.) * 0.002)
            .collect();
        assert!(capture.accept(&noise).is_empty());
        assert!(capture.pending.len() < WINDOW);
    }
    #[test]
    fn chinese_and_english_speech_wait_for_a_pause_then_emit_once() {
        for name in ["speech-zh.wav", "speech-en.wav"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let wave = sherpa_onnx::Wave::read(path.to_str().unwrap()).unwrap();
            let mut capture = SpeechCapture::new(wave.sample_rate() as u32, &resources()).unwrap();
            let mut clips = Vec::new();
            for chunk in wave.samples().chunks(193) {
                clips.extend(capture.accept(chunk));
            }
            clips.extend(capture.accept(&vec![0.; wave.sample_rate() as usize]));
            assert_eq!(clips.len(), 1, "{name}");
            let Utterance::Complete(audio) = &clips[0] else {
                panic!("short speech was rejected");
            };
            assert_eq!(&audio.wav[..4], b"RIFF");
            assert_eq!(
                u32::from_le_bytes(audio.wav[24..28].try_into().unwrap()),
                16_000
            );
            assert!(audio.wav.len() > 44 + 16_000);
            assert!(
                capture
                    .accept(&vec![0.; wave.sample_rate() as usize * 2])
                    .is_empty()
            );
            capture.restart();
            assert!(capture.pending.is_empty());
        }
    }
    #[test]
    fn brief_pauses_stay_in_one_turn_and_continuous_overlong_speech_is_rejected() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/speech-zh.wav");
        let wave = sherpa_onnx::Wave::read(path.to_str().unwrap()).unwrap();
        assert_eq!(wave.sample_rate(), RATE as i32);
        let samples = wave.samples();
        let start = samples.iter().position(|value| value.abs() > 0.02).unwrap();
        let end = samples
            .iter()
            .rposition(|value| value.abs() > 0.02)
            .unwrap()
            + 1;
        let speech = &samples[start..end];
        let mut capture = SpeechCapture::new(RATE as u32, &resources()).unwrap();
        assert!(capture.accept(speech).is_empty());
        assert!(capture.accept(&vec![0.; RATE * 3 / 10]).is_empty());
        assert!(capture.accept(speech).is_empty());
        assert_eq!(capture.accept(&vec![0.; RATE]).len(), 1);
        capture.restart();
        for _ in 0..(RATE * 65 / speech.len() + 1) {
            let clips = capture.accept(speech);
            if !clips.is_empty() {
                assert_eq!(clips.len(), 1);
                assert!(matches!(clips[0], Utterance::TooLong));
                return;
            }
        }
        panic!("continuous speech must be bounded");
    }
}
