//! macOS's installed speech voices, with text on stdin and cancellable playback.
use relay_core::{
    settings::Language,
    voice::{SpeechOutput, spoken_text},
};
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub struct MacSpeechOutput;

impl SpeechOutput for MacSpeechOutput {
    fn speak(&self, text: &str, language: Language, cancelled: &AtomicBool) -> Result<(), String> {
        let text = spoken_text(text, language);
        if cancelled.load(Ordering::Acquire) || text.is_empty() {
            return Ok(());
        }
        let voice = if text
            .chars()
            .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
        {
            "Tingting"
        } else {
            "Samantha"
        };
        let mut child = Command::new("/usr/bin/say")
            .args(["-v", voice, "-r", "190"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Could not start macOS speech: {error}"))?;
        let written = child
            .stdin
            .take()
            .ok_or_else(|| "Missing speech input pipe.".to_string())
            .and_then(|mut input| {
                input
                    .write_all(text.as_bytes())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return if cancelled.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err("Speech playback timed out.".into())
                };
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("macOS speech playback failed: {status}")),
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.to_string());
                }
            }
        }
    }
}
