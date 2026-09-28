use relay_core::ProjectId;
use std::{fs, io::Write, path::Path, time::SystemTime};

/// Persistent errors are available even when macOS launches Relay without a
/// terminal. Callers pass errors only, never prompts, selections or snapshots.
pub(crate) fn error(root: &Path, project: ProjectId, operation: &str, message: &str) {
    let entry = serde_json::json!({
        "time_unix_ms": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default().as_millis(),
        "pid": std::process::id(),
        "project": project.0,
        "operation": operation,
        "error": message,
    });
    eprintln!("relay error: {entry}");
    let result = (|| -> std::io::Result<()> {
        let directory = root.join("logs");
        fs::create_dir_all(&directory)?;
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(directory.join("runtime.jsonl"))?;
        file.write_all(format!("{entry}\n").as_bytes())
    })();
    if let Err(error) = result {
        eprintln!("relay error: could not write runtime log: {error}");
    }
}
