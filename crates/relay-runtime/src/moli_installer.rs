//! Fixed release, separate from Agent installation; never blocks prompt submission.
use crate::installer::{Staging, activate, run, verify_sha256};
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::Duration,
};

pub(crate) const VERSION: &str = "1.1.10";
pub(crate) struct MoliInstaller {
    root: PathBuf,
    gate: Mutex<()>,
}
impl MoliInstaller {
    pub fn new(directory: &Path) -> Self {
        Self {
            root: directory.join("components/moli"),
            gate: Mutex::new(()),
        }
    }
    pub fn prepare(&self, cancelled: &impl Fn() -> bool) -> Result<PathBuf> {
        let _gate = self
            .gate
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Moli is being prepared"))?;
        if cancelled() {
            bail!("Moli preparation cancelled");
        }
        let destination = self.root.join(VERSION);
        let executable = destination.join("moli");
        if executable.is_file() {
            return Ok(executable);
        }
        let (target, checksum) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => (
                "aarch64-apple-darwin",
                "bf98e9f01fcf504a26e7e5c9b10b0a02ea1196c59622c6a6fb9ba7f2c21c457a",
            ),
            ("macos", "x86_64") => (
                "x86_64-apple-darwin",
                "d2fb76cd298af45b9b518dd90b8037a2f0cc03ad07f7b823b68dc6d60882e8d4",
            ),
            _ => bail!("Managed Moli currently requires macOS"),
        };
        fs::create_dir_all(&self.root)?;
        let staging = Staging::new(&self.root)?;
        let archive = staging.0.join("moli.tar.gz");
        let url = format!(
            "https://github.com/lexmount/moli/releases/download/v{VERSION}/moli-{target}.tar.gz"
        );
        run(
            Command::new("/usr/bin/curl")
                .args([
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--tlsv1.2",
                    "-fsSL",
                    "--max-time",
                    "180",
                    "--max-filesize",
                    "104857600",
                    "--output",
                ])
                .arg(&archive)
                .arg(url),
            &staging.0,
            Duration::from_secs(185),
            cancelled,
        )?;
        verify_sha256(&archive, checksum)?;
        let package = staging.0.join("package");
        fs::create_dir(&package)?;
        run(
            Command::new("/usr/bin/tar")
                .arg("-xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&package)
                .arg("--strip-components=1"),
            &staging.0,
            Duration::from_secs(30),
            cancelled,
        )?;
        let version = run(
            Command::new(package.join("moli")).arg("--version"),
            &staging.0,
            Duration::from_secs(10),
            cancelled,
        )?;
        if version.trim() != format!("moli {VERSION}") {
            bail!("Unexpected Moli version");
        }
        if cancelled() {
            bail!("Moli preparation cancelled");
        }
        // Keep the release's licenses and notices alongside the executable.
        activate(&package, &destination)?;
        Ok(executable)
    }
}
