//! Headless configuration and diagnostics; API keys are accepted only on stdin.
use super::*;
use std::io::{BufRead, Read};
use std::path::Path;

pub fn run_memory_cli(root: &Path, args: &[String]) -> Result<bool> {
    let Some(command) = args.first().filter(|s| s.starts_with("--memory-")) else {
        return Ok(false);
    };
    let store = || NativeStore::open(root.to_owned());
    match (command.as_str(), args.len()) {
        ("--memory-configure", 3 | 4) => {
            ensure!(
                args.len() == 3 || args[3] == "--key-stdin",
                "Only --key-stdin is supported; never pass an API key in arguments"
            );
            let mut key = String::new();
            if args.len() == 4 {
                std::io::stdin()
                    .lock()
                    .take(4098)
                    .read_line(&mut key)
                    .context("Could not read API key from stdin")?;
            }
            let config = EmbeddingConfig {
                base_url: args[1].clone(),
                model: args[2].clone(),
                dimensions: None,
                chunk_chars: 2048,
                batch_size: 16,
            };
            configure_embeddings(root, &config, (args.len() == 4).then_some(key.as_str()))?;
            println!("Embedding configuration saved. API keys are stored only in macOS Keychain.");
        }
        ("--memory-check", 1) => println!("{}", super::embedding::check(root)?),
        ("--memory-status", 1) => println!("{}", store()?.call(MAIN, "memory_status", &json!({}))?),
        ("--memory-query", 2) => println!(
            "{}",
            store()?.call(MAIN, "memory_search", &json!({"query":args[1]}))?
        ),
        ("--memory-retry", 1) => {
            store()?.retry_embeddings()?;
            println!("Embedding retries queued.");
        }
        ("--memory-index", 1) => {
            let store = store()?;
            let mut indexed = 0;
            loop {
                let n = store.index_embedding_batch()?;
                if n == 0 {
                    break;
                }
                indexed += n;
            }
            println!(
                "{}",
                json!({"indexed":indexed,"status":store.embedding_status()?})
            );
        }
        ("--memory-reindex", 1) => {
            store()?.rebuild_embeddings()?;
            println!("Embedding rebuild queued.");
        }
        _ => bail!(
            "Usage: --memory-configure <Base URL> <model> [--key-stdin], --memory-check, --memory-status, --memory-query <text>, --memory-index, --memory-reindex, or --memory-retry"
        ),
    }
    Ok(true)
}
