//! Opt-in end-to-end check; installs managed components and connects to Codex.
//! No inference request is sent unless --prompt is supplied.
use relay_core::{
    ProjectId,
    agents::{AgentCommand, AgentService, AgentSource, ConnectionStatus, MessageRole},
};
use relay_runtime::AgentRuntime;
use std::time::{Duration, Instant};

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let prompt = args
        .iter()
        .position(|a| a == "--prompt")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let id = ProjectId(9001);
    let runtime = AgentRuntime::new(AgentRuntime::default_directory(), [id]);
    runtime.dispatch(id, AgentCommand::Connect(AgentSource::Managed))?;
    let started = Instant::now();
    let mut previous = String::new();
    let mut sent = false;
    loop {
        let snapshot = runtime.snapshot(id);
        let status = snapshot.status.label().to_owned();
        if status != previous {
            println!("{status}");
            previous = status;
        }
        if let Some(error) = &snapshot.error {
            return Err(error.clone());
        }
        match snapshot.status {
            ConnectionStatus::Ready if !sent => {
                let model = snapshot
                    .model()
                    .ok_or("Agent returned no model configuration")?;
                println!(
                    "ACP model catalog: {} choices; current: {}",
                    model.choices.len(),
                    model.current_name()
                );
                if let Some(prompt) = &prompt {
                    runtime.dispatch(id, AgentCommand::Send(prompt.clone()))?;
                    sent = true;
                } else {
                    break;
                }
            }
            ConnectionStatus::Ready if sent => {
                let reply = snapshot
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::Assistant)
                    .ok_or("Missing reply")?;
                println!("Reply: {}", reply.text);
                if reply.text.is_empty() {
                    return Err("Empty reply".into());
                }
                break;
            }
            ConnectionStatus::NeedsAuthentication => {
                return Err(
                    "Components are ready; sign in through Relay to finish the live check.".into(),
                );
            }
            ConnectionStatus::Failed => return Err("Agent connection failed".into()),
            _ => {}
        }
        for permission in snapshot.permissions {
            runtime.dispatch(
                id,
                AgentCommand::AnswerPermission {
                    id: permission.id,
                    choice: None,
                },
            )?;
        }
        if started.elapsed() > Duration::from_secs(600) {
            runtime.shutdown();
            return Err("Probe timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    runtime.shutdown();
    Ok(())
}
