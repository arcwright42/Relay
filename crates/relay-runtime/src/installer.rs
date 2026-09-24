use anyhow::{Context, Result, bail};
use relay_acp::LaunchSpec;
use relay_core::agents::AgentSource;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const NODE_VERSION: &str = "24.21.0";
pub const ADAPTER_VERSION: &str = "1.12.0";
pub const CODEX_VERSION: &str = "0.154.0";
pub const RELEASE: &str = "codex-acp-1.12.0-codex-0.154.0";
const PACKAGE: &str = include_str!("../resources/codex/package.json");
const LOCK: &str = include_str!("../resources/codex/package-lock.json");

pub struct Installer {
    root: PathBuf,
    gate: Mutex<()>,
}

pub struct PreparedAgent {
    pub launch: LaunchSpec,
    pub version: String,
}

impl Installer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            gate: Mutex::new(()),
        }
    }
    fn node_dir(&self) -> PathBuf {
        self.root.join("components/node").join(NODE_VERSION)
    }
    fn agent_dir(&self) -> PathBuf {
        self.root.join("components/agents").join(RELEASE)
    }
    pub fn installed(&self) -> bool {
        self.node_dir().join("bin/node").is_file()
            && self
                .agent_dir()
                .join("node_modules/@agentclientprotocol/codex-acp/dist/index.js")
                .is_file()
            && fs::read_to_string(self.agent_dir().join("relay-lock.sha256"))
                .ok()
                .as_deref()
                == Some(&lock_hash())
    }

    /// Stage a complete version before activation. Existing versions are retained for rollback.
    pub fn prepare(
        &self,
        source: &AgentSource,
        progress: impl Fn(&str),
        cancelled: impl Fn() -> bool,
    ) -> Result<PreparedAgent> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Installer lock unavailable"))?;
        if cancelled() {
            bail!("Preparation cancelled");
        }
        let (platform, checksum) = node_archive()?;
        fs::create_dir_all(self.root.join("components"))?;
        if !self.node_dir().join("bin/node").is_file() {
            progress("Downloading runtime…");
            let staging = Staging::new(&self.root.join("components"))?;
            let archive_name = format!("node-v{NODE_VERSION}-{platform}.tar.gz");
            let archive = staging.0.join(&archive_name);
            let mut download = Command::new("/usr/bin/curl");
            download
                .args([
                    "--fail",
                    "--location",
                    "--proto",
                    "=https",
                    "--tlsv1.2",
                    "--retry",
                    "2",
                    "--connect-timeout",
                    "15",
                    "--max-time",
                    "240",
                    "--silent",
                    "--show-error",
                    "--output",
                ])
                .arg(&archive)
                .arg(format!(
                    "https://nodejs.org/dist/v{NODE_VERSION}/{archive_name}"
                ));
            run(
                &mut download,
                &staging.0,
                Duration::from_secs(260),
                &cancelled,
            )?;
            verify_sha256(&archive, checksum)?;
            progress("Preparing runtime…");
            let mut unpack = Command::new("/usr/bin/tar");
            unpack.arg("-xzf").arg(&archive).arg("-C").arg(&staging.0);
            run(&mut unpack, &staging.0, Duration::from_secs(60), &cancelled)?;
            let extracted = staging.0.join(format!("node-v{NODE_VERSION}-{platform}"));
            let mut check = Command::new(extracted.join("bin/node"));
            check.arg("--version");
            let version = run(&mut check, &staging.0, Duration::from_secs(10), &cancelled)?;
            if version.trim() != format!("v{NODE_VERSION}") {
                bail!("Downloaded Node version did not match the manifest");
            }
            activate(&extracted, &self.node_dir())?;
        }
        if !self.installed() {
            progress("Installing Codex…");
            let staging = Staging::new(&self.root.join("components"))?;
            fs::write(staging.0.join("package.json"), PACKAGE)?;
            fs::write(staging.0.join("package-lock.json"), LOCK)?;
            fs::write(staging.0.join("user.npmrc"), "")?;
            fs::write(staging.0.join("global.npmrc"), "")?;
            let mut install = Command::new(self.node_dir().join("bin/node"));
            install
                .arg(self.node_dir().join("lib/node_modules/npm/bin/npm-cli.js"))
                .args([
                    "ci",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                    "--registry=https://registry.npmjs.org",
                ])
                .arg("--userconfig")
                .arg(staging.0.join("user.npmrc"))
                .arg("--globalconfig")
                .arg(staging.0.join("global.npmrc"))
                .arg("--cache")
                .arg(self.root.join("cache/npm"))
                .current_dir(&staging.0)
                .env("PATH", self.child_path()?);
            run(
                &mut install,
                &staging.0,
                Duration::from_secs(300),
                &cancelled,
            )?;
            let mut check = Command::new(staging.0.join("node_modules/.bin/codex"));
            check.arg("--version").env("PATH", self.child_path()?);
            let version = run(&mut check, &staging.0, Duration::from_secs(15), &cancelled)?;
            if !version.split_whitespace().any(|part| part == CODEX_VERSION) {
                bail!("Installed Codex version did not match the manifest");
            }
            fs::write(staging.0.join("relay-lock.sha256"), lock_hash())?;
            if cancelled() {
                bail!("Preparation cancelled");
            }
            activate(&staging.0, &self.agent_dir())?;
        }
        let (codex, version) = match source {
            AgentSource::Managed => (
                self.agent_dir().join("node_modules/.bin/codex"),
                format!("Codex {CODEX_VERSION}"),
            ),
            AgentSource::Local(path) => {
                if !path.is_absolute() || !path.is_file() {
                    bail!("Choose an existing absolute path to the Codex executable.");
                }
                let mut check = Command::new(path);
                check.arg("--version").env("PATH", self.child_path()?);
                let version = run(&mut check, &self.root, Duration::from_secs(10), &cancelled)?;
                if !version.to_ascii_lowercase().contains("codex") {
                    bail!("The selected executable did not identify itself as Codex.");
                }
                (path.clone(), version.trim().to_owned())
            }
        };
        let mut env = BTreeMap::new();
        env.insert(
            "PATH".into(),
            self.child_path()?.to_string_lossy().into_owned(),
        );
        env.insert("CODEX_PATH".into(), codex.to_string_lossy().into_owned());
        env.insert("INITIAL_AGENT_MODE".into(), "agent".into());
        Ok(PreparedAgent {
            launch: LaunchSpec {
                command: self.node_dir().join("bin/node"),
                args: vec![
                    self.agent_dir()
                        .join("node_modules/@agentclientprotocol/codex-acp/dist/index.js")
                        .to_string_lossy()
                        .into_owned(),
                ],
                env,
            },
            version: format!("{version} · ACP {ADAPTER_VERSION}"),
        })
    }

    fn child_path(&self) -> Result<std::ffi::OsString> {
        let mut paths = vec![self.node_dir().join("bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        Ok(std::env::join_paths(paths)?)
    }
}

fn node_archive() -> Result<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok((
            "darwin-arm64",
            "bed7eea5325e1108f32ce5228ddd6a5f0f08a499ee42aa7442aea583702f6057",
        )),
        ("macos", "x86_64") => Ok((
            "darwin-x64",
            "1462cb3b3046b815cf8ea436d3da450ec1a9f11dac7e5a46b0ada5305d7e8097",
        )),
        _ => bail!("Managed Codex currently supports macOS on Apple Silicon and Intel."),
    }
}

fn lock_hash() -> String {
    digest_hex(Sha256::digest(LOCK.as_bytes()).as_ref())
}

fn digest_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").expect("write digest to string");
    }
    text
}

pub(crate) fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if digest_hex(hash.finalize().as_ref()) != expected {
        bail!("Runtime download failed checksum verification. Try preparing the agent again.");
    }
    Ok(())
}

pub(crate) fn activate(staging: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(
        destination
            .parent()
            .context("Component directory has no parent")?,
    )?;
    let backup = destination.with_extension(format!("previous-{}", unique_id()));
    let existed = destination.exists();
    if existed {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(staging, destination) {
        if existed {
            fs::rename(&backup, destination).context("Restoring previous component")?;
        }
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn run(
    command: &mut Command,
    log_directory: &Path,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<String> {
    let log = log_directory.join(format!("prepare-{}.log", unique_id()));
    let output = fs::File::create(&log)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .spawn()
        .context("Starting component preparation")?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancelled() || started.elapsed() > timeout {
            #[cfg(unix)]
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&log);
            bail!("Component preparation was cancelled or timed out. You can retry safely.");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut file = fs::File::open(&log)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(4096)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let output = String::from_utf8_lossy(&bytes).into_owned();
    let _ = fs::remove_file(log);
    if !status.success() {
        bail!("Component preparation failed: {}", output.trim());
    }
    Ok(output)
}

pub(crate) fn unique_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{time}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
pub(crate) struct Staging(pub(crate) PathBuf);
impl Staging {
    pub(crate) fn new(root: &Path) -> Result<Self> {
        let path = root.join(format!(".prepare-{}", unique_id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn discover_local() -> Vec<PathBuf> {
    let mut directories: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    directories.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        directories.extend([
            home.join(".local/bin"),
            home.join(".npm-global/bin"),
            home.join(".volta/bin"),
            home.join(".asdf/shims"),
        ]);
        if let Ok(entries) = fs::read_dir(home.join(".nvm/versions/node")) {
            directories.extend(
                entries
                    .filter_map(Result::ok)
                    .take(32)
                    .map(|entry| entry.path().join("bin")),
            );
        }
    }
    let mut found = BTreeMap::new();
    for directory in directories {
        let path = directory.join("codex");
        if path.is_file()
            && let Ok(real) = path.canonicalize()
        {
            found.entry(real).or_insert(path);
        }
    }
    found.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_identity_matches_the_embedded_package_lock() {
        let package: serde_json::Value = serde_json::from_str(PACKAGE).unwrap();
        let lock: serde_json::Value = serde_json::from_str(LOCK).unwrap();
        assert_eq!(
            package["dependencies"]["@agentclientprotocol/codex-acp"],
            ADAPTER_VERSION
        );
        assert_eq!(package["dependencies"]["@openai/codex"], CODEX_VERSION);
        assert_eq!(
            lock["packages"]["node_modules/@openai/codex"]["version"],
            CODEX_VERSION
        );
        assert_eq!(
            RELEASE,
            format!("codex-acp-{ADAPTER_VERSION}-codex-{CODEX_VERSION}")
        );
    }
    #[test]
    fn checksum_rejects_corruption_and_activation_preserves_previous_version() {
        let temp = Staging::new(&std::env::temp_dir()).unwrap();
        let checksum_file = temp.0.join("checksum-input");
        fs::write(&checksum_file, "abc").unwrap();
        verify_sha256(
            &checksum_file,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        let dest = temp.0.join("active");
        let new = temp.0.join("staged");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("version"), "old").unwrap();
        fs::create_dir(&new).unwrap();
        fs::write(new.join("version"), "new").unwrap();
        assert!(verify_sha256(&new.join("version"), "bad").is_err());
        activate(&new, &dest).unwrap();
        assert_eq!(fs::read_to_string(dest.join("version")).unwrap(), "new");
        let previous = fs::read_dir(&temp.0)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().contains("previous-"))
            .unwrap();
        assert_eq!(
            fs::read_to_string(previous.path().join("version")).unwrap(),
            "old"
        );
        assert!(activate(&temp.0.join("missing"), &dest).is_err());
        assert_eq!(fs::read_to_string(dest.join("version")).unwrap(), "new");
    }
}
