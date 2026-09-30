//! Opt-in live OpenRouter routing check using disposable projects.
//! --execute also connects the installed Codex and verifies project context delivery.
use relay_core::{
    ProjectId,
    agents::{
        AgentCommand, AgentService, AgentSource, ConnectionStatus, ContextDeliveryKind, MessageRole,
    },
    projects::{ProjectCommand, ProjectDraft, ProjectService},
    routing::{RouteDecision, RouteTarget, RoutingProvider, RoutingService},
};
use relay_runtime::{AgentRuntime, JevRouter, ProjectStore};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CONTEXT_MARKER: &str = "RELAY_JEV_CONTEXT_OK_20260930";
const REUSE_PROMPT: &str = "继续 Relay 的 Rust / GPUI 桌面 Agent 客户端项目。请先确认上下文，只回复项目名称及所选联调资料里的口令；不要调用工具或修改文件。";
const NEW_PROMPT: &str = "请新建独立的半程马拉松训练项目，为我安排十二周跑步训练。此轮先只回复项目名和“已收到跑步训练需求”，不要调用工具或修改文件。";

struct Sandbox(PathBuf);
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let execute = args.iter().any(|arg| arg == "--execute");
    let model = args
        .iter()
        .position(|arg| arg == "--model")
        .map(|index| {
            args.get(index + 1)
                .map(String::as_str)
                .ok_or("--model requires an ACP model ID")
        })
        .transpose()?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let sandbox = Sandbox(
        std::env::temp_dir().join(format!("relay-jev-probe-{}-{unique}", std::process::id())),
    );
    fs::create_dir_all(&sandbox.0).map_err(|e| e.to_string())?;
    if execute {
        reuse_installed_components(&sandbox)?;
    }
    let projects = Arc::new(ProjectStore::new(sandbox.0.clone()));
    let relay = projects
        .snapshot()
        .projects
        .first()
        .ok_or("Missing probe project")?
        .clone();
    projects.apply(ProjectCommand::Edit {
        project: relay.id,
        expected_revision: relay.revision,
        draft: ProjectDraft {
            name: "Relay".into(),
            description: "Rust / GPUI 桌面 Agent 客户端：通过 ACP 接入 Codex，管理项目上下文与 Jev 项目路由。".into(),
            instructions: "此项目开发 Rust 桌面客户端 Relay，先确认收到的项目资料，再遵循用户当前请求。".into(),
        },
    })?;
    projects.apply(ProjectCommand::SaveContext {
        project: relay.id,
        expected_revision: projects
            .project(relay.id)
            .ok_or("Missing Relay project")?
            .revision,
        id: None,
        name: "联调资料".into(),
        content: format!("联调口令：{CONTEXT_MARKER}"),
        included: true,
    })?;
    projects.apply(ProjectCommand::Create(ProjectDraft {
        name: "客户门户网站".into(),
        description: "Next.js 客户门户，管理客户登录、付款账单和订单查询。".into(),
        ..Default::default()
    }))?;
    let runtime = Arc::new(AgentRuntime::new(sandbox.0.clone(), projects.clone()));
    let router = JevRouter::new(&sandbox.0, projects.clone(), runtime.clone());
    let result = check(&router, &projects, &runtime, relay.id, execute, model);
    runtime.shutdown();
    result
}

fn check(
    router: &JevRouter,
    projects: &ProjectStore,
    runtime: &AgentRuntime,
    relay: ProjectId,
    execute: bool,
    model: Option<&str>,
) -> Result<(), String> {
    let snapshot = router.snapshot();
    if !snapshot.configured || snapshot.provider != RoutingProvider::OpenRouter {
        return Err(format!(
            "Configure OPENROUTER_API_KEY first: {:?}",
            snapshot.error
        ));
    }
    if execute && !runtime.snapshot(relay).installed {
        return Err("Install Codex through Relay before running --execute".into());
    }
    println!(
        "provider=OpenRouter credential_source={:?}",
        snapshot.credential_source
    );
    let reuse = classify(
        router,
        "reuse",
        REUSE_PROMPT,
        Some(RouteTarget::Existing(relay)),
    )?;
    let new = classify(router, "new", NEW_PROMPT, Some(RouteTarget::NewProject))?;
    classify(router, "ambiguous", "继续", None)?;
    if !execute {
        return Ok(());
    }
    if projects.revision() != reuse.catalog_revision || projects.revision() != new.catalog_revision
    {
        return Err("Catalog changed before delivery".into());
    }
    run_agent(
        runtime,
        relay,
        REUSE_PROMPT,
        &["Relay", CONTEXT_MARKER],
        model,
    )?;
    let id = projects.apply(ProjectCommand::CreateAtRevision {
        expected_catalog_revision: new.catalog_revision,
        draft: ProjectDraft {
            name: NEW_PROMPT
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(60)
                .collect(),
            description: NEW_PROMPT.chars().take(600).collect(),
            instructions: String::new(),
        },
    })?;
    run_agent(
        runtime,
        id,
        NEW_PROMPT,
        &["半程马拉松", "已收到跑步训练需求"],
        model,
    )?;
    println!("PASS: live routing, project creation, ACP prompt delivery and project context");
    Ok(())
}

fn classify(
    router: &JevRouter,
    label: &str,
    prompt: &str,
    expected: Option<RouteTarget>,
) -> Result<RouteDecision, String> {
    let decision = router
        .decide(prompt)
        .map_err(|e| format!("{label}: {e:?}"))?;
    println!(
        "{label}: target={:?} confidence={:.4} elapsed_ms={} model={} options={:?}",
        decision.automatic,
        decision.confidence,
        decision.elapsed_ms,
        decision.model,
        decision.options
    );
    if decision.automatic != expected {
        return Err(format!(
            "{label}: expected {expected:?}, received {:?}",
            decision.automatic
        ));
    }
    Ok(decision)
}

fn run_agent(
    runtime: &AgentRuntime,
    project: ProjectId,
    prompt: &str,
    expected: &[&str],
    model: Option<&str>,
) -> Result<(), String> {
    runtime.dispatch(project, AgentCommand::Connect(AgentSource::Managed))?;
    let started = Instant::now();
    let mut sent = false;
    let mut selected_model = false;
    let mut previous = String::new();
    loop {
        let snapshot = runtime.snapshot(project);
        if snapshot.status.label() != previous {
            println!("agent project={}: {}", project.0, snapshot.status.label());
            previous = snapshot.status.label().into();
        }
        if let Some(error) = &snapshot.error {
            return Err(error.clone());
        }
        if !snapshot.permissions.is_empty() {
            for permission in snapshot.permissions {
                runtime.dispatch(
                    project,
                    AgentCommand::AnswerPermission {
                        id: permission.id,
                        choice: None,
                    },
                )?;
            }
            return Err("The context-only probe unexpectedly requested tool permission".into());
        }
        match snapshot.status {
            ConnectionStatus::Ready if !selected_model => {
                let config = snapshot.model().ok_or("Missing ACP model catalog")?;
                let choice = match model {
                    Some(id) => config
                        .choices
                        .iter()
                        .find(|choice| choice.id == id)
                        .ok_or("Requested probe model is not in the ACP catalog")?,
                    None => config
                        .choices
                        .iter()
                        .find(|choice| choice.id == config.current)
                        .or_else(|| config.choices.first())
                        .ok_or("Empty ACP model catalog")?,
                };
                println!(
                    "agent project={} probe_model={} available_models={:?}",
                    project.0,
                    choice.id,
                    config
                        .choices
                        .iter()
                        .map(|choice| &choice.id)
                        .collect::<Vec<_>>()
                );
                // Keep global Codex defaults untouched; use an advertised model for this probe.
                runtime.dispatch(
                    project,
                    AgentCommand::SetConfig {
                        id: config.id.clone(),
                        value: choice.id.clone(),
                    },
                )?;
                selected_model = true;
            }
            ConnectionStatus::Ready if !sent && snapshot.pending_config.is_none() => {
                runtime.dispatch(project, AgentCommand::Send(prompt.into()))?;
                sent = true;
            }
            ConnectionStatus::Ready if sent => {
                let answer = snapshot
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::Assistant)
                    .ok_or("Missing agent answer")?;
                println!("agent project={} reply={}", project.0, answer.text);
                if !expected.iter().all(|text| answer.text.contains(text)) {
                    return Err("Agent answer did not confirm the expected project context".into());
                }
                let metrics = answer
                    .metrics
                    .as_ref()
                    .ok_or("Missing context delivery metrics")?;
                if metrics.context_kind != ContextDeliveryKind::Snapshot
                    || metrics.context_bytes == 0
                {
                    return Err("The initial project context snapshot was not delivered".into());
                }
                if !answer.tools.is_empty() {
                    return Err("The context-only probe unexpectedly used tools".into());
                }
                return Ok(());
            }
            ConnectionStatus::NeedsAuthentication => {
                return Err("Sign in to Codex through Relay first".into());
            }
            ConnectionStatus::Failed => return Err("Agent connection failed".into()),
            _ => {}
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err("Agent probe timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn reuse_installed_components(sandbox: &Sandbox) -> Result<(), String> {
    let components = AgentRuntime::default_directory().join("components");
    if !components.is_dir() {
        return Err("Install Codex through Relay before running --execute".into());
    }
    std::os::unix::fs::symlink(components, sandbox.0.join("components")).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn reuse_installed_components(_: &Sandbox) -> Result<(), String> {
    Err("The --execute probe currently requires a Unix installation".into())
}
