use super::{config::Config, transport::*};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio_tungstenite::tungstenite::Message;

pub(super) const MODEL: &str = "qwen-audio-3.1-asr-flash-message";

pub(super) async fn transcribe(config: &Config, wav: &[u8]) -> Result<String, String> {
    let pcm = pcm(wav)?;
    let id = uuid::Uuid::new_v4().to_string();
    let socket = start(
        config,
        &id,
        json!({
            "task_group":"audio", "task":"asr", "function":"recognition", "model":MODEL,
            "parameters": {"format":"pcm", "sample_rate":16000,
                "intermediate_result_enabled":false, "disfluency_removal_enabled":false,
                "vocabulary":{"Relay":5,"Jev":5}, "max_sentence_silence":800}, "input":{}
        }),
    )
    .await?;
    let (mut send, mut receive) = socket.split();
    let upload = async {
        for chunk in pcm.chunks(16_000) {
            send.send(Message::Binary(chunk.to_vec().into()))
                .await
                .map_err(network_error)?;
        }
        send.send(finish(&id)).await.map_err(network_error)
    };
    let download = async {
        let mut sentences = Sentences::default();
        while let Some(incoming) = receive.next().await {
            match incoming.map_err(network_error)? {
                Message::Text(text) => {
                    let event = event(&text, &id)?;
                    match event["header"]["event"].as_str() {
                        Some("result-generated") => sentences.accept(&event)?,
                        Some("task-finished") => return Ok(sentences.text()),
                        _ => {}
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                _ => return Err("Voice transcription ended before completion.".into()),
            }
        }
        Err("Voice transcription ended before completion.".into())
    };
    let (_, transcript) = futures_util::try_join!(upload, download)?;
    Ok(transcript)
}

#[derive(Default)]
pub(super) struct Sentences(BTreeMap<u64, String>);
impl Sentences {
    pub fn accept(&mut self, event: &Value) -> Result<(), String> {
        let sentence = &event["payload"]["output"]["sentence"];
        if sentence["heartbeat"] == true || sentence["sentence_end"] != true {
            return Ok(());
        }
        let Some(id) = sentence["sentence_id"].as_u64().filter(|id| *id > 0) else {
            return Err("Invalid transcription sentence identifier.".into());
        };
        let text = sentence["text"]
            .as_str()
            .ok_or("Invalid transcription text.")?;
        if text.len() > 32_768 || self.0.len() >= 256 && !self.0.contains_key(&id) {
            return Err("Transcription exceeded its size limit.".into());
        }
        self.0.insert(id, text.trim().to_owned());
        if self.0.values().map(String::len).sum::<usize>() > 32_768 {
            return Err("Transcription exceeded its size limit.".into());
        }
        Ok(())
    }
    pub fn text(self) -> String {
        self.0
            .into_values()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Validate the boundary, including extra RIFF chunks, before uploading anything.
pub(super) fn pcm(wav: &[u8]) -> Result<&[u8], String> {
    let invalid = || "Expected mono 16 kHz PCM16 WAV, at most 60 seconds.".to_string();
    if wav.len() < 44
        || wav.len() > 1_920_000 + 65_536
        || &wav[..4] != b"RIFF"
        || &wav[8..12] != b"WAVE"
    {
        return Err(invalid());
    }
    let length = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize + 8;
    if length != wav.len() {
        return Err(invalid());
    }
    let mut offset = 12;
    let mut valid_format = false;
    let mut audio = None;
    while offset + 8 <= length {
        let tag = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let begin = offset + 8;
        let end = begin
            .checked_add(size)
            .filter(|end| *end <= length)
            .ok_or_else(invalid)?;
        let data = &wav[begin..end];
        if tag == b"fmt " {
            if data.len() < 16 {
                return Err(invalid());
            }
            valid_format = data[..4] == [1, 0, 1, 0]
                && data[4..8] == 16_000_u32.to_le_bytes()
                && data[8..12] == 32_000_u32.to_le_bytes()
                && data[12..16] == [2, 0, 16, 0];
        } else if tag == b"data" {
            if audio.is_some() || size == 0 || size > 1_920_000 || !size.is_multiple_of(2) {
                return Err(invalid());
            }
            audio = Some(data);
        }
        offset = end + size % 2;
    }
    if !valid_format {
        return Err(invalid());
    }
    audio.ok_or_else(invalid)
}
