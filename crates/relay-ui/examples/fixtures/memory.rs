use relay_core::memory::*;
use std::sync::Mutex;

pub struct MemoryFixtures {
    state: Mutex<(u64, String, String, bool, bool)>,
    pub commands: Mutex<Vec<MemoryCommand>>,
    pending_only: bool,
}
impl Default for MemoryFixtures {
    fn default() -> Self {
        Self {
            state: Mutex::new((
                1,
                "跨会话记忆保留来源".into(),
                "记忆记录需要关联原始会话与来源版本，修订时保留证据。".into(),
                false,
                false,
            )),
            commands: Mutex::new(vec![]),
            pending_only: false,
        }
    }
}
impl MemoryFixtures {
    pub fn pending() -> Self {
        Self {
            pending_only: true,
            ..Default::default()
        }
    }
}
impl MemoryService for MemoryFixtures {
    fn overview(&self, query: &MemoryQuery) -> Result<MemoryOverview, String> {
        if self.pending_only {
            return Ok(MemoryOverview {
                progress: MemoryProgress {
                    sources: 3873,
                    sessions: 629,
                    pending: 3873,
                    enabled: true,
                    hourly_budget: 60,
                    ..Default::default()
                },
                sources: [10, 11]
                    .map(|id| MemorySourceEntry {
                        state: "pending".into(),
                        error: None,
                        ..source(id)
                    })
                    .into(),
                ..Default::default()
            });
        }
        let state = self.state.lock().unwrap();
        let entry = MemoryEntry {
            id: state.0,
            title: state.1.clone(),
            preview: state.2.clone(),
            kind: "decision".into(),
            status: if state.3 { "confirmed" } else { "candidate" }.into(),
        };
        let visible = !state.4
            && (query.text.is_empty()
                || state.1.contains(&query.text)
                || state.2.contains(&query.text));
        Ok(MemoryOverview {
            progress: MemoryProgress {
                sources: 3873,
                sessions: 629,
                pending: 3850,
                done: 22,
                failed: 1,
                candidates: (!state.3 && !state.4) as u64,
                confirmed: (state.3 && !state.4) as u64,
                enabled: true,
                hourly_budget: 60,
                runs_this_hour: 23,
                ..Default::default()
            },
            memories: if visible { vec![entry] } else { vec![] },
            topics: if state.4 {
                vec![]
            } else {
                vec![MemoryTopic {
                    name: "relay".into(),
                    memories: 1,
                    sessions: 2,
                }]
            },
            sources: vec![source(10), source(11)],
            failures: vec![source(11)],
            ..Default::default()
        })
    }
    fn detail(&self, id: u64) -> Result<MemoryDetail, String> {
        let state = self.state.lock().unwrap();
        if state.4 || id != state.0 {
            return Err("Memory changed".into());
        }
        Ok(MemoryDetail {
            entry: MemoryEntry {
                id,
                title: state.1.clone(),
                preview: state.2.clone(),
                kind: "decision".into(),
                status: if state.3 { "confirmed" } else { "candidate" }.into(),
            },
            body: state.2.clone(),
            topics: vec!["relay".into()],
            evidence: vec![MemoryEvidence {
                id: 10,
                revision: 1,
                title: "原生记忆设计讨论".into(),
                origin: "client:codex:local-design-session".into(),
            }],
        })
    }
    fn source(&self, id: u64, offset: usize) -> Result<MemorySourcePage, String> {
        Ok(MemorySourcePage {
            source: if self.pending_only {MemorySourceEntry {state: "pending".into(), error: None, ..source(id)}} else {source(id)},
            revision: 1,
            body: "user\n决定保留来源与版本，历史会话仍按原身份保存。\n\nassistant\n将提炼结果作为候选记忆，用户可以确认和修订。".into(),
            offset,
            next_offset: None,
            archive: Some(relay_core::sessions::ClientSessionId("codex:local-design-session".into())),
        })
    }
    fn apply(&self, command: MemoryCommand) -> Result<Option<u64>, String> {
        self.commands.lock().unwrap().push(command.clone());
        let mut state = self.state.lock().unwrap();
        match command {
            MemoryCommand::Confirm(_) => {
                state.0 += 1;
                state.3 = true;
                Ok(Some(state.0))
            }
            MemoryCommand::Revise { title, body, .. } => {
                state.0 += 1;
                state.1 = title;
                state.2 = body;
                state.3 = true;
                Ok(Some(state.0))
            }
            MemoryCommand::ForgetMemory(_) => {
                state.4 = true;
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
fn source(id: u64) -> MemorySourceEntry {
    MemorySourceEntry {
        id,
        title: if id == 10 {
            "原生记忆设计讨论"
        } else {
            "客户端历史导入"
        }
        .into(),
        origin: "client:codex:local-design-session".into(),
        state: if id == 10 { "done" } else { "failed" }.into(),
        error: (id == 11)
            .then(|| "Observer returned no complete extraction result after 3 attempts.".into()),
    }
}
