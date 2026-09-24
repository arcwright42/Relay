use serde_json::Value;
use std::{error::Error, fs, path::Path, process::Command};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    std::env::set_current_dir(root)?;
    match std::env::args().nth(1).as_deref() {
        Some("check-packages") => check_packages(),
        Some("verify") => {
            check_packages()?;
            run("cargo", &["fmt", "--all", "--", "--check"])?;
            run(
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
            )?;
            run(
                "cargo",
                &["test", "--workspace", "--all-features", "--locked"],
            )
        }
        Some("icon") => build_icon(root),
        Some("bundle") => bundle(root, std::env::args().any(|arg| arg == "--release")),
        _ => {
            println!("cargo xtask <check-packages | verify | icon | bundle [--release]>");
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

fn validate_package(package: &Value) -> Result<()> {
    let name = package["name"].as_str().ok_or("Missing package name")?;
    let allowed: &[&str] = match name {
        "relay" => &["gpui-kit", "relay-ui", "relay-core", "relay-runtime"],
        "relay-ui" => &["gpui-kit", "relay-core"],
        "relay-core" => &[],
        "relay-runtime" => &[
            "anyhow",
            "relay-core",
            "relay-acp",
            "serde",
            "serde_json",
            "sha2",
            "ureq",
            "security-framework",
        ],
        "relay-acp" => &[
            "agent-client-protocol",
            "async-channel",
            "async-io",
            "futures-lite",
            "relay-core",
            "serde_json",
        ],
        "xtask" => &["serde_json"],
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
    let metadata = metadata()?;
    for package in metadata["packages"].as_array().ok_or("Missing packages")? {
        validate_package(package)?;
        println!(
            "{}: package boundary OK",
            package["name"].as_str().unwrap_or("unknown")
        );
    }
    validate_managed_packages(
        &serde_json::from_str(include_str!(
            "../../crates/relay-runtime/resources/codex/package.json"
        ))?,
        &serde_json::from_str(include_str!(
            "../../crates/relay-runtime/resources/codex/package-lock.json"
        ))?,
    )?;
    println!("managed Codex: package versions and integrity lock OK");
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

fn validate_managed_packages(package: &Value, lock: &Value) -> Result<()> {
    if package["private"] != true || lock["lockfileVersion"] != 3 {
        return Err("Managed components must be private and use npm lockfile v3".into());
    }
    let dependencies = package["dependencies"]
        .as_object()
        .ok_or("Missing managed dependencies")?;
    if dependencies.len() != 2
        || !dependencies.contains_key("@agentclientprotocol/codex-acp")
        || !dependencies.contains_key("@openai/codex")
    {
        return Err("Review the managed Codex package boundary before adding dependencies".into());
    }
    if lock["packages"][""]["dependencies"] != package["dependencies"]
        || package["overrides"]["@openai/codex"] != package["dependencies"]["@openai/codex"]
    {
        return Err("Managed dependency manifest, override, and lockfile disagree".into());
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

fn bundle(root: &Path, release: bool) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("Relay currently bundles for macOS only".into());
    }
    let mut args = vec!["build", "--package", "relay", "--locked"];
    if release {
        args.push("--release");
    }
    run("cargo", &args)?;
    let metadata = metadata()?;
    let target = Path::new(
        metadata["target_directory"]
            .as_str()
            .ok_or("Missing target directory")?,
    );
    let app = root.join("dist/Relay.app");
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
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
            "-",
            app.to_str().ok_or("Invalid app path")?,
        ],
    )?;
    println!("Built {}", app.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
fn managed_packages_reject_floating_versions_and_missing_integrity() {
    let package: Value = serde_json::from_str(include_str!(
        "../../crates/relay-runtime/resources/codex/package.json"
    ))
    .unwrap();
    let lock: Value = serde_json::from_str(include_str!(
        "../../crates/relay-runtime/resources/codex/package-lock.json"
    ))
    .unwrap();
    assert!(validate_managed_packages(&package, &lock).is_ok());
    let mut floating = package.clone();
    floating["dependencies"]["@openai/codex"] = "latest".into();
    assert!(validate_managed_packages(&floating, &lock).is_err());
    let mut corrupt = lock.clone();
    corrupt["packages"]["node_modules/@openai/codex"]["integrity"] = Value::Null;
    assert!(validate_managed_packages(&package, &corrupt).is_err());
}
