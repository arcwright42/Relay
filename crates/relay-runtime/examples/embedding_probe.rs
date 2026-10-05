//! The same headless memory commands as Relay, with an explicit data directory.
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 2,
        "Usage: embedding_probe <data directory> --memory-<command> ..."
    );
    anyhow::ensure!(
        relay_runtime::native::run_memory_cli(std::path::Path::new(&args[0]), &args[1..])?,
        "Unknown memory command"
    );
    Ok(())
}
