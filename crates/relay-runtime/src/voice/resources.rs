use relay_core::voice::{VoiceError, VoiceErrorKind, WakeResources};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MANIFEST: &str = include_str!("../../resources/voice/manifest.json");

/// Bundled resources in production; a fixed target directory for direct Cargo runs.
/// The optional override is for explicit developer probes, with identical validation.
pub struct BundledWakeResources {
    directory: PathBuf,
}

impl BundledWakeResources {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn discover() -> Self {
        if let Some(directory) = std::env::var_os("RELAY_WAKE_MODEL_DIR") {
            return Self::new(directory.into());
        }
        let executable = std::env::current_exe().unwrap_or_default();
        let parent = executable.parent().unwrap_or_else(|| Path::new("."));
        let directory = if parent.file_name().is_some_and(|name| name == "MacOS") {
            parent.join("../Resources/voice")
        } else {
            parent.join("../relay-resources/voice")
        };
        Self::new(directory)
    }
}

impl WakeResources for BundledWakeResources {
    fn directory(&self) -> Result<PathBuf, VoiceError> {
        validate(&self.directory).map_err(|error| {
            VoiceError::new(
                VoiceErrorKind::ResourcesUnavailable,
                format!("Voice resources could not be verified: {error:#}"),
            )
        })?;
        Ok(self.directory.clone())
    }
}

fn validate(directory: &Path) -> anyhow::Result<()> {
    let spec: Value = serde_json::from_str(MANIFEST)?;
    if fs::read(directory.join("manifest.json"))? != MANIFEST.as_bytes() {
        anyhow::bail!("The installed voice resource manifest does not match this Relay build.");
    }
    for item in spec["model"]["files"]
        .as_array()
        .expect("voice model files")
        .iter()
        .chain(std::iter::once(&spec["vad"]))
    {
        let name = item["name"].as_str().expect("voice model filename");
        let path = directory.join(name);
        if fs::metadata(&path)?.len() != item["bytes"].as_u64().expect("voice model size") {
            anyhow::bail!("Unexpected size for {name}.");
        }
        crate::installer::verify_sha256(
            &path,
            item["sha256"].as_str().expect("voice model checksum"),
        )?;
    }
    crate::installer::verify_sha256(
        &directory.join("keywords.txt"),
        spec["keywords"]["sha256"]
            .as_str()
            .expect("voice keyword checksum"),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_resources_fail_before_native_model_loading() {
        let directory = std::env::temp_dir().join(format!(
            "relay-voice-resources-{}",
            crate::installer::unique_id()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("manifest.json"), "{\"version\":999}").unwrap();
        let error = BundledWakeResources::new(directory.clone())
            .directory()
            .unwrap_err();
        assert_eq!(error.kind, VoiceErrorKind::ResourcesUnavailable);
        assert!(error.detail.contains("manifest does not match"));
        fs::remove_dir_all(directory).unwrap();
    }
}
