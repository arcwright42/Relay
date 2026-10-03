//! Run after `cargo xtask prepare-voice`; no microphone access is requested.
fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let directory = args
        .next()
        .ok_or("usage: wake_probe MODEL_DIRECTORY WAV [WAV ...]")?;
    for wav in args {
        let count = relay_platform::probe_wake_file(
            std::path::Path::new(&directory),
            std::path::Path::new(&wav),
        )
        .map_err(|error| error.detail)?;
        println!(
            "{}: {count} wake candidates",
            std::path::Path::new(&wav).display()
        );
    }
    Ok(())
}
