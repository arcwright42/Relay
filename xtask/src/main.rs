use serde_json::Value;
use std::{error::Error, fs, path::Path, process::Command};
mod voice;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    std::env::set_current_dir(root)?;
    match std::env::args().nth(1).as_deref() {
        Some("check-packages") => check_packages(),
        Some("verify") => {
            check_packages()?;
            run("cargo", &["fmt", "--all", "--", "--check"])?;
            let target = target_directory()?;
            let native = voice::prepare_engine(&target)?;
            voice::prepare_model(&target)?;
            run_with_voice(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--locked",
                    "--",
                    "-D",
                    "warnings",
                ],
                &native,
            )?;
            run_with_voice(
                "cargo",
                &["test", "--workspace", "--all-features", "--locked"],
                &native,
            )
        }
        Some("icon") => build_icon(root),
        Some("prepare-voice") => {
            let target = target_directory()?;
            let native = voice::prepare_engine(&target)?;
            let model = voice::prepare_model(&target)?;
            println!(
                "Voice native libraries: {}\nVoice model resources: {}",
                native.display(),
                model.display()
            );
            Ok(())
        }
        Some("bundle") => bundle(root, std::env::args().any(|arg| arg == "--release")),
        Some("start") => start(root),
        _ => {
            println!(
                "cargo xtask <check-packages | verify | prepare-voice | icon | bundle [--release] | start>"
            );
            Ok(())
        }
    }
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    if !Command::new(program).args(args).status()?.success() {
        return Err(format!("{program} {} failed", args.join(" ")).into());
    }
    Ok(())
}

fn metadata() -> Result<Value> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn target_directory() -> Result<std::path::PathBuf> {
    Ok(metadata()?["target_directory"]
        .as_str()
        .ok_or("Missing target directory")?
        .into())
}

fn run_with_voice(program: &str, args: &[&str], native: &Path) -> Result<()> {
    let resources = target_directory()?.join("relay-resources/voice");
    if !Command::new(program)
        .args(args)
        .env("SHERPA_ONNX_LIB_DIR", native)
        .env("RELAY_TEST_VOICE_RESOURCES", resources)
        .status()?
        .success()
    {
        return Err(format!("{program} {} failed", args.join(" ")).into());
    }
    Ok(())
}

fn validate_package(package: &Value) -> Result<()> {
    let name = package["name"].as_str().ok_or("Missing package name")?;
    let allowed: &[&str] = match name {
        "relay" => &[
            "gpui-kit",
            "relay-ui",
            "relay-core",
            "relay-runtime",
            "relay-platform",
        ],
        "relay-ui" => &["gpui-kit", "relay-core"],
        "relay-core" => &[],
        "relay-platform" => &[
            "relay-core",
            "async-channel",
            "block2",
            "objc2",
            "objc2-app-kit",
            "objc2-foundation",
            "objc2-av-foundation",
            "cpal",
            "sherpa-onnx",
        ],
        "relay-runtime" => &[
            "anyhow",
            "rusqlite",
            "dotenvy",
            "relay-core",
            "relay-acp",
            "serde",
            "serde_json",
            "sha2",
            "ureq",
            "security-framework",
            "futures-util",
            "tokio",
            "tokio-tungstenite",
            "uuid",
        ],
        "relay-acp" => &[
            "agent-client-protocol",
            "async-channel",
            "async-io",
            "futures-lite",
            "relay-core",
            "serde_json",
        ],
        "xtask" => &["serde_json", "sha2"],
        _ => {
            return Err(format!(
                "{name}: define its layer in docs/PACKAGES.md and xtask before adding a package"
            )
            .into());
        }
    };
    if package["publish"] != serde_json::json!([]) {
        return Err(format!("{name}: workspace packages must inherit publish = false").into());
    }
    if package["version"] != env!("CARGO_PKG_VERSION")
        || package["edition"] != "2024"
        || package["rust_version"] != "1.97"
    {
        return Err(
            format!("{name}: inherit the workspace version, edition, and Rust version").into(),
        );
    }
    for dependency in package["dependencies"]
        .as_array()
        .ok_or("Missing dependency list")?
    {
        let dep = dependency["name"]
            .as_str()
            .ok_or("Missing dependency name")?;
        if !allowed.contains(&dep) {
            return Err(format!("{name} → {dep}: dependency crosses a package boundary").into());
        }
        let source = dependency["source"].as_str().unwrap_or("");
        if source.starts_with("git+") || !dependency["req"].as_str().unwrap_or("").starts_with('=')
        {
            return Err(format!(
                "{name} → {dep}: use an exact workspace dependency, without a Git branch"
            )
            .into());
        }
    }
    Ok(())
}

fn check_packages() -> Result<()> {
    voice::check()?;
    let metadata = metadata()?;
    voice::check_sdk(&metadata)?;
    for package in metadata["packages"].as_array().ok_or("Missing packages")? {
        validate_package(package)?;
        println!(
            "{}: package boundary OK",
            package["name"].as_str().unwrap_or("unknown")
        );
    }
    validate_adapter_packages(
        &serde_json::from_str(include_str!(
            "../../crates/relay-runtime/resources/codex/package.json"
        ))?,
        &serde_json::from_str(include_str!(
            "../../crates/relay-runtime/resources/codex/package-lock.json"
        ))?,
    )?;
    println!("Codex ACP adapter: package versions and integrity lock OK");
    println!("Voice wake: SDK, native archives, model and keyword integrity lock OK");
    Ok(())
}

fn exact_version(value: &str) -> bool {
    let base = value.split_once('-').map_or(value, |(base, _)| base);
    let parts: Vec<_> = base.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
}

fn validate_adapter_packages(package: &Value, lock: &Value) -> Result<()> {
    if package["private"] != true || lock["lockfileVersion"] != 3 {
        return Err("Adapter components must be private and use npm lockfile v3".into());
    }
    let dependencies = package["dependencies"]
        .as_object()
        .ok_or("Missing adapter dependency")?;
    if dependencies.len() != 1 || !dependencies.contains_key("@agentclientprotocol/codex-acp") {
        return Err(
            "Only the ACP adapter may be a direct dependency; use a local Codex executable".into(),
        );
    }
    if lock["packages"][""]["dependencies"] != package["dependencies"]
        || !package["overrides"].is_null()
    {
        return Err("Adapter dependency manifest and lockfile disagree".into());
    }
    for (name, version) in dependencies {
        let version = version.as_str().ok_or("Invalid component version")?;
        if !exact_version(version)
            || lock["packages"][format!("node_modules/{name}")]["version"] != version
        {
            return Err(format!("{name}: pin the exact installed version").into());
        }
    }
    for (path, entry) in lock["packages"]
        .as_object()
        .ok_or("Missing locked packages")?
    {
        if path.is_empty() {
            continue;
        }
        if !path.starts_with("node_modules/")
            || !exact_version(entry["version"].as_str().unwrap_or(""))
            || !entry["resolved"]
                .as_str()
                .unwrap_or("")
                .starts_with("https://registry.npmjs.org/")
            || !entry["integrity"]
                .as_str()
                .unwrap_or("")
                .strip_prefix("sha512-")
                .is_some_and(|digest| digest.len() == 88)
        {
            return Err(format!(
                "{path}: require a pinned registry package with SHA-512 integrity"
            )
            .into());
        }
    }
    Ok(())
}

fn build_icon(root: &Path) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("Icon generation uses the macOS Quick Look and iconutil tools".into());
    }
    let output = root.join("target/relay-icon");
    let iconset = output.join("Relay.iconset");
    fs::create_dir_all(&iconset)?;
    run(
        "qlmanage",
        &[
            "-t",
            "-s",
            "1024",
            "-o",
            output.to_str().ok_or("Invalid output path")?,
            "assets/relay-icon.svg",
        ],
    )?;
    let png = output.join("relay-icon.svg.png");
    for (name, size) in [
        ("icon_16x16.png", "16"),
        ("icon_16x16@2x.png", "32"),
        ("icon_32x32.png", "32"),
        ("icon_32x32@2x.png", "64"),
        ("icon_128x128.png", "128"),
        ("icon_128x128@2x.png", "256"),
        ("icon_256x256.png", "256"),
        ("icon_256x256@2x.png", "512"),
        ("icon_512x512.png", "512"),
        ("icon_512x512@2x.png", "1024"),
    ] {
        run(
            "sips",
            &[
                "-z",
                size,
                size,
                png.to_str().ok_or("Invalid PNG path")?,
                "--out",
                iconset.join(name).to_str().ok_or("Invalid icon path")?,
            ],
        )?;
    }
    run(
        "iconutil",
        &[
            "-c",
            "icns",
            iconset.to_str().ok_or("Invalid iconset path")?,
            "-o",
            "assets/Relay.icns",
        ],
    )?;
    fs::copy(png, root.join("assets/relay-icon.png"))?;
    println!("Generated assets/Relay.icns and assets/relay-icon.png");
    Ok(())
}

fn select_signing_identity(
    configured: Option<&str>,
    saved: Option<&str>,
    existing: Option<&str>,
) -> Result<String> {
    let identity = configured.or(saved).or(existing).unwrap_or("-").trim();
    if identity.is_empty() || identity.contains(['\n', '\r']) {
        return Err(
            "Signing identity must be a nonempty, single-line name or certificate SHA-1".into(),
        );
    }
    Ok(identity.to_owned())
}

fn bundle_signing_identity(root: &Path, app: &Path) -> Result<String> {
    let configured = match std::env::var("RELAY_SIGNING_IDENTITY") {
        Ok(identity) => Some(identity),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let saved = if configured.is_none() {
        match fs::read_to_string(root.join(".relay-signing-identity")) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        }
    } else {
        None
    };
    let existing = if configured.is_none() && saved.is_none() && app.exists() {
        // Read the old signature before replacing its executable. Rebuilding an
        // already signed app must not silently change its macOS privacy identity.
        let output = Command::new("codesign")
            .args(["--display", "--verbose=2"])
            .arg(app)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Could not inspect the existing Relay signature: {}. Set RELAY_SIGNING_IDENTITY explicitly to choose a new identity",
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let details = String::from_utf8_lossy(&output.stderr);
        if let Some(authority) = details
            .lines()
            .find_map(|line| line.strip_prefix("Authority="))
        {
            Some(authority.to_owned())
        } else if details.lines().any(|line| line == "Signature=adhoc") {
            None
        } else {
            return Err("Existing Relay signature has no reusable signing identity; configure RELAY_SIGNING_IDENTITY explicitly".into());
        }
    } else {
        None
    };
    select_signing_identity(configured.as_deref(), saved.as_deref(), existing.as_deref())
}

fn start(root: &Path) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("Relay currently runs on macOS only".into());
    }
    let app = root.join("dist/Relay.app");
    if !app.join("Contents/MacOS/relay").is_file() {
        return Err("Build dist/Relay.app with cargo xtask bundle before starting Relay".into());
    }
    let metadata = metadata()?;
    let target = Path::new(
        metadata["target_directory"]
            .as_str()
            .ok_or("Missing target directory")?,
    );
    fs::create_dir_all(target)?;
    let log = target.join("relay-launch.log");
    let mut directory = std::ffi::OsString::from("RELAY_WORKING_DIRECTORY=");
    directory.push(root);
    // Launch Services gives Relay its own privacy responsibility. Only pass the
    // repository path; Relay reads .env itself, without putting keys in argv.
    if !Command::new("/usr/bin/open")
        .arg("--env")
        .arg(directory)
        .arg("--stdout")
        .arg(&log)
        .arg("--stderr")
        .arg(&log)
        .arg(&app)
        .status()?
        .success()
    {
        return Err("Could not start Relay through Launch Services".into());
    }
    println!("Opened {} (an existing instance is reused)", app.display());
    println!("Startup log: {}", log.display());
    Ok(())
}

fn bundle(root: &Path, release: bool) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("Relay currently bundles for macOS only".into());
    }
    let app = root.join("dist/Relay.app");
    let signing_identity = bundle_signing_identity(root, &app)?;
    if signing_identity == "-" {
        eprintln!(
            "Warning: ad-hoc signing can invalidate macOS privacy permissions after a rebuild. Configure RELAY_SIGNING_IDENTITY or .relay-signing-identity for a stable identity."
        );
    }
    let mut args = vec!["build", "--package", "relay", "--locked"];
    if release {
        args.push("--release");
    }
    let target = target_directory()?;
    let native = voice::prepare_engine(&target)?;
    let model = voice::prepare_model(&target)?;
    run_with_voice("cargo", &args, &native)?;
    let metadata = metadata()?;
    let target = Path::new(
        metadata["target_directory"]
            .as_str()
            .ok_or("Missing target directory")?,
    );
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    voice::bundle_resources(&model, &native, &contents.join("Resources/voice"))?;
    // Replace the executable atomically so an open development app can finish its
    // current conversation using the old inode while the next launch uses this build.
    let pending_executable = contents.join(format!("MacOS/.relay-next-{}", std::process::id()));
    fs::copy(
        target.join(if release {
            "release/relay"
        } else {
            "debug/relay"
        }),
        &pending_executable,
    )?;
    fs::rename(pending_executable, contents.join("MacOS/relay"))?;
    fs::copy(
        root.join("assets/Relay.icns"),
        contents.join("Resources/Relay.icns"),
    )?;
    fs::write(
        contents.join("Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Relay</string>
<key>CFBundleDisplayName</key><string>Relay</string>
<key>CFBundleIdentifier</key><string>com.arcwright42.relay</string>
<key>CFBundleExecutable</key><string>relay</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleIconFile</key><string>Relay</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSMicrophoneUsageDescription</key><string>Relay 使用麦克风在本机识别“嘿 relay”，并在唤醒后持续接收你的讲话。配置语音识别服务后，会将会话音频发送给该服务转写。</string>
</dict></plist>
"#,
            version = env!("CARGO_PKG_VERSION")
        ),
    )?;
    run(
        "codesign",
        &[
            "--force",
            "--sign",
            &signing_identity,
            app.to_str().ok_or("Invalid app path")?,
        ],
    )?;
    run(
        "codesign",
        &[
            "--verify",
            "--strict",
            app.to_str().ok_or("Invalid app path")?,
        ],
    )?;
    println!(
        "Built {} (signing identity: {signing_identity})",
        app.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_preserves_existing_signer_without_configuration() {
        let signer = "Developer ID Application: Example (TEAM123456)";
        assert_eq!(
            select_signing_identity(None, None, Some(signer)).unwrap(),
            signer
        );
        assert_eq!(select_signing_identity(None, None, None).unwrap(), "-");
    }

    #[test]
    fn explicit_and_saved_signers_override_the_existing_bundle() {
        assert_eq!(
            select_signing_identity(Some("override"), Some("saved"), Some("existing")).unwrap(),
            "override"
        );
        assert_eq!(
            select_signing_identity(None, Some("saved\n"), Some("existing")).unwrap(),
            "saved"
        );
        assert_eq!(
            select_signing_identity(Some("-"), None, Some("existing")).unwrap(),
            "-"
        );
    }

    #[test]
    fn invalid_signing_configuration_never_falls_back_to_another_identity() {
        for invalid in ["", " \n", "first\nsecond"] {
            assert!(
                select_signing_identity(Some(invalid), Some("saved"), Some("existing")).is_err()
            );
            assert!(select_signing_identity(None, Some(invalid), Some("existing")).is_err());
        }
    }

    fn package(name: &str, dependencies: Value) -> Value {
        serde_json::json!({"name":name,"publish":[],"version":env!("CARGO_PKG_VERSION"),"edition":"2024","rust_version":"1.97","dependencies":dependencies})
    }

    #[test]
    fn domain_cannot_depend_on_ui_or_an_agent() {
        assert!(validate_package(&package("relay-core", serde_json::json!([]))).is_ok());
        for name in ["gpui-kit", "relay-ui", "agent-client-protocol"] {
            let result = validate_package(&package(
                "relay-core",
                serde_json::json!([{"name":name,"req":"=0.6.6"}]),
            ));
            assert!(result.unwrap_err().to_string().contains("package boundary"));
        }
    }

    #[test]
    fn direct_dependencies_must_be_fixed_and_reviewed() {
        let valid = package(
            "relay-ui",
            serde_json::json!([{"name":"gpui-kit","req":"=0.6.6","source":"registry+https://github.com/rust-lang/crates.io-index"}]),
        );
        assert!(validate_package(&valid).is_ok());
        let mut floating = valid.clone();
        floating["dependencies"][0]["req"] = "^0.6".into();
        assert!(validate_package(&floating).is_err());
        let mut git = valid;
        git["dependencies"][0]["source"] = "git+https://example.com/gpui#main".into();
        assert!(validate_package(&git).is_err());
        assert!(validate_package(&package("surprise-package", serde_json::json!([]))).is_err());
    }
}
#[test]
fn adapter_packages_reject_floating_versions_and_missing_integrity() {
    let package: Value = serde_json::from_str(include_str!(
        "../../crates/relay-runtime/resources/codex/package.json"
    ))
    .unwrap();
    let lock: Value = serde_json::from_str(include_str!(
        "../../crates/relay-runtime/resources/codex/package-lock.json"
    ))
    .unwrap();
    assert!(validate_adapter_packages(&package, &lock).is_ok());
    let mut floating = package.clone();
    floating["dependencies"]["@agentclientprotocol/codex-acp"] = "latest".into();
    assert!(validate_adapter_packages(&floating, &lock).is_err());
    let mut corrupt = lock.clone();
    corrupt["packages"]["node_modules/@openai/codex"]["integrity"] = Value::Null;
    assert!(validate_adapter_packages(&package, &corrupt).is_err());
}
