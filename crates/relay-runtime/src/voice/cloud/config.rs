//! Voice credentials are independent of Jev, and never enter domain snapshots.
use std::{collections::BTreeMap, path::Path};

pub(super) struct Config {
    pub key: String,
    pub endpoint: String,
    pub workspace: Option<String>,
}

const SERVICE: &str = "com.arcwright42.relay.voice.dashscope";
const VARIABLES: [&str; 3] = [
    "DASHSCOPE_API_KEY",
    "RELAY_VOICE_REGION",
    "DASHSCOPE_WORKSPACE_ID",
];

pub(super) fn load(root: &Path) -> Result<Option<Config>, String> {
    let mut values = BTreeMap::new();
    let path = std::env::current_dir()
        .map_err(|_| "Could not locate voice configuration.")?
        .join(".env");
    match dotenvy::from_path_iter(path) {
        Ok(entries) => {
            for entry in entries {
                let (name, value) = entry.map_err(|_| "Could not parse voice configuration.")?;
                if VARIABLES.contains(&name.as_str()) {
                    values.entry(name).or_insert(value);
                }
            }
        }
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Could not read voice configuration.".into()),
    }
    for name in VARIABLES {
        match std::env::var(name) {
            Ok(value) if !value.trim().is_empty() => {
                values.insert(name.into(), value);
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err("Voice configuration must be valid text.".into());
            }
            _ => {}
        }
    }
    let key = match values
        .remove("DASHSCOPE_API_KEY")
        .filter(|v| !v.trim().is_empty())
    {
        Some(key) => Some(key),
        None => read_key(root)?,
    };
    let Some(key) = key else { return Ok(None) };
    let key = validate_key(&key)?.to_owned();
    let workspace = values
        .remove("DASHSCOPE_WORKSPACE_ID")
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_owned());
    let endpoint = endpoint(
        values
            .get("RELAY_VOICE_REGION")
            .map(String::as_str)
            .unwrap_or(""),
        workspace.as_deref(),
    )?;
    Ok(Some(Config {
        key,
        endpoint,
        workspace,
    }))
}

fn endpoint(region: &str, workspace: Option<&str>) -> Result<String, String> {
    if region.trim().is_empty() && workspace.is_none() {
        return Ok("wss://maas.qianwenaiapi.com/api-ws/v1/inference".into());
    }
    let (region, legacy) = match region.trim() {
        "" | "cn-beijing" => ("cn-beijing", "dashscope.aliyuncs.com"),
        "ap-southeast-1" => ("ap-southeast-1", "dashscope-intl.aliyuncs.com"),
        _ => return Err("RELAY_VOICE_REGION must be cn-beijing or ap-southeast-1.".into()),
    };
    let host = match workspace {
        Some(id)
            if !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') =>
        {
            format!("{id}.{region}.maas.aliyuncs.com")
        }
        Some(_) => return Err("Invalid DASHSCOPE_WORKSPACE_ID.".into()),
        None => legacy.into(),
    };
    Ok(format!("wss://{host}/api-ws/v1/inference"))
}

fn validate_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("Invalid voice API key.".into());
    }
    Ok(key)
}

fn account(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(root.as_os_str().as_encoded_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(target_os = "macos")]
fn read_key(root: &Path) -> Result<Option<String>, String> {
    match security_framework::passwords::get_generic_password(SERVICE, &account(root)) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "Invalid voice Keychain entry.".into()),
        Err(error) if error.code() == -25300 => Ok(None),
        Err(_) => Err("Could not read the voice API key from macOS Keychain.".into()),
    }
}

/// Call on a background worker or the headless credential command, never with argv secrets.
#[cfg(target_os = "macos")]
pub fn save_voice_key(root: &Path, key: &str) -> Result<(), String> {
    let key = validate_key(key)?;
    security_framework::passwords::set_generic_password(SERVICE, &account(root), key.as_bytes())
        .map_err(|_| "Could not save the voice API key to macOS Keychain.".into())
}

#[cfg(not(target_os = "macos"))]
fn read_key(_: &Path) -> Result<Option<String>, String> {
    Ok(None)
}
#[cfg(not(target_os = "macos"))]
pub fn save_voice_key(_: &Path, _: &str) -> Result<(), String> {
    Err("Voice Keychain requires macOS.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_are_official_and_keys_cannot_inject_headers() {
        assert_eq!(
            endpoint("", None).unwrap(),
            "wss://maas.qianwenaiapi.com/api-ws/v1/inference"
        );
        assert_eq!(
            endpoint("cn-beijing", None).unwrap(),
            "wss://dashscope.aliyuncs.com/api-ws/v1/inference"
        );
        assert_eq!(
            endpoint("ap-southeast-1", Some("ws-123")).unwrap(),
            "wss://ws-123.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/inference"
        );
        assert!(endpoint("unknown", None).is_err());
        assert!(endpoint("cn-beijing", Some("host.attacker.test/")).is_err());
        assert!(validate_key("key\r\nHeader: value").is_err());
        assert_eq!(validate_key(" test-key ").unwrap(), "test-key");
    }
}
