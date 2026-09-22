use anyhow::{Context, Result, bail};
use relay_core::{ProjectId, agents::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
pub struct SavedProject {
    #[serde(default = "format_version")]
    pub version: u32,
    pub local_codex: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub session_key: Option<String>,
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
    #[serde(default)]
    pub messages: Vec<SavedMessage>,
}
fn format_version() -> u32 {
    1
}

#[derive(Serialize, Deserialize)]
pub struct SavedMessage {
    pub id: u64,
    pub role: String,
    pub text: String,
    pub complete: bool,
    #[serde(default)]
    pub tools: Vec<SavedTool>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedTool {
    id: String,
    title: String,
    status: String,
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
            tools: message
                .tools
                .iter()
                .map(|t| SavedTool {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    status: t.status.clone(),
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
            tools: self
                .tools
                .into_iter()
                .map(|t| ToolActivity {
                    id: t.id,
                    title: t.title,
                    status: t.status,
                })
                .collect(),
        }
    }
}

pub fn load(root: &Path, project: ProjectId) -> Result<SavedProject> {
    let path = root
        .join("projects")
        .join(project.0.to_string())
        .join("conversation.json");
    if !path.exists() {
        return Ok(SavedProject {
            version: 1,
            ..Default::default()
        });
    }
    let bytes = fs::read(path)?;
    let saved: SavedProject =
        serde_json::from_slice(&bytes).context("Reading project conversation")?;
    if saved.version != 1 {
        bail!("This project was saved by a newer Relay version.");
    }
    Ok(saved)
}

pub fn save(root: &Path, project: ProjectId, saved: &SavedProject) -> Result<()> {
    let directory = root.join("projects").join(project.0.to_string());
    fs::create_dir_all(&directory)?;
    let temporary = directory.join("conversation.json.pending");
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(saved)?)?;
    file.sync_all()?;
    fs::rename(temporary, directory.join("conversation.json"))?;
    Ok(())
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
