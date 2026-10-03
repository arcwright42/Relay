//! Headless Jev credential setup through the signed application's Keychain identity.
use relay_core::routing::RoutingProvider;
use std::{
    io::{BufRead, Read},
    path::Path,
};

pub(super) fn run(root: &Path) -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("--jev-save-key") {
        return false;
    }
    let result = if args.len() == 1 {
        save(root)
    } else {
        Err("Usage: --jev-save-key (OpenRouter key on stdin; never in arguments).".into())
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
    true
}
fn save(root: &Path) -> Result<(), String> {
    let mut key = String::new();
    std::io::stdin()
        .lock()
        .take(4098)
        .read_line(&mut key)
        .map_err(|_| "Could not read Jev key from stdin.")?;
    relay_runtime::save_routing_key(root, RoutingProvider::OpenRouter, &key)
        .map_err(|error| format!("Could not save Jev credential: {error:?}"))?;
    println!("Jev OpenRouter credential saved to macOS Keychain.");
    Ok(())
}
