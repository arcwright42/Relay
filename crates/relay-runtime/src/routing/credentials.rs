use relay_core::routing::{RoutingCredentialSource, RoutingError, RoutingProvider};
use serde::{Deserialize, Serialize};
use std::{env::VarError, path::Path};

pub(super) struct ConfiguredCredential {
    pub credential: Credential,
    pub source: RoutingCredentialSource,
}

pub(super) fn openrouter_configuration() -> Result<Option<ConfiguredCredential>, RoutingError> {
    let key = std::env::var("OPENROUTER_API_KEY");
    let directory = std::env::current_dir().map_err(|_| RoutingError::ConfigurationFile)?;
    from_environment(key, &directory.join(".env"))
}

// Read only: do not mutate process environment or pass .env secrets to agent subprocesses.
pub(super) fn from_environment(
    key: Result<String, VarError>,
    env_file: &Path,
) -> Result<Option<ConfiguredCredential>, RoutingError> {
    match key {
        Ok(key) if !key.trim().is_empty() => {
            return configured_key(key, RoutingCredentialSource::Environment);
        }
        Err(VarError::NotUnicode(_)) => return Err(RoutingError::InvalidKey),
        _ => {}
    }
    let entries = match dotenvy::from_path_iter(env_file) {
        Ok(entries) => entries,
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(_) => return Err(RoutingError::ConfigurationFile),
    };
    let mut key = None;
    for entry in entries {
        let (name, value) = entry.map_err(|_| RoutingError::ConfigurationFile)?;
        if name == "OPENROUTER_API_KEY" && key.is_none() {
            key = Some(value);
        }
    }
    match key {
        Some(key) => configured_key(key, RoutingCredentialSource::EnvFile),
        None => Ok(None),
    }
}

fn configured_key(
    key: String,
    source: RoutingCredentialSource,
) -> Result<Option<ConfiguredCredential>, RoutingError> {
    if key.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(ConfiguredCredential {
        credential: Credential {
            provider: RoutingProvider::OpenRouter,
            key: validate_key(&key)?.into(),
        },
        source,
    }))
}

pub(super) fn validate_key(key: &str) -> Result<&str, RoutingError> {
    let key = key.trim();
    if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(RoutingError::InvalidKey);
    }
    Ok(key)
}

pub(super) trait Credentials: Send + Sync {
    fn read(&self) -> Result<Option<Credential>, RoutingError>;
    fn write(&self, credential: Option<&Credential>) -> Result<(), RoutingError>;
}

#[derive(Clone)]
pub(super) struct Credential {
    pub provider: RoutingProvider,
    pub key: String,
}

#[derive(Serialize, Deserialize)]
struct SavedCredential {
    version: u32,
    provider: String,
    key: String,
}

pub(super) fn decode(bytes: &[u8]) -> Result<Credential, RoutingError> {
    let saved: SavedCredential =
        serde_json::from_slice(bytes).map_err(|_| RoutingError::Keychain)?;
    if saved.version != 1 {
        return Err(RoutingError::Keychain);
    }
    Ok(Credential {
        provider: RoutingProvider::from_code(&saved.provider).ok_or(RoutingError::Keychain)?,
        key: validate_key(&saved.key)
            .map_err(|_| RoutingError::Keychain)?
            .into(),
    })
}

pub(super) fn encode(credential: &Credential) -> Result<Vec<u8>, RoutingError> {
    serde_json::to_vec(&SavedCredential {
        version: 1,
        provider: credential.provider.code().into(),
        key: credential.key.clone(),
    })
    .map_err(|_| RoutingError::Keychain)
}

pub(super) struct Keychain {
    account: String,
}

impl Keychain {
    pub fn new(root: &Path) -> Self {
        use sha2::{Digest, Sha256};
        Self {
            account: Sha256::digest(root.as_os_str().as_encoded_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        }
    }
}

#[cfg(target_os = "macos")]
impl Credentials for Keychain {
    fn read(&self) -> Result<Option<Credential>, RoutingError> {
        match security_framework::passwords::get_generic_password(
            "com.arcwright42.relay.jev",
            &self.account,
        ) {
            Ok(bytes) => decode(&bytes).map(Some),
            Err(error) if error.code() == -25300 => Ok(None), // errSecItemNotFound
            Err(_) => Err(RoutingError::Keychain),
        }
    }
    fn write(&self, credential: Option<&Credential>) -> Result<(), RoutingError> {
        use security_framework::passwords::{delete_generic_password, set_generic_password};
        let result = match credential {
            Some(credential) => set_generic_password(
                "com.arcwright42.relay.jev",
                &self.account,
                &encode(credential)?,
            ),
            None => delete_generic_password("com.arcwright42.relay.jev", &self.account),
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if credential.is_none() && error.code() == -25300 => Ok(()),
            Err(_) => Err(RoutingError::Keychain),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl Credentials for Keychain {
    fn read(&self) -> Result<Option<Credential>, RoutingError> {
        let _ = &self.account;
        Err(RoutingError::Keychain)
    }
    fn write(&self, _: Option<&Credential>) -> Result<(), RoutingError> {
        Err(RoutingError::Keychain)
    }
}
