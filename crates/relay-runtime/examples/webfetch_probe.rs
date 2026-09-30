//! Explicit network smoke check against a supplied public URL; never reads project data.
use relay_core::capture::WebFetchService;
use relay_runtime::{AgentRuntime, MoliFetcher};
fn main() -> Result<(), String> {
    let url = std::env::args()
        .nth(1)
        .ok_or("Provide a public HTTP(S) URL")?;
    let fetcher = MoliFetcher::new(&AgentRuntime::default_directory().join("probes/webfetch"));
    let result = fetcher.fetch(&url);
    fetcher.shutdown();
    let body = result?;
    println!(
        "Fetched {} characters: {}",
        body.chars().count(),
        body.lines().next().unwrap_or_default()
    );
    Ok(())
}
