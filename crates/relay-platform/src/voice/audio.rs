use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use relay_core::voice::{VoiceError, VoiceErrorKind};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

const MAX_CALLBACK_SAMPLES: usize = 32_768;

pub(super) struct Capture {
    _stream: cpal::Stream,
    samples: mpsc::Receiver<Vec<f32>>,
    failed: Arc<Mutex<Option<String>>>,
    gap: Arc<AtomicBool>,
    rate: u32,
    device_name: String,
}

impl Capture {
    pub(super) fn start() -> Result<Self, VoiceError> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| unavailable("No input microphone is available."))?;
        let device_name = device.name().map_err(unavailable)?;
        let supported = device.default_input_config().map_err(unavailable)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        eprintln!(
            "voice: capture input={device_name:?} rate={} channels={} format={format:?}",
            config.sample_rate.0, config.channels
        );
        if config.channels == 0 || !(8_000..=192_000).contains(&config.sample_rate.0) {
            return Err(unavailable(
                "Unsupported microphone sample rate or channel count.",
            ));
        }
        let (sender, samples) = mpsc::sync_channel(16);
        let failed = Arc::new(Mutex::new(None));
        let gap = Arc::new(AtomicBool::new(false));
        let stream = match format {
            SampleFormat::F32 => {
                input::<f32>(&device, &config, sender, gap.clone(), failed.clone())
            }
            SampleFormat::I16 => {
                input::<i16>(&device, &config, sender, gap.clone(), failed.clone())
            }
            SampleFormat::U16 => {
                input::<u16>(&device, &config, sender, gap.clone(), failed.clone())
            }
            SampleFormat::I32 => {
                input::<i32>(&device, &config, sender, gap.clone(), failed.clone())
            }
            SampleFormat::F64 => {
                input::<f64>(&device, &config, sender, gap.clone(), failed.clone())
            }
            _ => {
                return Err(unavailable(format!(
                    "Unsupported microphone sample format: {format}"
                )));
            }
        }?;
        stream.play().map_err(unavailable)?;
        Ok(Self {
            _stream: stream,
            samples,
            failed,
            gap,
            rate: config.sample_rate.0,
            device_name,
        })
    }

    pub(super) fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub(super) fn device_name(&self) -> &str {
        &self.device_name
    }
    pub(super) fn error(&self) -> Option<VoiceError> {
        self.failed
            .lock()
            .expect("microphone error lock")
            .clone()
            .map(unavailable)
    }
    pub(super) fn default_device_changed(&self) -> bool {
        cpal::default_host()
            .default_input_device()
            .and_then(|device| device.name().ok())
            .is_none_or(|name| name != self.device_name)
    }
    pub(super) fn discard_gap(&self) -> bool {
        if !self.gap.swap(false, Ordering::AcqRel) {
            return false;
        }
        while self.samples.try_recv().is_ok() {}
        true
    }
    pub(super) fn next(&self) -> Result<Option<Vec<f32>>, VoiceError> {
        match self.samples.recv_timeout(Duration::from_millis(20)) {
            Ok(samples) => Ok(Some(samples)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(unavailable("Microphone capture disconnected."))
            }
        }
    }
}

fn input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sender: mpsc::SyncSender<Vec<f32>>,
    gap: Arc<AtomicBool>,
    failed: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, VoiceError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if data.len() > MAX_CALLBACK_SAMPLES {
                    gap.store(true, Ordering::Release);
                    return;
                }
                let mono = mono(data, channels);
                // No inference, I/O, UI calls or waiting in the Core Audio callback.
                if sender.try_send(mono).is_err() {
                    gap.store(true, Ordering::Release);
                }
            },
            move |error| {
                *failed.lock().expect("microphone error lock") = Some(error.to_string());
            },
            None,
        )
        .map_err(unavailable)
}

fn mono<T: Sample>(samples: &[T], channels: usize) -> Vec<f32>
where
    f32: FromSample<T>,
{
    samples
        .chunks_exact(channels)
        .map(|frame| {
            frame
                .iter()
                .map(|sample| {
                    let value: f32 = (*sample).to_sample();
                    if value.is_finite() {
                        value.clamp(-1., 1.)
                    } else {
                        0.
                    }
                })
                .sum::<f32>()
                / channels as f32
        })
        .collect()
}

fn unavailable(detail: impl ToString) -> VoiceError {
    VoiceError::new(VoiceErrorKind::MicrophoneUnavailable, detail.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multichannel_pcm_is_averaged_and_invalid_samples_are_sanitized() {
        assert_eq!(mono(&[0.5f32, -0.5, 1., 0.5], 2), vec![0., 0.75]);
        assert_eq!(
            mono(&[f32::NAN, f32::INFINITY, 4., -4.], 1),
            vec![0., 0., 1., -1.]
        );
        let pcm = mono(&[i16::MIN, i16::MAX], 1);
        assert_eq!(pcm[0], -1.);
        assert!(pcm[1] > 0.99 && pcm[1] <= 1.);
    }
}
