use super::*;
use anyhow::{Context, bail, ensure};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    #[default]
    ClaudeMem,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ContextMode {
    Disabled,
    Session,
    #[default]
    Turn,
}
impl ContextMode {
    pub fn applies(self, fresh: bool) -> bool {
        self == Self::Turn || (self == Self::Session && fresh)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    pub version: u32,
    pub provider: ProviderKind,
    #[serde(default)]
    pub context: ContextMode,
    #[serde(default)]
    pub claude_mem_url: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Explicit opt-in for importing existing Relay/client archives into another engine.
    #[serde(default)]
    pub import_history: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            version: 2,
            provider: ProviderKind::ClaudeMem,
            context: ContextMode::Turn,
            claude_mem_url: None,
            namespace: None,
            import_history: false,
        }
    }
}

impl MemoryConfig {
    pub fn load(root: &Path) -> Result<Self> {
        let mut config = Self::load_saved(root)?;
        config.resolve_url()?;
        config.validate()?;
        Ok(config)
    }

    fn load_saved(root: &Path) -> Result<Self> {
        let path = root.join("memory-provider.json");
        let config: Self = match fs::read(&path) {
            Ok(bytes) => {
                ensure!(bytes.len() <= 16_384, "Memory provider config is too large");
                let mut value: Value = serde_json::from_slice(&bytes)
                    .context("Invalid memory-provider.json; existing configuration preserved")?;
                ensure!(
                    matches!(value["version"].as_u64(), Some(1 | 2)),
                    "Unsupported memory provider configuration version; file preserved"
                );
                // v1 Native installations now select Claude-Mem. No memory content is migrated.
                // Keep the original file until an explicit config command saves the new format.
                if value["version"] == 1 {
                    if value["provider"] == "native" {
                        value["provider"] = json!("claude-mem");
                        value["import_history"] = json!(false);
                    }
                    value["version"] = json!(2);
                }
                serde_json::from_value(value)
                    .context("Invalid memory-provider.json; file preserved")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        Self::validate(&config)?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 2,
            "Unsupported memory provider configuration version"
        );
        if let Some(namespace) = &self.namespace {
            ensure!(
                !namespace.is_empty()
                    && namespace.len() <= 80
                    && namespace
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "Memory namespace must contain 1–80 ASCII letters, digits, - or _"
            );
        }
        if let Some(url) = self.claude_mem_url.as_deref() {
            // This adapter targets the local Worker API, not the hosted/sync API.
            let uri: ureq::http::Uri = url.parse().context("Invalid Claude-Mem worker URL")?;
            ensure!(
                uri.scheme_str() == Some("http")
                    && matches!(uri.host(), Some("127.0.0.1" | "[::1]"))
                    && uri.port_u16().is_some_and(|port| port > 0),
                "Use a literal loopback Worker URL with explicit port, e.g. http://127.0.0.1:37777"
            );
            ensure!(
                uri.path() == "/" && uri.query().is_none() && !url.contains('@'),
                "Worker URL must not contain credentials, a path or query"
            );
        }
        Ok(())
    }
    pub(super) fn resolve_url(&mut self) -> Result<()> {
        if self.claude_mem_url.is_some() {
            return Ok(());
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut data = std::env::var_os("CLAUDE_MEM_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|p| p.join(".claude-mem")));
        let read_settings = |data: &Option<PathBuf>| -> Result<Value> {
            let Some(data) = data else {
                return Ok(json!({}));
            };
            match fs::read(data.join("settings.json")) {
                Ok(bytes) => {
                    ensure!(
                        bytes.len() <= 1024 * 1024,
                        "Claude-Mem settings are too large"
                    );
                    serde_json::from_slice(&bytes).context("Invalid Claude-Mem settings; configure --memory-provider-set claude-mem <URL> explicitly")
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
                Err(e) => Err(e.into()),
            }
        };
        let expand = |path: PathBuf| -> PathBuf {
            if let Ok(tail) = path.strip_prefix("~") {
                home.as_ref().map(|p| p.join(tail)).unwrap_or(path)
            } else {
                path
            }
        };
        data = data.map(&expand);
        let mut settings = read_settings(&data)?;
        if std::env::var_os("CLAUDE_MEM_DATA_DIR").is_none()
            && let Some(path) = settings["CLAUDE_MEM_DATA_DIR"]
                .as_str()
                .filter(|p| !p.is_empty())
        {
            data = Some(expand(PathBuf::from(path)));
            settings = read_settings(&data)?;
        }
        let configured = std::env::var("CLAUDE_MEM_WORKER_PORT")
            .ok()
            .or_else(|| {
                settings["CLAUDE_MEM_WORKER_PORT"]
                    .as_str()
                    .map(str::to_owned)
            })
            .or_else(|| {
                settings["CLAUDE_MEM_WORKER_PORT"]
                    .as_u64()
                    .map(|p| p.to_string())
            });
        let port = if let Some(port) = configured {
            port.parse::<u16>()
                .context("Invalid Claude-Mem worker port")?
        } else {
            #[cfg(unix)]
            let uid = {
                let output = std::process::Command::new("/usr/bin/id").arg("-u").output()
                    .context("Cannot determine Claude-Mem's default port; configure a Worker URL explicitly")?;
                ensure!(
                    output.status.success(),
                    "Cannot determine the current user ID"
                );
                String::from_utf8(output.stdout)?.trim().parse::<u32>()?
            };
            #[cfg(not(unix))]
            let uid = 77;
            37700 + (uid % 100) as u16
        };
        ensure!(port > 0, "Invalid Claude-Mem worker port");
        self.claude_mem_url = Some(format!("http://127.0.0.1:{port}"));
        Ok(())
    }

    pub fn identity(&self) -> String {
        let hash = Sha256::digest(serde_json::to_vec(self).expect("memory config"));
        format!(
            "memory-v2-{}",
            hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(root)?;
        let path = root.join(format!(".memory-provider-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&path)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            f.sync_all()?;
            fs::rename(&path, root.join("memory-provider.json"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }
}

pub fn run_cli(root: &Path, args: &[String]) -> Result<bool> {
    let Some(cmd) = args.first() else {
        return Ok(false);
    };
    if !matches!(
        cmd.as_str(),
        "--memory-provider"
            | "--memory-provider-set"
            | "--memory-context"
            | "--memory-provider-check"
            | "--memory-provider-drain"
            | "--memory-provider-import-history"
            | "--memory-provider-resolve"
            | "--memory-status"
            | "--memory-query"
            | "--memory-provider-retry"
    ) {
        ensure!(
            !cmd.starts_with("--memory-") && !cmd.starts_with("--embedding-"),
            "Unknown or retired memory command; use --memory-provider or --memory-status. Model and embedding settings belong to Claude-Mem."
        );
        return Ok(false);
    }
    let mut config = MemoryConfig::load_saved(root)?;
    match (cmd.as_str(), args.len()) {
        ("--memory-provider", 1) => {
            config.resolve_url()?;
            config.validate()?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ("--memory-provider-set", 2 | 3) => {
            ensure!(
                args[1] == "claude-mem",
                "Native memory has been removed; the supported provider is claude-mem"
            );
            ensure!(args.len() == 3, "claude-mem requires a local Worker URL");
            config.claude_mem_url = Some(args[2].trim_end_matches('/').to_owned());
            config.save(root)?;
            println!(
                "Provider saved. Restart Relay to apply. Existing memory databases are preserved; history import is opt-in."
            );
        }
        ("--memory-context", 2) => {
            config.context = match args[1].as_str() {
                "disabled" => ContextMode::Disabled,
                "session" => ContextMode::Session,
                "turn" => ContextMode::Turn,
                _ => bail!("Context mode must be disabled, session or turn"),
            };
            config.save(root)?;
            println!("Memory context policy saved. Restart Relay to apply.");
        }
        ("--memory-provider-import-history", 2) => {
            config.import_history = match args[1].as_str() {
                "on" => true,
                "off" => false,
                _ => bail!("Expected on or off"),
            };
            config.save(root)?;
            println!("History import preference saved. Restart Relay to apply.");
        }
        ("--memory-provider-check", 1)
        | ("--memory-status", 1)
        | ("--memory-query", 2)
        | ("--memory-provider-retry", 1) => {
            let provider = super::open(root)?;
            if cmd == "--memory-provider-retry" {
                provider
                    .apply(MemoryCommand::RetryFailed)
                    .map_err(anyhow::Error::msg)?;
                println!(
                    "Failed delivery jobs queued. Uncertain deliveries require explicit resolution; they are not blindly replayed."
                );
            } else {
                let (name, a) = if cmd == "--memory-query" {
                    ("memory_search", json!({"query":args[1]}))
                } else {
                    ("memory_status", json!({}))
                };
                println!("{}", provider.call(crate::resident::MAIN, name, &a)?);
            }
        }
        ("--memory-provider-drain", 1) => {
            ensure!(
                config.provider == ProviderKind::ClaudeMem,
                "Drain is for the Claude-Mem delivery queue"
            );
            let provider = super::claude_mem::ClaudeMemProvider::open(root, config)?;
            println!("{}", provider.drain()?);
        }
        ("--memory-provider-resolve", 3) => {
            ensure!(
                config.provider == ProviderKind::ClaudeMem,
                "Resolution is for the Claude-Mem delivery queue"
            );
            let provider = super::claude_mem::ClaudeMemProvider::open(root, config)?;
            provider.resolve(args[1].parse().context("Expected a delivery ID")?, &args[2])?;
            println!("Delivery resolution saved.");
        }
        _ => bail!(
            "Usage: --memory-provider, --memory-provider-set claude-mem <loopback URL>, --memory-context disabled|session|turn, --memory-provider-import-history on|off, --memory-provider-check, --memory-provider-drain, --memory-provider-retry, --memory-provider-resolve <id> accepted|retry|discard, --memory-status, --memory-query <text>"
        ),
    }
    Ok(true)
}

pub(super) fn private_path(root: &Path, identity: &str) -> Result<PathBuf> {
    let path = root.join("memory-providers").join(identity);
    fs::create_dir_all(&path)?;
    Ok(path)
}
