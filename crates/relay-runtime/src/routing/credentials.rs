use relay_core::routing::{RoutingError, RoutingProvider};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
        key: saved.key,
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
