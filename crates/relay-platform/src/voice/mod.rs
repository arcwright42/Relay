//! Native microphone capture and sherpa-onnx stay behind the domain backend port.
mod activity;
mod audio;
mod detector;
mod output;
mod permission;
mod playback;
mod speech;
pub use output::MacSpeechOutput;
pub use playback::MacAudioOutput;

use relay_core::voice::*;
use std::{
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub struct MacVoiceBackend;

impl VoiceInputBackend for MacVoiceBackend {
    fn listen(
        &self,
        directory: &Path,
        control: &VoiceCaptureControl,
        emit: &dyn Fn(VoiceInputEvent),
    ) -> Result<(), VoiceError> {
        let cancelled = &control.cancelled;
        if cancelled.load(Ordering::Acquire) || !permission::authorize(cancelled, emit)? {
            return Ok(());
        }
        let mut detector = detector::Detector::new(directory)?;
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut capture = audio::Capture::start()?;
        let mut speech = speech::SpeechCapture::new(capture.sample_rate(), directory)?;
        let _activity = activity::ListeningActivity::begin();
        emit(VoiceInputEvent::InputDevice(
            capture.device_name().to_owned(),
        ));
        emit(VoiceInputEvent::Listening);
        let mut checked_device = Instant::now();
        let mut last_level = Instant::now();
        let mut session = 0;
        let mut suspended = false;
        let mut capture_epoch = 0;
        let mut pre_roll = std::collections::VecDeque::<f32>::new();
        #[cfg(debug_assertions)]
        let mut telemetry = (Instant::now(), 0_usize, 0_f32, 0_f64);
        while !cancelled.load(Ordering::Acquire) {
            if let Some(error) = capture.error() {
                return Err(error);
            }
            if checked_device.elapsed() >= Duration::from_secs(1) {
                checked_device = Instant::now();
                if capture.default_device_changed() {
                    capture = audio::Capture::start()?;
                    speech = speech::SpeechCapture::new(capture.sample_rate(), directory)?;
                    detector.restart();
                    pre_roll.clear();
                    emit(VoiceInputEvent::InputDevice(
                        capture.device_name().to_owned(),
                    ));
                    emit(VoiceInputEvent::Listening);
                }
            }
            if capture.discard_gap() {
                eprintln!("voice: capture gap; restarting audio state");
                detector.restart();
                speech.restart();
                pre_roll.clear();
            }
            if let Some(samples) = capture.next()? {
                let blocked = control.paused.load(Ordering::Acquire)
                    || control.speaking.load(Ordering::Acquire);
                let epoch = control.capture_epoch.load(Ordering::Acquire);
                if blocked != suspended || epoch != capture_epoch {
                    suspended = blocked;
                    capture_epoch = epoch;
                    detector.restart();
                    speech.restart();
                    pre_roll.clear();
                }
                if blocked {
                    continue;
                }
                let requested = control.session.load(Ordering::Acquire);
                if requested != session {
                    detector.restart();
                    speech.restart();
                    session = requested;
                    if session != 0 && control.wake_preroll.load(Ordering::Acquire) {
                        let (first, second) = pre_roll.as_slices();
                        for clip in speech
                            .accept(first)
                            .into_iter()
                            .chain(speech.accept(second))
                        {
                            emit_utterance(session, clip, emit);
                            if control.paused.load(Ordering::Acquire) {
                                break;
                            }
                        }
                    }
                }
                let energy = samples.iter().map(|sample| sample * sample).sum::<f32>()
                    / samples.len().max(1) as f32;
                if last_level.elapsed() >= Duration::from_millis(50) {
                    last_level = Instant::now();
                    let level =
                        ((10. * energy.max(1e-9).log10() + 90.) / 90. * 100.).clamp(0., 100.) as u8;
                    emit(VoiceInputEvent::InputLevel {
                        session_id: session,
                        level,
                    });
                }
                #[cfg(debug_assertions)]
                {
                    telemetry.1 += samples.len();
                    telemetry.2 = samples
                        .iter()
                        .fold(telemetry.2, |peak, sample| peak.max(sample.abs()));
                    telemetry.3 += samples
                        .iter()
                        .map(|sample| f64::from(*sample).powi(2))
                        .sum::<f64>();
                }
                if control.paused.load(Ordering::Acquire) {
                    continue;
                } else if session == 0 {
                    if detector.accept(capture.sample_rate(), &samples) {
                        eprintln!("voice: wake candidate detected");
                        emit(VoiceInputEvent::Detected);
                    }
                } else {
                    for clip in speech.accept(&samples) {
                        emit_utterance(session, clip, emit);
                        if control.paused.load(Ordering::Acquire) {
                            break;
                        }
                    }
                }
                pre_roll.extend(samples.iter().copied());
                let keep = capture.sample_rate() as usize;
                if pre_roll.len() > keep {
                    pre_roll.drain(..pre_roll.len() - keep);
                }
            }
            #[cfg(debug_assertions)]
            if telemetry.0.elapsed() >= Duration::from_secs(2) {
                let rms = (telemetry.3 / telemetry.1.max(1) as f64).sqrt();
                eprintln!(
                    "voice: input samples={} peak={:.5} rms={rms:.5} session={session}",
                    telemetry.1, telemetry.2
                );
                telemetry = (Instant::now(), 0, 0., 0.);
            }
        }
        Ok(()) // Dropping Capture releases Core Audio; unfinished speech is discarded.
    }
}

fn emit_utterance(session_id: u64, utterance: speech::Utterance, emit: &dyn Fn(VoiceInputEvent)) {
    emit(match utterance {
        speech::Utterance::Complete(audio) => VoiceInputEvent::Speech { session_id, audio },
        speech::Utterance::TooLong => VoiceInputEvent::SpeechTooLong { session_id },
    });
}

/// Developer verification uses the same incremental detector without microphone access.
pub fn probe_wake_file(directory: &Path, wav: &Path) -> Result<usize, VoiceError> {
    let path = wav.to_str().ok_or_else(|| {
        VoiceError::new(VoiceErrorKind::DetectionFailed, "WAV path is not UTF-8.")
    })?;
    let wave = sherpa_onnx::Wave::read(path).ok_or_else(|| {
        VoiceError::new(
            VoiceErrorKind::DetectionFailed,
            "Could not read a PCM WAV file.",
        )
    })?;
    let mut detector = detector::Detector::new(directory)?;
    let rate = wave.sample_rate();
    if !(8_000..=192_000).contains(&rate) {
        return Err(VoiceError::new(
            VoiceErrorKind::DetectionFailed,
            "WAV sample rate must be between 8 and 192 kHz.",
        ));
    }
    let mut count = 0;
    for chunk in wave.samples().chunks((rate as usize / 50).max(1)) {
        count += usize::from(detector.accept(rate as u32, chunk));
    }
    count += usize::from(detector.accept(rate as u32, &vec![0.; rate as usize]));
    Ok(count)
}
