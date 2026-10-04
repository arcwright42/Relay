//! Prompt routing and correlated project turns; no audio, UI or speech SDKs.
use relay_core::{ProjectId, agents::*, projects::*, routing::*, voice::*};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct ProjectVoiceDialogue {
    routing: Arc<dyn RoutingService>,
    projects: Arc<dyn ProjectService>,
    agents: Arc<dyn AgentService>,
}
impl ProjectVoiceDialogue {
    pub fn new(
        routing: Arc<dyn RoutingService>,
        projects: Arc<dyn ProjectService>,
        agents: Arc<dyn AgentService>,
    ) -> Self {
        Self {
            routing,
            projects,
            agents,
        }
    }

    fn ready(&self, project: ProjectId, cancelled: &AtomicBool) -> Result<(), VoiceTurnError> {
        check_cancelled(cancelled)?;
        let state = self.agents.snapshot(project);
        if matches!(
            state.status,
            ConnectionStatus::Running | ConnectionStatus::Cancelling
        ) {
            return Err(VoiceTurnError::AgentBusy);
        }
        if matches!(
            state.status,
            ConnectionStatus::Disconnected | ConnectionStatus::Failed
        ) {
            self.agents
                .dispatch(project, AgentCommand::Connect(state.source))
                .map_err(VoiceTurnError::Agent)?;
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            check_cancelled(cancelled)?;
            let state = self.agents.snapshot(project);
            match state.status {
                ConnectionStatus::Ready if state.pending_config.is_none() => return Ok(()),
                ConnectionStatus::Running | ConnectionStatus::Cancelling => {
                    return Err(VoiceTurnError::AgentBusy);
                }
                ConnectionStatus::NeedsAuthentication | ConnectionStatus::Authenticating => {
                    return Err(VoiceTurnError::AgentAuthentication);
                }
                ConnectionStatus::Disconnected | ConnectionStatus::Failed => {
                    return Err(VoiceTurnError::Agent(
                        state
                            .error
                            .unwrap_or_else(|| "The project agent is unavailable.".into()),
                    ));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err(VoiceTurnError::Agent(
                    "The project agent did not become ready in time.".into(),
                ));
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), VoiceTurnError> {
    if cancelled.load(Ordering::Acquire) {
        Err(VoiceTurnError::Cancelled)
    } else {
        Ok(())
    }
}

struct OwnedTurn<'a> {
    agents: &'a dyn AgentService,
    project: ProjectId,
    response_id: u64,
    complete: bool,
}
impl Drop for OwnedTurn<'_> {
    fn drop(&mut self) {
        if !self.complete {
            let _ = self.agents.dispatch(
                self.project,
                AgentCommand::CancelTurn {
                    response_id: self.response_id,
                },
            );
        }
    }
}

impl VoicePromptService for ProjectVoiceDialogue {
    fn respond(
        &self,
        prompt: &str,
        selection: Option<VoiceRouteSelection>,
        cancelled: &AtomicBool,
        progress: &dyn Fn(VoicePromptProgress),
    ) -> Result<VoicePromptResult, VoiceTurnError> {
        check_cancelled(cancelled)?;
        progress(VoicePromptProgress::Stage(VoiceTurnStage::Routing));
        let mut catalog = self.projects.snapshot();
        if catalog.error.is_some() {
            return Err(VoiceTurnError::Routing(RoutingError::CatalogUnavailable));
        }
        let (target, revision) = if let Some(selection) = selection {
            (Some(selection.target), selection.catalog_revision)
        } else {
            let decision = self
                .routing
                .decide(prompt)
                .map_err(VoiceTurnError::Routing)?;
            check_cancelled(cancelled)?;
            catalog = self.projects.snapshot();
            if catalog.error.is_some() {
                return Err(VoiceTurnError::Routing(RoutingError::CatalogUnavailable));
            }
            if decision.catalog_revision != catalog.revision {
                return Err(VoiceTurnError::CatalogChanged);
            }
            if decision.automatic.is_none() {
                let choices = decision
                    .options
                    .into_iter()
                    .filter_map(|option| {
                        Some(VoiceRouteChoice {
                            target: option.target,
                            project_name: match option.target {
                                RouteTarget::Existing(id) => Some(
                                    catalog
                                        .projects
                                        .iter()
                                        .find(|project| project.id == id)?
                                        .name
                                        .clone(),
                                ),
                                RouteTarget::NewProject => None,
                            },
                        })
                    })
                    .collect();
                return Ok(VoicePromptResult::ChooseProject {
                    catalog_revision: decision.catalog_revision,
                    choices,
                });
            }
            (decision.automatic, decision.catalog_revision)
        };
        if revision != self.projects.revision() {
            return Err(VoiceTurnError::CatalogChanged);
        }
        check_cancelled(cancelled)?;
        let project = match target.ok_or(VoiceTurnError::CatalogChanged)? {
            RouteTarget::Existing(project) => project,
            RouteTarget::NewProject => self
                .projects
                .apply(ProjectCommand::CreateAtRevision {
                    expected_catalog_revision: revision,
                    draft: ProjectDraft {
                        name: prompt
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(60)
                            .collect(),
                        description: prompt.chars().take(600).collect(),
                        instructions: String::new(),
                    },
                })
                .map_err(VoiceTurnError::Agent)?,
        };
        let definition = self
            .projects
            .project(project)
            .ok_or(VoiceTurnError::CatalogChanged)?;
        progress(VoicePromptProgress::Project(VoiceProject {
            id: project,
            name: definition.name,
        }));
        progress(VoicePromptProgress::Stage(VoiceTurnStage::Connecting));
        self.ready(project, cancelled)?;
        check_cancelled(cancelled)?;
        let response_id = self
            .agents
            .send_turn(project, prompt.to_owned())
            .map_err(VoiceTurnError::Agent)?;
        let mut owned = OwnedTurn {
            agents: &*self.agents,
            project,
            response_id,
            complete: false,
        };
        let mut prior_text = String::new();
        loop {
            check_cancelled(cancelled)?;
            let state = self.agents.snapshot(project);
            progress(VoicePromptProgress::Stage(
                if state.permissions.is_empty() {
                    VoiceTurnStage::WaitingForAgent
                } else {
                    VoiceTurnStage::NeedsAttention
                },
            ));
            let response = state
                .messages
                .iter()
                .find(|message| message.id == response_id)
                .ok_or_else(|| {
                    VoiceTurnError::Agent(
                        "The project conversation changed before the voice reply completed.".into(),
                    )
                })?;
            if response.text != prior_text {
                prior_text.clone_from(&response.text);
                progress(VoicePromptProgress::Response(
                    response.text.chars().take(32_000).collect(),
                ));
            }
            match response.status {
                MessageStatus::Complete => {
                    owned.complete = true;
                    if matches!(
                        response
                            .metrics
                            .as_ref()
                            .and_then(|metrics| metrics.outcome),
                        Some(TurnOutcome::Cancelled | TurnOutcome::Failed)
                    ) {
                        return Err(VoiceTurnError::Agent(
                            "The voice turn did not finish successfully.".into(),
                        ));
                    }
                    return Ok(VoicePromptResult::Reply(response.text.clone()));
                }
                MessageStatus::Interrupted => {
                    owned.complete = true;
                    return Err(VoiceTurnError::Agent(
                        state
                            .error
                            .unwrap_or_else(|| "The voice turn was interrupted.".into()),
                    ));
                }
                MessageStatus::Streaming => {}
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
}
