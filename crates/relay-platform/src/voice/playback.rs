//! Bounded streaming PCM playback on the current macOS output device.
use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use relay_core::voice::{SpeechAudioOutput, SpeechAudioStream};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct MacAudioOutput;
struct Playing {
    _stream: cpal::Stream,
    audio: Arc<Mutex<Buffer>>,
    failed: Arc<AtomicBool>,
}
struct Buffer {
    samples: VecDeque<f32>,
    capacity: usize,
    phase: f64,
    last_audio: Instant,
}
impl Buffer {
    fn new(rate: u32) -> Self {
        let capacity = rate as usize * 4;
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
            phase: 0.,
            last_audio: Instant::now(),
        }
    }
    fn next(&mut self, step: f64) -> f32 {
        let Some(&first) = self.samples.front() else {
            self.phase = 0.;
            return 0.;
        };
        let second = self.samples.get(1).copied().unwrap_or(first);
        let value = first + (second - first) * self.phase as f32;
        self.phase += step;
        while self.phase >= 1. {
            self.samples.pop_front();
            self.phase -= 1.;
        }
        self.last_audio = Instant::now();
        value
    }
}
impl SpeechAudioOutput for MacAudioOutput {
    fn open(&self, sample_rate: u32) -> Result<Box<dyn SpeechAudioStream>, String> {
        if !(8_000..=48_000).contains(&sample_rate) {
            return Err("Unsupported speech sample rate.".into());
        }
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device is available.")?;
        let supported = device
            .default_output_config()
            .map_err(|_| "Could not read output device format.")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        if config.channels == 0 || !(8_000..=192_000).contains(&config.sample_rate.0) {
            return Err("Unsupported audio output format.".into());
        }
        let audio = Arc::new(Mutex::new(Buffer::new(sample_rate)));
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match format {
            SampleFormat::F32 => {
                build::<f32>(&device, &config, sample_rate, audio.clone(), failed.clone())
            }
            SampleFormat::I16 => {
                build::<i16>(&device, &config, sample_rate, audio.clone(), failed.clone())
            }
            SampleFormat::U16 => {
                build::<u16>(&device, &config, sample_rate, audio.clone(), failed.clone())
            }
            SampleFormat::I32 => {
                build::<i32>(&device, &config, sample_rate, audio.clone(), failed.clone())
            }
            SampleFormat::F64 => {
                build::<f64>(&device, &config, sample_rate, audio.clone(), failed.clone())
            }
            _ => return Err("Unsupported audio output sample format.".into()),
        }?;
        stream
            .play()
            .map_err(|_| "Could not start audio playback.")?;
        Ok(Box::new(Playing {
            _stream: stream,
            audio,
            failed,
        }))
    }
}
impl SpeechAudioStream for Playing {
    fn try_write(&mut self, samples: &[i16]) -> Result<bool, String> {
        if self.failed.load(Ordering::Acquire) {
            return Err("Audio output device failed.".into());
        }
        let mut audio = self
            .audio
            .lock()
            .map_err(|_| "Audio playback buffer failed.")?;
        if samples.len() > audio.capacity {
            return Err("Audio chunk exceeds playback buffer.".into());
        }
        if audio.samples.len() + samples.len() > audio.capacity {
            return Ok(false);
        }
        audio
            .samples
            .extend(samples.iter().map(|&s| s as f32 / 32768.));
        Ok(true)
    }
    fn is_drained(&self) -> Result<bool, String> {
        if self.failed.load(Ordering::Acquire) {
            return Err("Audio output device failed.".into());
        }
        let audio = self
            .audio
            .lock()
            .map_err(|_| "Audio playback buffer failed.")?;
        // Include the hardware buffer before releasing capture's echo gate.
        Ok(audio.samples.is_empty() && audio.last_audio.elapsed() >= Duration::from_millis(150))
    }
}
fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    source_rate: u32,
    audio: Arc<Mutex<Buffer>>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let step = f64::from(source_rate) / f64::from(config.sample_rate.0);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                // The realtime callback never waits for network work or a producer lock.
                if let Ok(mut audio) = audio.try_lock() {
                    for frame in data.chunks_mut(channels) {
                        frame.fill(T::from_sample(audio.next(step)));
                    }
                } else {
                    data.fill(T::from_sample(0.));
                }
            },
            move |_| {
                failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|_| "Could not open audio output device.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_resampling_handles_upsampling_downsampling_and_gaps() {
        let mut buffer = Buffer::new(24_000);
        buffer.samples.extend([0., 1., 0.]);
        let out: Vec<_> = (0..6).map(|_| buffer.next(0.5)).collect();
        assert_eq!(out, [0., 0.5, 1., 0.5, 0., 0.]);
        assert!(buffer.samples.is_empty());
        assert_eq!(buffer.next(0.5), 0.);
        buffer.samples.extend([0., 0.5, 1., 0.5]);
        assert_eq!(buffer.next(2.), 0.);
        assert_eq!(buffer.next(2.), 1.);
        assert!(buffer.samples.is_empty());
    }
}
