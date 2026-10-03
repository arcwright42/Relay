use relay_core::voice::{VoiceError, VoiceErrorKind};
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};
use std::{collections::VecDeque, path::Path};

pub(super) struct Detector {
    // Drop the stream before its native engine.
    stream: OnlineStream,
    engine: KeywordSpotter,
    recent: VecDeque<f32>,
    frames: u64,
    quiet_frames: u64,
    paused: bool,
}

impl Detector {
    pub(super) fn new(directory: &Path) -> Result<Self, VoiceError> {
        let file = |name: &str| {
            directory
                .join(name)
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| failed("Model path is not UTF-8."))
        };
        let mut config = KeywordSpotterConfig::default();
        config.model_config.transducer.encoder = Some(file("encoder.onnx")?);
        config.model_config.transducer.decoder = Some(file("decoder.onnx")?);
        config.model_config.transducer.joiner = Some(file("joiner.onnx")?);
        config.model_config.tokens = Some(file("tokens.txt")?);
        config.model_config.provider = Some("cpu".into());
        config.model_config.num_threads = 1;
        config.max_active_paths = 8;
        config.keywords_file = Some(file("keywords.txt")?);
        let engine = KeywordSpotter::create(&config)
            .ok_or_else(|| failed("Could not initialize the local wake detector."))?;
        let stream = engine.create_stream();
        Ok(Self {
            engine,
            stream,
            recent: VecDeque::new(),
            frames: 0,
            quiet_frames: 0,
            paused: false,
        })
    }

    pub(super) fn restart(&mut self) {
        self.stream = self.engine.create_stream();
        self.recent.clear();
        self.frames = 0;
        self.quiet_frames = 0;
        self.paused = false;
    }

    pub(super) fn accept(&mut self, rate: u32, samples: &[f32]) -> bool {
        if samples.is_empty() {
            return false;
        }
        let energy =
            samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
        // Quiet input (below -50 dBFS) carries little useful phonetic
        // context. Flush a full second for trailing blanks, then stop feeding
        // silence so a later word starts from fresh model state. A half-second
        // pre-roll preserves the cold-start context without unbounded silence.
        if energy < 1e-5 {
            self.quiet_frames += samples.len() as u64;
            if self.paused {
                self.recent.extend(samples.iter().copied());
                self.retain_recent(rate as usize / 2);
                return false;
            }
        } else {
            self.quiet_frames = 0;
            if self.paused {
                self.paused = false;
                let (first, second) = self.recent.as_slices();
                self.stream.accept_waveform(rate as i32, first);
                if !second.is_empty() {
                    self.stream.accept_waveform(rate as i32, second);
                }
                self.frames = self.recent.len() as u64;
            }
        }
        // Periodically release stream history so always-on operation has bounded
        // memory. Replaying two seconds preserves words across the boundary;
        // the application cooldown suppresses any replayed wake candidate.
        if self.frames >= u64::from(rate) * 45 {
            self.stream = self.engine.create_stream();
            let (first, second) = self.recent.as_slices();
            self.stream.accept_waveform(rate as i32, first);
            self.stream.accept_waveform(rate as i32, second);
            self.frames = self.recent.len() as u64;
        }
        self.stream.accept_waveform(rate as i32, samples);
        self.frames += samples.len() as u64;
        self.recent.extend(samples.iter().copied());
        let keep = (rate as usize * 2).min(384_000);
        self.retain_recent(keep);
        let mut detected = false;
        while self.engine.is_ready(&self.stream) {
            self.engine.decode(&self.stream);
            if self
                .engine
                .get_result(&self.stream)
                .is_some_and(|result| result.keyword == "hey_relay")
            {
                detected = true;
                self.engine.reset(&self.stream);
            }
        }
        if self.quiet_frames >= u64::from(rate) {
            self.stream = self.engine.create_stream();
            self.retain_recent(rate as usize / 2);
            self.frames = 0;
            self.paused = true;
        }
        detected
    }

    fn retain_recent(&mut self, keep: usize) {
        if self.recent.len() > keep {
            self.recent.drain(..self.recent.len() - keep);
        }
    }
}

fn failed(detail: impl Into<String>) -> VoiceError {
    VoiceError::new(VoiceErrorKind::DetectionFailed, detail)
}
