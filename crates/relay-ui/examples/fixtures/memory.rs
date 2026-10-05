use relay_core::memory::*;
use std::sync::Mutex;

pub struct MemoryFixtures {
    forgotten: Mutex<bool>,
    pub commands: Mutex<Vec<MemoryCommand>>,
    pending_only: bool,
}
impl Default for MemoryFixtures {
    fn default() -> Self {
        Self {
            forgotten: Mutex::new(false),
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
    fn entry() -> MemoryEntry {
        MemoryEntry {
            id: 1,
            title: "任务调度和记忆分开管理".into(),
            preview: "Relay 管任务与会话，Claude-Mem 管观察记录和总结。".into(),
            kind: "decision".into(),
        }
    }
}
impl MemoryService for MemoryFixtures {
    fn provider(&self) -> MemoryProviderInfo {
        MemoryProviderInfo {
            id: "claude-mem-fixture".into(),
            name: "Claude-Mem".into(),
            capabilities: MemoryCapabilities {
                forget_memory: true,
                retry: true,
            },
            description: "Preview fixture · observations and delivery state".into(),
        }
    }
    fn overview(&self, query: &MemoryQuery) -> Result<MemoryOverview, String> {
        let entry = Self::entry();
        let visible = !self.pending_only
            && !*self.forgotten.lock().unwrap()
            && (query.text.is_empty()
                || entry.title.contains(&query.text)
                || entry.preview.contains(&query.text));
        Ok(MemoryOverview {
            progress: MemoryProgress {
                provider_status:Some(if self.pending_only {"Worker offline · 3,873 queued · 0 accepted"} else {"Worker reachable · 3,850 queued · 22 accepted · 1 failed · 2 uncertain. Accepted ≠ extracted/indexed."}.into()),
                pending: if self.pending_only {3873} else {3850},
                accepted: if self.pending_only {0} else {22},
                failed: u64::from(!self.pending_only), uncertain: if self.pending_only {0} else {2},
                ..Default::default()
            },
            memories: if visible {vec![entry]} else {vec![]}, ..Default::default()
        })
    }
    fn detail(&self, id: u64) -> Result<MemoryDetail, String> {
        if *self.forgotten.lock().unwrap() || id != 1 {
            return Err("Observation unavailable".into());
        }
        Ok(MemoryDetail {
            entry: Self::entry(),
            body: Self::entry().preview,
            concepts: vec!["architecture".into()],
        })
    }
    fn apply(&self, command: MemoryCommand) -> Result<(), String> {
        self.commands.lock().unwrap().push(command.clone());
        if let MemoryCommand::ForgetMemory(_) = command {
            *self.forgotten.lock().unwrap() = true;
        }
        Ok(())
    }
}
