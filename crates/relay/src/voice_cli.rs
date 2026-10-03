//! Explicit, headless checks. Agent execution requires the roundtrip command.
mod roundtrip;
use relay_core::{
    settings::Language,
    voice::{SpeechAudio, SpeechOutput, SpeechTranscriber},
};
use std::{
    io::{BufRead, Read},
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

pub(super) fn run(root: &Path) -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        return false;
    };
    if !matches!(
        command.as_str(),
        "--voice-save-key" | "--voice-check" | "--voice-roundtrip-check"
    ) {
        return false;
    }
    let result = match (command.as_str(), args.len()) {
        ("--voice-save-key", 1) => save(root),
        ("--voice-check", 2) => check(root, Path::new(&args[1])),
        ("--voice-roundtrip-check", 2) => read_audio(Path::new(&args[1])).and_then(|audio| roundtrip::check(root, audio)),
        _ => Err(
            "Usage: --voice-save-key (key on stdin), --voice-check <WAV>, or --voice-roundtrip-check <WAV> (disposable Agent session).".into(),
        ),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
    true
}
fn save(root: &Path) -> Result<(), String> {
    let mut key = String::new();
    std::io::stdin()
        .lock()
        .take(4098)
        .read_line(&mut key)
        .map_err(|_| "Could not read key from stdin.")?;
    relay_runtime::save_voice_key(root, &key)?;
    println!("Voice API key saved to macOS Keychain.");
    Ok(())
}
fn check(root: &Path, wav: &Path) -> Result<(), String> {
    let audio = read_audio(wav)?;
    let speech = relay_runtime::QwenSpeech::new(root, Arc::new(relay_platform::MacAudioOutput));
    let cancelled = AtomicBool::new(false);
    let transcript = speech.transcribe(&SpeechAudio { wav: audio }, &cancelled)?;
    println!("ASR: {}", transcript);
    speech.speak(
        "Relay 的语音识别和语音播报已连接。",
        Language::SimplifiedChinese,
        &cancelled,
    )?;
    println!("TTS: playback complete.");
    Ok(())
}
fn read_audio(wav: &Path) -> Result<Vec<u8>, String> {
    let mut audio = Vec::new();
    std::fs::File::open(wav)
        .map_err(|_| "Could not open ASR check WAV.")?
        .take(2_000_000)
        .read_to_end(&mut audio)
        .map_err(|_| "Could not read ASR check WAV.")?;
    Ok(audio)
}
