//! Pinned native build inputs and model resources for the macOS application bundle.
use super::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

const MANIFEST: &str = include_str!("../../crates/relay-runtime/resources/voice/manifest.json");
const KEYWORDS: &str = include_str!("../../crates/relay-runtime/resources/voice/keywords.txt");

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).expect("checked-in voice manifest")
}

fn notice_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/relay-runtime/resources/voice/licenses")
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or_else(|| format!("Missing voice resource field {name}").into())
}

fn hash(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest_hex(digest.finalize().as_ref()))
}

fn digest_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").expect("format resource checksum");
    }
    text
}

pub(super) fn check() -> Result<()> {
    let spec = manifest();
    if spec["version"] != 1 || spec["engine"]["version"] != "1.13.8" {
        return Err("Review the voice resource version before upgrading the engine".into());
    }
    let digest = digest_hex(Sha256::digest(KEYWORDS.as_bytes()).as_ref());
    if digest != field(&spec["keywords"], "sha256")? {
        return Err("Voice keyword configuration must match its pinned SHA-256".into());
    }
    let vad = &spec["vad"];
    let name = field(vad, "name")?;
    let digest = field(vad, "sha256")?;
    if name != "silero_vad.onnx"
        || !field(vad, "url")?
            .starts_with("https://github.com/k2-fsa/sherpa-onnx/releases/download/")
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || vad["bytes"].as_u64().is_none_or(|size| size == 0)
    {
        return Err("Voice activity detection requires a pinned model and SHA-256".into());
    }
    for item in spec["model"]["files"]
        .as_array()
        .ok_or("Missing voice model files")?
    {
        for key in ["name", "source"] {
            let name = field(item, key)?;
            if Path::new(name).components().count() != 1 || name.starts_with('.') {
                return Err("Voice model paths must be plain filenames".into());
            }
        }
        let digest = field(item, "sha256")?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Voice model files require fixed SHA-256 values".into());
        }
    }
    for item in spec["notices"]
        .as_array()
        .ok_or("Missing voice dependency notices")?
    {
        let name = field(item, "name")?;
        if Path::new(name).components().count() != 1
            || name.starts_with('.')
            || hash(&notice_directory().join(name))? != field(item, "sha256")?
        {
            return Err(
                "Voice dependency notices must match their pinned sources and SHA-256".into(),
            );
        }
    }
    Ok(())
}

pub(super) fn check_sdk(metadata: &Value) -> Result<()> {
    let packages = metadata["packages"].as_array().ok_or("Missing packages")?;
    let platform = packages
        .iter()
        .find(|package| package["name"] == "relay-platform")
        .ok_or("Missing platform package")?;
    let dependencies = platform["dependencies"]
        .as_array()
        .ok_or("Missing platform dependencies")?;
    let sdk = dependencies
        .iter()
        .find(|dependency| dependency["name"] == "sherpa-onnx")
        .ok_or("Missing voice wake SDK")?;
    let version = manifest()["engine"]["version"]
        .as_str()
        .ok_or("Missing wake engine version")?
        .to_owned();
    if sdk["req"] != format!("={version}")
        || sdk["uses_default_features"] != false
        || sdk["features"] != serde_json::json!(["static"])
        || sdk["target"] != "cfg(target_os = \"macos\")"
    {
        return Err("Voice wake SDK must match the pinned macOS static engine manifest".into());
    }
    Ok(())
}

fn archive(target: &Path, name: &str, url: &str, expected: &str) -> Result<PathBuf> {
    let cache = target.join("relay-deps/voice/archives");
    fs::create_dir_all(&cache)?;
    let path = cache.join(name);
    if path.is_file() && hash(&path)? == expected {
        return Ok(path);
    }
    let pending = cache.join(format!(".{name}-{}.pending", std::process::id()));
    let result = (|| -> Result<()> {
        if !Command::new("/usr/bin/curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--connect-timeout",
                "20",
                "--max-time",
                "180",
                "--retry",
                "2",
                "--output",
            ])
            .arg(&pending)
            .arg(url)
            .status()?
            .success()
        {
            return Err(format!("Could not download pinned voice resource {name}").into());
        }
        if hash(&pending)? != expected {
            return Err(format!("SHA-256 mismatch for voice resource {name}").into());
        }
        fs::rename(&pending, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result?;
    Ok(path)
}

fn unpack(archive: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    if !Command::new("/usr/bin/tar")
        .arg("-xjf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()?
        .success()
    {
        return Err("Could not extract verified voice resources".into());
    }
    Ok(())
}

pub(super) fn prepare_engine(target: &Path) -> Result<PathBuf> {
    check()?;
    let spec = manifest();
    let item = &spec["engine"]["archives"][std::env::consts::ARCH];
    let name = field(item, "name")?;
    let checksum = field(item, "sha256")?;
    let url = format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/v{}/{name}",
        field(&spec["engine"], "version")?
    );
    let archive = archive(target, name, &url, checksum)?;
    let directory = target.join("relay-deps/voice/native");
    // Always restore the native libraries from the verified archive. Local cache
    // modifications must not become unreviewed linker inputs on the governed path.
    unpack(&archive, &directory)?;
    let library = directory
        .join(name.trim_end_matches(".tar.bz2"))
        .join("lib");
    if !library.join("libsherpa-onnx-c-api.a").is_file()
        || !library.join("libonnxruntime.a").is_file()
    {
        return Err("Pinned voice engine archive is missing its static libraries".into());
    }
    Ok(library)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let pending = path.with_extension(format!("{}.pending", std::process::id()));
    fs::write(&pending, bytes)?;
    fs::rename(pending, path)?;
    Ok(())
}

pub(super) fn prepare_model(target: &Path) -> Result<PathBuf> {
    check()?;
    let spec = manifest();
    let model = &spec["model"];
    let output = target.join("relay-resources/voice");
    let files = model["files"]
        .as_array()
        .ok_or("Missing voice model files")?;
    let complete = files.iter().all(|item| {
        let Some(name) = item["name"].as_str() else {
            return false;
        };
        let Some(expected) = item["sha256"].as_str() else {
            return false;
        };
        hash(&output.join(name)).is_ok_and(|digest| digest == expected)
    });
    if !complete {
        let name = format!("{}.tar.bz2", field(model, "name")?);
        let archive = archive(target, &name, field(model, "url")?, field(model, "sha256")?)?;
        let unpacked = target.join("relay-deps/voice/model");
        unpack(&archive, &unpacked)?;
        let source = unpacked.join(field(model, "name")?);
        fs::create_dir_all(&output)?;
        for item in files {
            let input = source.join(field(item, "source")?);
            if hash(&input)? != field(item, "sha256")? {
                return Err("Extracted voice model failed integrity verification".into());
            }
            write_atomic(&output.join(field(item, "name")?), &fs::read(input)?)?;
        }
    }
    fs::create_dir_all(&output)?;
    let vad = &spec["vad"];
    let vad_file = archive(
        target,
        field(vad, "name")?,
        field(vad, "url")?,
        field(vad, "sha256")?,
    )?;
    write_atomic(&output.join(field(vad, "name")?), &fs::read(vad_file)?)?;
    write_atomic(&output.join("keywords.txt"), KEYWORDS.as_bytes())?;
    write_atomic(&output.join("manifest.json"), MANIFEST.as_bytes())?;
    let notices = output.join("licenses");
    fs::create_dir_all(&notices)?;
    for item in spec["notices"]
        .as_array()
        .ok_or("Missing voice dependency notices")?
    {
        let name = field(item, "name")?;
        write_atomic(
            &notices.join(name),
            &fs::read(notice_directory().join(name))?,
        )?;
    }
    Ok(output)
}

pub(super) fn bundle_resources(source: &Path, native: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for item in fs::read_dir(source)? {
        let item = item?;
        if item.file_type()?.is_file() {
            write_atomic(&destination.join(item.file_name()), &fs::read(item.path())?)?;
        } else if item.file_type()?.is_dir() {
            copy_directory(&item.path(), &destination.join(item.file_name()))?;
        }
    }
    let credits = native
        .parent()
        .ok_or("Missing native voice resource directory")?;
    for name in ["LICENSE", "LICENSE.txt", "LICENSES", "licenses"] {
        let source = credits.join(name);
        if source.is_file() {
            write_atomic(&destination.join(name), &fs::read(&source)?)?;
        }
        if source.is_dir() {
            copy_directory(&source, &destination.join(name))?;
        }
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for item in fs::read_dir(source)? {
        let item = item?;
        if item.file_type()?.is_dir() {
            copy_directory(&item.path(), &destination.join(item.file_name()))?;
        } else if item.file_type()?.is_file() {
            write_atomic(&destination.join(item.file_name()), &fs::read(item.path())?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_engine_sdk_version_and_link_mode_drift() {
        let sdk = serde_json::json!({
            "name": "sherpa-onnx", "req": "=1.13.8", "uses_default_features": false,
            "features": ["static"], "target": "cfg(target_os = \"macos\")"
        });
        let mut metadata =
            serde_json::json!({"packages": [{"name": "relay-platform", "dependencies": [sdk]}]});
        assert!(check_sdk(&metadata).is_ok());
        metadata["packages"][0]["dependencies"][0]["req"] = "=1.13.9".into();
        assert!(check_sdk(&metadata).is_err());
        metadata["packages"][0]["dependencies"][0]["req"] = "=1.13.8".into();
        metadata["packages"][0]["dependencies"][0]["features"] = serde_json::json!(["shared"]);
        assert!(check_sdk(&metadata).is_err());
    }
}
