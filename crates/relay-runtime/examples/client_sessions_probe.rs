//! Opt-in local import check. Reads native sessions without connecting or prompting any agent.
use relay_core::sessions::{ClientSessionsCommand, ClientSessionsService};
use relay_runtime::{AgentRuntime, ClientSessionStore};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn wait(store: &ClientSessionStore) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let view = store.snapshot();
        if !view.syncing {
            println!(
                "sessions={} scanned={} updated={} skipped={} interval_secs={}",
                view.sessions.len(),
                view.scanned_files,
                view.updated_sessions,
                view.failed_files,
                view.next_sync_unix
                    .unwrap_or(0)
                    .saturating_sub(view.last_sync_unix.unwrap_or(0))
            );
            if let Some(error) = view.error {
                println!("diagnostic: {error}");
            }
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(600) {
            return Err("Local session import timed out".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let root = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| AgentRuntime::default_directory().join("probes/client-sessions"));
    let store = ClientSessionStore::new(root);
    wait(&store)?;
    store.dispatch(ClientSessionsCommand::Sync)?;
    wait(&store)?;
    store.shutdown();
    Ok(())
}
