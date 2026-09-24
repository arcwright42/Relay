//! One-shot, bounded Moli fetches. Failure never prevents submitting a user prompt.
use relay_core::capture::WebFetchService;
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 256 * 1024;
const BODY_CHARS: usize = 12_000;

pub struct MoliFetcher {
    executable: PathBuf,
    installer: Option<crate::moli_installer::MoliInstaller>,
    active: AtomicUsize,
    stopping: AtomicBool,
    timeout: Duration,
}
impl Default for MoliFetcher {
    fn default() -> Self {
        let executable = std::env::var_os("RELAY_MOLI_PATH")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/bin/moli"))
                    .filter(|path| path.is_file())
            })
            .unwrap_or_else(|| PathBuf::from("moli"));
        Self {
            executable,
            installer: None,
            active: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
            timeout: Duration::from_secs(9),
        }
    }
}
impl MoliFetcher {
    pub fn new(directory: &std::path::Path) -> Self {
        let mut fetcher = Self::default();
        if std::env::var_os("RELAY_MOLI_PATH").is_none() {
            fetcher.installer = Some(crate::moli_installer::MoliInstaller::new(directory));
        }
        fetcher
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.active.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
    }
}
struct ActiveFetch<'a>(&'a AtomicUsize);
impl Drop for ActiveFetch<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn body(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > OUTPUT_LIMIT {
        return Err("Web page exceeded the capture limit".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Web page was not valid UTF-8")?;
    if text.trim().is_empty() {
        return Err("Web page returned no text".into());
    }
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(BODY_CHARS).collect();
    if chars.next().is_some() {
        result.push_str("\n[网页正文已截取：仅保留前 12,000 字]");
    }
    Ok(result)
}
impl WebFetchService for MoliFetcher {
    fn fetch(&self, url: &str) -> Result<String, String> {
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 8192 {
            return Err("Unsupported web URL".into());
        }
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| "Web fetch is busy; continuing with selected text")?;
        let _active = ActiveFetch(&self.active);
        if self.stopping.load(Ordering::Acquire) {
            return Err("Relay is shutting down".into());
        }
        let executable = if let Some(installer) = &self.installer {
            installer
                .prepare(&|| self.stopping.load(Ordering::Acquire))
                .map_err(|_| "Moli preparation unavailable; continuing with selected text")?
        } else {
            self.executable.clone()
        };
        let staging = crate::installer::Staging::new(&std::env::temp_dir())
            .map_err(|_| "Could not prepare web capture")?;
        let mut output_options = std::fs::OpenOptions::new();
        output_options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            output_options.mode(0o600);
        }
        let output = output_options
            .open(staging.0.join("page.txt"))
            .map_err(|_| "Could not prepare web output")?;
        let mut command = Command::new(executable);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
            .args([
                "fetch",
                "--timeout",
                "8000",
                "--block-private-networks",
                "--wait-until",
                "done",
                "--dump",
                "markdown",
                url,
            ])
            .stdin(Stdio::null())
            .stdout(
                output
                    .try_clone()
                    .map_err(|_| "Could not prepare web output")?,
            )
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|_| "Moli is unavailable; continuing with selected text")?;
        let deadline = Instant::now() + self.timeout;
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err("Web fetch failed")
                    };
                }
                Ok(None)
                    if Instant::now() < deadline
                        && !self.stopping.load(Ordering::Acquire)
                        && output
                            .metadata()
                            .is_ok_and(|meta| meta.len() <= OUTPUT_LIMIT as u64) =>
                {
                    thread::sleep(Duration::from_millis(25))
                }
                _ => {
                    #[cfg(unix)]
                    let _ = Command::new("/bin/kill")
                        .args(["-KILL", "--", &format!("-{}", child.id())])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("Web fetch timed out");
                }
            }
        };
        result.map_err(str::to_owned)?;
        use std::io::{Seek, SeekFrom};
        let mut output = output;
        output
            .seek(SeekFrom::Start(0))
            .map_err(|_| "Could not read web output")?;
        let mut bytes = Vec::new();
        output
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Web fetch output failed")?;
        body(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_unicode_and_marks_truncation() {
        let result = body("字".repeat(12_001).into_bytes()).unwrap();
        assert!(result.starts_with(&"字".repeat(12_000)));
        assert!(result.contains("已截取"));
        assert!(body(vec![b'x'; OUTPUT_LIMIT + 1]).is_err());
        assert!(body(vec![]).is_err());
    }
    #[test]
    fn unavailable_binary_and_non_web_urls_fail_without_installation() {
        let fetcher = MoliFetcher {
            executable: PathBuf::from("/not-present/relay-moli"),
            ..Default::default()
        };
        assert!(fetcher.fetch("https://example.com").is_err());
        assert!(fetcher.fetch("file:///etc/hosts").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn process_errors_limits_and_timeout_are_bounded_and_reaped() {
        use std::os::unix::fs::PermissionsExt;
        let staging = crate::installer::Staging::new(&std::env::temp_dir()).unwrap();
        let executable = staging.0.join("fake-moli");
        let fetcher = MoliFetcher {
            executable: executable.clone(),
            timeout: Duration::from_millis(500),
            ..Default::default()
        };
        for (script, expected) in [
            ("#!/bin/sh\nprintf 'test page'\n", Some("test page")),
            ("#!/bin/sh\nprintf 'failure page'\nexit 2\n", None),
            ("#!/bin/sh\nexec /usr/bin/head -c 270000 /dev/zero\n", None),
            ("#!/bin/sh\nexec /bin/sleep 5\n", None),
        ] {
            std::fs::write(&executable, script).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let started = Instant::now();
            let result = fetcher.fetch("https://example.com");
            match expected {
                Some(text) => assert_eq!(result.unwrap(), text),
                None => assert!(result.is_err()),
            }
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(fetcher.active.load(Ordering::Acquire), 0);
        }
        fetcher.shutdown();
        assert!(fetcher.fetch("https://example.com").is_err());
    }
}
