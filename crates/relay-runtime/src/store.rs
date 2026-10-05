use anyhow::{Context, Result, bail};
use relay_core::{ThreadId, agents::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
pub struct SavedThread {
    #[serde(default = "format_version")]
    pub version: u32,
    pub local_codex: Option<PathBuf>,
    #[serde(flatten)]
    pub identity: SavedIdentity,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub session_key: Option<String>,
    #[serde(default)]
    pub context_checkpoint: crate::context::Checkpoint,
    #[serde(default)]
    pub restore_history: bool,
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
    #[serde(default)]
    pub messages: Vec<SavedMessage>,
}

/// Keep v3 harness metadata intact until the user explicitly connects Codex.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SavedIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_executable: Option<PathBuf>,
}

impl SavedIdentity {
    pub fn is_codex(&self) -> bool {
        self.harness.as_deref().is_none_or(|name| name == "codex")
    }
}
fn format_version() -> u32 {
    1
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedMessage {
    pub id: u64,
    pub role: String,
    pub text: String,
    pub complete: bool,
    #[serde(default)]
    pub tools: Vec<SavedTool>,
    #[serde(default)]
    pub metrics: Option<crate::metrics::SavedMetrics>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedTool {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) input: String,
    #[serde(default)]
    pub(crate) output: String,
}

impl SavedMessage {
    pub fn from_message(message: &ChatMessage) -> Self {
        Self {
            id: message.id,
            role: match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            }
            .into(),
            text: message.text.clone(),
            complete: message.status == MessageStatus::Complete,
            metrics: message.metrics.as_ref().map(Into::into),
            tools: message
                .tools
                .iter()
                .map(|t| SavedTool {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    status: t.status.clone(),
                    input: t.input.clone(),
                    output: t.output.clone(),
                })
                .collect(),
        }
    }
    pub fn into_message(self) -> ChatMessage {
        ChatMessage {
            id: self.id,
            role: if self.role == "user" {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            text: self.text,
            status: if self.complete {
                MessageStatus::Complete
            } else {
                MessageStatus::Interrupted
            },
            metrics: self.metrics.map(Into::into),
            tools: self
                .tools
                .into_iter()
                .map(|t| ToolActivity {
                    id: t.id,
                    title: t.title,
                    status: t.status,
                    input: t.input,
                    output: t.output,
                })
                .collect(),
        }
    }
}

pub fn load(root: &Path, thread: ThreadId) -> Result<SavedThread> {
    let path = root
        .join("threads")
        .join(thread.0.to_string())
        .join("conversation.json");
    if !path.try_exists()? {
        return Ok(SavedThread {
            version: 1,
            ..Default::default()
        });
    }
    let bytes = fs::read(&path).with_context(|| format!("Reading {}", path.display()))?;
    let saved: SavedThread =
        serde_json::from_slice(&bytes).with_context(|| format!("Reading {}", path.display()))?;
    if !matches!(saved.version, 1..=3) {
        bail!(
            "Unsupported conversation format v{} in {}. This Relay supports v1–v3. The file has been preserved.",
            saved.version,
            path.display()
        );
    }
    if saved
        .context_checkpoint
        .acknowledged
        .as_ref()
        .is_some_and(|snapshot| snapshot.thread_id != thread.0)
    {
        bail!("Conversation context belongs to another thread. The file has been preserved.");
    }
    Ok(saved)
}

pub fn save(root: &Path, thread: ThreadId, saved: &SavedThread) -> Result<()> {
    let directory = root.join("threads").join(thread.0.to_string());
    write_json(&directory.join("conversation.json"), saved)
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let directory = path.parent().context("Missing storage directory")?;
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(".relay-{}.pending", crate::installer::unique_id()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_response_remains_visible_after_reload() {
        let message = ChatMessage {
            id: 7,
            role: MessageRole::Assistant,
            text: "Partial response".into(),
            status: MessageStatus::Streaming,
            tools: vec![],
            metrics: None,
        };
        let saved = SavedMessage::from_message(&message);
        let json = serde_json::to_string(&saved).unwrap();
        let restored = serde_json::from_str::<SavedMessage>(&json)
            .unwrap()
            .into_message();
        assert_eq!(restored.status, MessageStatus::Interrupted);
        assert_eq!(restored.text, message.text);
    }
}
