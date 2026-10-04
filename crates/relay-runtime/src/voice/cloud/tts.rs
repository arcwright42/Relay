use super::{config::Config, transport::*};
use futures_util::{SinkExt, StreamExt};
use relay_core::voice::SpeechAudioOutput;
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

pub(super) const MODEL: &str = "qwen-audio-3.1-tts-flash";
const RATE: u32 = 24_000;

pub(super) async fn speak(
    config: &Config,
    text: &str,
    output: &dyn SpeechAudioOutput,
) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let voice = if text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        "longanhuan_v3.1"
    } else {
        "Annie_v3.1"
    };
    let mut socket = start(config, &id, json!({
        "task_group":"audio", "task":"tts", "function":"SpeechSynthesizer", "model":MODEL,
        "parameters":{"text_type":"PlainText", "voice":voice, "format":"pcm", "sample_rate":RATE}, "input":{}
    })).await?;
    socket
        .send(message(
            json!({"header":header("continue-task", &id), "payload":{"input":{"text":text}}}),
        ))
        .await
        .map_err(network_error)?;
    socket.send(finish(&id)).await.map_err(network_error)?;
    // The platform owns its output device; networking never knows native handles.
    let mut playback = output.open(RATE)?;
    let mut decoder = PcmDecoder::default();
    while let Some(incoming) = socket.next().await {
        match incoming.map_err(network_error)? {
            Message::Binary(bytes) => {
                let samples = decoder.decode(&bytes)?;
                for chunk in samples.chunks(4096) {
                    while !playback.try_write(chunk)? {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
            Message::Text(text) => {
                if event(&text, &id)?["header"]["event"] == "task-finished" {
                    decoder.finish()?;
                    // Release the cloud connection before waiting for the speaker.
                    drop(socket);
                    while !playback.is_drained()? {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    return Ok(());
                }
            }
            Message::Ping(bytes) => socket
                .send(Message::Pong(bytes))
                .await
                .map_err(network_error)?,
            Message::Pong(_) => {}
            _ => return Err("Voice synthesis ended before completion.".into()),
        }
    }
    Err("Voice synthesis ended before completion.".into())
}

#[derive(Default)]
pub(super) struct PcmDecoder {
    pending: Option<u8>,
    bytes: usize,
}
impl PcmDecoder {
    pub fn decode(&mut self, bytes: &[u8]) -> Result<Vec<i16>, String> {
        self.bytes += bytes.len();
        if self.bytes > RATE as usize * 2 * 300 {
            return Err("Synthesized speech exceeded five minutes.".into());
        }
        let mut samples = Vec::with_capacity(bytes.len().div_ceil(2));
        for &byte in bytes {
            if let Some(first) = self.pending.take() {
                samples.push(i16::from_le_bytes([first, byte]));
            } else {
                self.pending = Some(byte);
            }
        }
        Ok(samples)
    }
    pub fn finish(&self) -> Result<(), String> {
        if self.pending.is_some() || self.bytes == 0 {
            Err("Voice service returned incomplete or empty audio.".into())
        } else {
            Ok(())
        }
    }
}
