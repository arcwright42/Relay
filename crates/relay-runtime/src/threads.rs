use anyhow::{Context, Result, bail, ensure};
use relay_core::{ThreadId, threads::*};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io::ErrorKind, path::PathBuf, sync::Mutex};

#[derive(Clone, Serialize, Deserialize)]
struct SavedItem {
    id: u64,
    name: String,
    content: String,
    included: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedMemory {
    id: u64,
    kind: String,
    name: String,
    content: String,
    source_message_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_source: Option<SavedClientSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedClientSource {
    pub client: String,
    pub session_id: String,
    pub message_id: u64,
}

fn first_memory_id() -> u64 {
    1
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedDefinition {
    id: u64,
    revision: u64,
    name: String,
    description: String,
    instructions: String,
    next_context_id: u64,
    context: Vec<SavedItem>,
    #[serde(default = "first_memory_id")]
    next_memory_id: u64,
    #[serde(default)]
    memory: Vec<SavedMemory>,
}

impl SavedDefinition {
    fn new(id: u64, draft: ThreadDraft) -> Self {
        Self {
            id,
            revision: 1,
            name: draft.name.trim().into(),
            description: draft.description,
            instructions: draft.instructions,
            next_context_id: 1,
            context: vec![],
            next_memory_id: 1,
            memory: vec![],
        }
    }

    fn thread(&self) -> Thread {
        Thread {
            id: ThreadId(self.id),
            revision: self.revision,
            name: self.name.clone(),
            description: self.description.clone(),
            instructions: self.instructions.clone(),
            context: self
                .context
                .iter()
                .map(|item| ContextItem {
                    id: ContextId(item.id),
                    name: item.name.clone(),
                    content: item.content.clone(),
                    included: item.included,
                })
                .collect(),
            memory: self
                .memory
                .iter()
                .map(|item| MemoryItem {
                    id: MemoryId(item.id),
                    kind: MemoryKind::from_code(&item.kind).expect("validated memory kind"),
                    name: item.name.clone(),
                    content: item.content.clone(),
                    source: item
                        .source_message_id
                        .map(|message_id| MemorySource::Message { message_id })
                        .or_else(|| {
                            item.client_source
                                .as_ref()
                                .map(|s| MemorySource::ClientSession {
                                    client: s.client.clone(),
                                    session_id: s.session_id.clone(),
                                    message_id: s.message_id,
                                })
                        }),
                })
                .collect(),
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.id > 0 && self.revision > 0,
            "Invalid thread identifier or revision."
        );
        validate_name(&self.name)?;
        ensure!(
            self.description.chars().count() <= 1_000,
            "Description is limited to 1,000 characters."
        );
        ensure!(
            self.instructions.chars().count() <= MAX_INSTRUCTIONS_CHARS,
            "Thread instructions are limited to {MAX_INSTRUCTIONS_CHARS} characters."
        );
        ensure!(
            self.context.len() <= MAX_CONTEXT_ITEMS,
            "A thread can contain up to {MAX_CONTEXT_ITEMS} notes."
        );
        let mut ids = BTreeSet::new();
        let mut included = self.instructions.chars().count()
            + self.name.chars().count()
            + self.description.chars().count();
        for item in &self.context {
            ensure!(
                item.id > 0 && item.id < self.next_context_id && ids.insert(item.id),
                "Invalid context identifier."
            );
            validate_name(&item.name)?;
            ensure!(
                !item.content.trim().is_empty(),
                "Note content cannot be empty."
            );
            ensure!(
                item.content.chars().count() <= MAX_ITEM_CHARS,
                "Each note is limited to {MAX_ITEM_CHARS} characters."
            );
            if item.included {
                included += item.name.chars().count() + item.content.chars().count();
            }
        }
        ensure!(self.next_context_id > 0, "Invalid next context identifier.");
        ensure!(
            self.memory.len() <= MAX_MEMORY_ITEMS,
            "A thread can contain up to {MAX_MEMORY_ITEMS} memories."
        );
        ensure!(self.next_memory_id > 0, "Invalid next memory identifier.");
        let mut memory_ids = BTreeSet::new();
        for item in &self.memory {
            ensure!(
                item.id > 0 && item.id < self.next_memory_id && memory_ids.insert(item.id),
                "Invalid memory identifier."
            );
            ensure!(
                MemoryKind::from_code(&item.kind).is_some(),
                "Unknown memory kind."
            );
            validate_name(&item.name)?;
            ensure!(
                !item.content.trim().is_empty(),
                "Memory content cannot be empty."
            );
            ensure!(
                item.content.chars().count() <= MAX_MEMORY_CHARS,
                "Each memory is limited to {MAX_MEMORY_CHARS} characters."
            );
            ensure!(
                item.source_message_id != Some(0),
                "Invalid memory source message."
            );
            if let Some(source) = &item.client_source {
                ensure!(
                    item.source_message_id.is_none()
                        && source.message_id > 0
                        && !source.client.trim().is_empty()
                        && source.client.len() <= 32
                        && !source.session_id.trim().is_empty()
                        && source.session_id.len() <= 128,
                    "Invalid native client memory source."
                );
            }
            included += item.name.chars().count() + item.content.chars().count();
        }
        ensure!(
            included <= MAX_SELECTED_CONTEXT_CHARS,
            "Selected context and thread memory are limited to {MAX_SELECTED_CONTEXT_CHARS} characters. Reduce memory or deselect some notes before adding more."
        );
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(!name.trim().is_empty(), "Name cannot be empty.");
    ensure!(
        name.chars().count() <= MAX_NAME_CHARS,
        "Name is limited to {MAX_NAME_CHARS} characters."
    );
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedCatalog {
    version: u32,
    next_thread_id: u64,
    threads: Vec<SavedDefinition>,
}

impl SavedCatalog {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.version, 1..=3),
            "Unsupported thread catalog version {}. The file has been preserved.",
            self.version
        );
        let mut ids = BTreeSet::new();
        ensure!(!self.threads.is_empty(), "Thread catalog cannot be empty.");
        for thread in &self.threads {
            ensure!(
                thread.id < self.next_thread_id && ids.insert(thread.id),
                "Invalid thread identifier."
            );
            thread.validate()?;
            ensure!(
                self.version >= 3 || thread.memory.iter().all(|m| m.client_source.is_none()),
                "Native memory sources require thread catalog v3."
            );
        }
        Ok(())
    }
}

struct State {
    saved: Option<SavedCatalog>,
    view: ThreadCatalog,
}

/// Disk writes are serialized, but never hold the read lock during I/O.
/// Call `apply` on a background worker; readers see only successfully saved revisions.
pub struct ThreadStore {
    root: PathBuf,
    state: Mutex<State>,
    writer: Mutex<()>,
}

impl ThreadStore {
    pub fn new(root: PathBuf) -> Self {
        let loaded = Self::load_or_migrate(&root);
        let view = match &loaded {
            Ok(saved) => ThreadCatalog {
                revision: 1,
                threads: saved.threads.iter().map(SavedDefinition::thread).collect(),
                error: None,
            },
            Err(error) => ThreadCatalog {
                revision: 1,
                threads: vec![],
                error: Some(format!(
                    "Could not read threads: {error:#}. Existing files have been preserved."
                )),
            },
        };
        Self {
            root,
            state: Mutex::new(State {
                saved: loaded.ok(),
                view,
            }),
            writer: Mutex::new(()),
        }
    }

    fn load_or_migrate(root: &std::path::Path) -> Result<SavedCatalog> {
        match fs::read(root.join("threads.json")) {
            Ok(bytes) => {
                let saved: SavedCatalog =
                    serde_json::from_slice(&bytes).context("Reading thread catalog")?;
                saved.validate()?;
                return Ok(saved);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut threads = Vec::new();
        // These are the only product IDs in the preview version. Probe ID 9001 is not a user thread.
        for (id, name) in [(1, "Product Design"), (2, "Agent Infra"), (3, "Personal")] {
            if root
                .join(format!("threads/{id}/conversation.json"))
                .try_exists()?
            {
                threads.push(SavedDefinition::new(
                    id,
                    ThreadDraft {
                        name: name.into(),
                        ..Default::default()
                    },
                ));
            }
        }
        let mut next_id = 1;
        // Reserve every existing directory, including probe/unknown IDs, so none can be overwritten.
        match fs::read_dir(root.join("threads")) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(id) = entry?.file_name().to_string_lossy().parse::<u64>() {
                        next_id =
                            next_id.max(id.checked_add(1).context("Thread identifier exhausted")?);
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if threads.is_empty() {
            threads.push(SavedDefinition::new(
                next_id,
                ThreadDraft {
                    name: "My thread".into(),
                    ..Default::default()
                },
            ));
            next_id = next_id
                .checked_add(1)
                .context("Thread identifier exhausted")?;
        }
        let saved = SavedCatalog {
            version: 2,
            next_thread_id: next_id,
            threads,
        };
        saved.validate()?;
        super::store::write_json(&root.join("threads.json"), &saved)?;
        Ok(saved)
    }

    fn transaction(&self, command: ThreadCommand) -> Result<ThreadId> {
        let _writer = self.writer.lock().expect("thread writer lock");
        let command = match command {
            ThreadCommand::CreateAtRevision {
                expected_catalog_revision,
                draft,
            } => {
                ensure!(
                    self.revision() == expected_catalog_revision,
                    "The thread list changed. Review the destination and try again."
                );
                ThreadCommand::Create(draft)
            }
            command => command,
        };
        let mut saved = self
            .state
            .lock()
            .expect("thread catalog lock")
            .saved
            .clone()
            .context("Resolve the thread catalog error and restart Relay before editing threads")?;
        let id = if let ThreadCommand::Create(draft) = command {
            ensure!(
                saved.threads.len() < 128,
                "A workspace can contain up to 128 threads."
            );
            let mut id = saved.next_thread_id;
            while self.root.join(format!("threads/{id}")).try_exists()? {
                id = id.checked_add(1).context("Thread identifier exhausted")?;
            }
            saved.next_thread_id = id.checked_add(1).context("Thread identifier exhausted")?;
            saved.threads.push(SavedDefinition::new(id, draft));
            ThreadId(id)
        } else {
            let (id, expected) = match &command {
                ThreadCommand::Edit {
                    thread,
                    expected_revision,
                    ..
                }
                | ThreadCommand::SaveContext {
                    thread,
                    expected_revision,
                    ..
                }
                | ThreadCommand::RemoveContext {
                    thread,
                    expected_revision,
                    ..
                }
                | ThreadCommand::SaveMemory {
                    thread,
                    expected_revision,
                    ..
                }
                | ThreadCommand::RemoveMemory {
                    thread,
                    expected_revision,
                    ..
                } => (*thread, *expected_revision),
                ThreadCommand::Create(_) | ThreadCommand::CreateAtRevision { .. } => {
                    unreachable!()
                }
            };
            let thread = saved
                .threads
                .iter_mut()
                .find(|p| p.id == id.0)
                .context("Unknown thread")?;
            ensure!(
                thread.revision == expected,
                "This thread changed while you were editing. Reopen the editor and try again."
            );
            match command {
                ThreadCommand::Edit { draft, .. } => {
                    thread.name = draft.name.trim().into();
                    thread.description = draft.description;
                    thread.instructions = draft.instructions;
                }
                ThreadCommand::SaveContext {
                    id,
                    name,
                    content,
                    included,
                    ..
                } => {
                    let item = if let Some(id) = id {
                        thread
                            .context
                            .iter_mut()
                            .find(|item| item.id == id.0)
                            .context("Unknown note")?
                    } else {
                        let id = thread.next_context_id;
                        thread.next_context_id =
                            id.checked_add(1).context("Context identifier exhausted")?;
                        thread.context.push(SavedItem {
                            id,
                            name: String::new(),
                            content: String::new(),
                            included,
                        });
                        thread.context.last_mut().expect("new note")
                    };
                    item.name = name.trim().into();
                    item.content = content;
                    item.included = included;
                }
                ThreadCommand::RemoveContext { id, .. } => {
                    let length = thread.context.len();
                    thread.context.retain(|item| item.id != id.0);
                    if length == thread.context.len() {
                        bail!("Unknown note");
                    }
                }
                ThreadCommand::SaveMemory {
                    id,
                    kind,
                    name,
                    content,
                    source,
                    ..
                } => {
                    let item = if let Some(id) = id {
                        thread
                            .memory
                            .iter_mut()
                            .find(|item| item.id == id.0)
                            .context("Unknown memory")?
                    } else {
                        let id = thread.next_memory_id;
                        thread.next_memory_id =
                            id.checked_add(1).context("Memory identifier exhausted")?;
                        thread.memory.push(SavedMemory {
                            id,
                            kind: kind.code().into(),
                            name: String::new(),
                            content: String::new(),
                            source_message_id: match &source {
                                Some(MemorySource::Message { message_id }) => Some(*message_id),
                                _ => None,
                            },
                            client_source: match source {
                                Some(MemorySource::ClientSession {
                                    client,
                                    session_id,
                                    message_id,
                                }) => Some(SavedClientSource {
                                    client,
                                    session_id,
                                    message_id,
                                }),
                                _ => None,
                            },
                        });
                        thread.memory.last_mut().expect("new memory")
                    };
                    item.kind = kind.code().into();
                    item.name = name.trim().into();
                    item.content = content;
                }
                ThreadCommand::RemoveMemory { id, .. } => {
                    let length = thread.memory.len();
                    thread.memory.retain(|item| item.id != id.0);
                    ensure!(length != thread.memory.len(), "Unknown memory");
                }
                ThreadCommand::Create(_) | ThreadCommand::CreateAtRevision { .. } => {
                    unreachable!()
                }
            }
            thread.revision = thread
                .revision
                .checked_add(1)
                .context("Thread revision exhausted")?;
            id
        };
        // Older clients must reject native provenance instead of silently discarding it.
        saved.version = saved.version.max(
            if saved
                .threads
                .iter()
                .any(|p| p.memory.iter().any(|m| m.client_source.is_some()))
            {
                3
            } else {
                2
            },
        );
        saved.validate()?;
        super::store::write_json(&self.root.join("threads.json"), &saved)?;
        let mut state = self.state.lock().expect("thread catalog lock");
        state.view.threads = saved.threads.iter().map(SavedDefinition::thread).collect();
        state.view.revision += 1;
        state.saved = Some(saved);
        Ok(id)
    }
}

impl ThreadService for ThreadStore {
    fn revision(&self) -> u64 {
        self.state
            .lock()
            .expect("thread catalog lock")
            .view
            .revision
    }
    fn thread(&self, id: ThreadId) -> Option<Thread> {
        self.state
            .lock()
            .expect("thread catalog lock")
            .saved
            .as_ref()?
            .threads
            .iter()
            .find(|p| p.id == id.0)
            .map(SavedDefinition::thread)
    }
    fn snapshot(&self) -> ThreadCatalog {
        self.state.lock().expect("thread catalog lock").view.clone()
    }
    fn apply(&self, command: ThreadCommand) -> Result<ThreadId, String> {
        self.transaction(command)
            .map_err(|error| format!("{error:#}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_memory_source_survives_edits_restart_and_context_delivery() {
        let root = root();
        let store = ThreadStore::new(root.clone());
        let source = MemorySource::ClientSession {
            client: "codex".into(),
            session_id: "native-session".into(),
            message_id: 123,
        };
        store
            .apply(ThreadCommand::SaveMemory {
                thread: ThreadId(1),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Decision,
                name: "Use local clients".into(),
                content: "Keep the native harness and its configuration".into(),
                source: Some(source.clone()),
            })
            .unwrap();
        let memory = store.thread(ThreadId(1)).unwrap().memory[0].clone();
        store
            .apply(ThreadCommand::SaveMemory {
                thread: ThreadId(1),
                expected_revision: 2,
                id: Some(memory.id),
                kind: MemoryKind::Fact,
                name: memory.name,
                content: "Updated reviewed fact".into(),
                source: None,
            })
            .unwrap();
        drop(store);
        let restored = ThreadStore::new(root.clone()).thread(ThreadId(1)).unwrap();
        assert_eq!(restored.memory[0].source, Some(source));
        let context = crate::context::Snapshot::from_thread(&restored)
            .delivery(None)
            .text
            .unwrap();
        assert!(context.contains("\"client_source\":{\"client\":\"codex\""));
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("threads.json")).unwrap()).unwrap();
        assert_eq!(saved["version"], 3);
        fs::remove_dir_all(root).unwrap();
    }
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("relay-threads-{}", crate::installer::unique_id()))
    }
    fn add(
        store: &ThreadStore,
        id: ThreadId,
        revision: u64,
        included: bool,
        content: String,
    ) -> Result<ThreadId, String> {
        store.apply(ThreadCommand::SaveContext {
            thread: id,
            expected_revision: revision,
            id: None,
            name: "Reference".into(),
            content,
            included,
        })
    }

    #[test]
    fn thread_data_survives_restart_without_any_agent_session() {
        let root = root();
        let store = ThreadStore::new(root.clone());
        let id = store
            .apply(ThreadCommand::Create(ThreadDraft {
                name: "用户项目".into(),
                instructions: "Rust only".into(),
                description: "Local work".into(),
            }))
            .unwrap();
        add(&store, id, 1, true, "Decisions remain in the thread".into()).unwrap();
        let current = store.thread(id).unwrap();
        assert_eq!(current.revision, 2);
        assert!(
            store
                .apply(ThreadCommand::Edit {
                    thread: id,
                    expected_revision: 1,
                    draft: ThreadDraft::default()
                })
                .unwrap_err()
                .contains("changed")
        );
        drop(store);
        let store = ThreadStore::new(root.clone());
        assert_eq!(store.thread(id), Some(current));
        assert_eq!(store.thread(ThreadId(1)).unwrap().context.len(), 0);
        assert!(
            !root
                .join(format!("threads/{}/conversation.json", id.0))
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_preserves_legacy_ids_and_does_not_import_preview_materials_or_probe() {
        let root = root();
        for id in [1, 3, 9001] {
            fs::create_dir_all(root.join(format!("threads/{id}"))).unwrap();
            fs::write(
                root.join(format!("threads/{id}/conversation.json")),
                "untouched history",
            )
            .unwrap();
        }
        let store = ThreadStore::new(root.clone());
        let threads = store.snapshot().threads;
        assert_eq!(
            threads.iter().map(|p| p.id).collect::<Vec<_>>(),
            [ThreadId(1), ThreadId(3)]
        );
        assert!(
            threads
                .iter()
                .all(|p| p.context.is_empty() && p.instructions.is_empty())
        );
        let id = store
            .apply(ThreadCommand::Create(ThreadDraft {
                name: "New".into(),
                ..Default::default()
            }))
            .unwrap();
        assert!(id.0 > 9001);
        assert_eq!(
            fs::read_to_string(root.join("threads/1/conversation.json")).unwrap(),
            "untouched history"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_or_future_catalog_is_never_overwritten() {
        for content in [
            "broken json",
            r#"{"version":99,"next_thread_id":2,"threads":[]}"#,
        ] {
            let root = root();
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("threads.json"), content).unwrap();
            let store = ThreadStore::new(root.clone());
            assert!(store.snapshot().error.is_some());
            assert!(
                store
                    .apply(ThreadCommand::Create(ThreadDraft {
                        name: "New".into(),
                        ..Default::default()
                    }))
                    .is_err()
            );
            assert_eq!(
                fs::read_to_string(root.join("threads.json")).unwrap(),
                content
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn write_failure_does_not_publish_unsaved_context() {
        let root = root();
        let store = ThreadStore::new(root.clone());
        let initial = store.thread(ThreadId(1)).unwrap();
        fs::rename(root.join("threads.json"), root.join("original.json")).unwrap();
        fs::create_dir(root.join("threads.json")).unwrap();
        assert!(add(&store, ThreadId(1), 1, true, "must not leak".into()).is_err());
        assert_eq!(store.thread(ThreadId(1)).unwrap(), initial);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn limits_count_unicode_and_selection_without_silently_truncating() {
        let root = root();
        let store = ThreadStore::new(root.clone());
        let id = ThreadId(1);
        assert!(add(&store, id, 1, true, "中".repeat(MAX_ITEM_CHARS + 1)).is_err());
        add(&store, id, 1, true, "中".repeat(MAX_ITEM_CHARS)).unwrap();
        add(&store, id, 2, true, "文".repeat(MAX_ITEM_CHARS)).unwrap();
        assert!(add(&store, id, 3, true, "字".repeat(MAX_ITEM_CHARS)).is_err());
        add(&store, id, 3, false, "字".repeat(MAX_ITEM_CHARS)).unwrap();
        assert_eq!(
            store.thread(id).unwrap().context[0].content.chars().count(),
            MAX_ITEM_CHARS
        );
        let first = store.thread(id).unwrap().context[0].id;
        store
            .apply(ThreadCommand::RemoveContext {
                thread: id,
                expected_revision: 4,
                id: first,
            })
            .unwrap();
        add(&store, id, 5, true, "Replacement".into()).unwrap();
        assert!(store.thread(id).unwrap().context.last().unwrap().id > first);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn thread_memory_is_durable_isolated_and_keeps_its_source_on_edit() {
        let root = root();
        let store = ThreadStore::new(root.clone());
        let other = store
            .apply(ThreadCommand::Create(ThreadDraft {
                name: "Another thread".into(),
                ..Default::default()
            }))
            .unwrap();
        let save = |revision, id, content: &str, source| ThreadCommand::SaveMemory {
            thread: ThreadId(1),
            expected_revision: revision,
            id,
            kind: MemoryKind::Decision,
            name: "Storage decision".into(),
            content: content.into(),
            source,
        };
        let source = Some(MemorySource::Message { message_id: 12 });
        store
            .apply(save(1, None, "Use local JSON", source.clone()))
            .unwrap();
        let memory = store.thread(ThreadId(1)).unwrap().memory[0].clone();
        assert!(store.thread(other).unwrap().memory.is_empty());
        assert!(
            store
                .apply(save(1, Some(memory.id), "Stale edit", None))
                .is_err()
        );
        store
            .apply(save(2, Some(memory.id), "Use versioned local JSON", None))
            .unwrap();
        let expected = store.thread(ThreadId(1)).unwrap();
        assert_eq!(expected.memory[0].source, source);
        drop(store);
        let restored = ThreadStore::new(root.clone());
        assert_eq!(restored.thread(ThreadId(1)), Some(expected));
        restored
            .apply(ThreadCommand::RemoveMemory {
                thread: ThreadId(1),
                expected_revision: 3,
                id: memory.id,
            })
            .unwrap();
        restored
            .apply(save(4, None, "Replacement decision", None))
            .unwrap();
        assert!(restored.thread(ThreadId(1)).unwrap().memory[0].id > memory.id);
        assert!(restored.thread(other).unwrap().memory.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_catalog_loads_unchanged_and_upgrades_only_when_memory_is_saved() {
        let root = root();
        drop(ThreadStore::new(root.clone()));
        let path = root.join("threads.json");
        let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        old["version"] = 1.into();
        for thread in old["threads"].as_array_mut().unwrap() {
            thread.as_object_mut().unwrap().remove("memory");
            thread.as_object_mut().unwrap().remove("next_memory_id");
        }
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&path, &bytes).unwrap();
        let store = ThreadStore::new(root.clone());
        assert!(store.thread(ThreadId(1)).unwrap().memory.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        store
            .apply(ThreadCommand::SaveMemory {
                thread: ThreadId(1),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Fact,
                name: "Thread fact".into(),
                content: "Memory lives in the thread".into(),
                source: None,
            })
            .unwrap();
        let current: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current["version"], 2);
        assert_eq!(current["threads"][0]["memory"][0]["id"], 1);
        // An unrecognized memory kind cannot be silently discarded on the next write.
        let mut corrupt = current;
        corrupt["threads"][0]["memory"][0]["kind"] = "future_kind".into();
        let bytes = serde_json::to_vec(&corrupt).unwrap();
        fs::write(&path, &bytes).unwrap();
        let unreadable = ThreadStore::new(root.clone());
        assert!(unreadable.snapshot().error.is_some());
        assert!(
            unreadable
                .apply(ThreadCommand::Create(ThreadDraft {
                    name: "Must not overwrite".into(),
                    ..Default::default()
                }))
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memory_limits_and_failed_writes_do_not_publish_or_truncate_content() {
        let root = root();
        let store = ThreadStore::new(root.clone());
        let save = |content: String| ThreadCommand::SaveMemory {
            thread: ThreadId(1),
            expected_revision: 1,
            id: None,
            kind: MemoryKind::Fact,
            name: "Unicode fact".into(),
            content,
            source: None,
        };
        assert!(
            store
                .apply(save("中".repeat(MAX_MEMORY_CHARS + 1)))
                .is_err()
        );
        assert_eq!(store.thread(ThreadId(1)).unwrap().revision, 1);
        store.apply(save("中".repeat(MAX_MEMORY_CHARS))).unwrap();
        assert_eq!(
            store.thread(ThreadId(1)).unwrap().memory[0]
                .content
                .chars()
                .count(),
            MAX_MEMORY_CHARS
        );
        add(&store, ThreadId(1), 2, true, "a".repeat(MAX_ITEM_CHARS)).unwrap();
        add(&store, ThreadId(1), 3, true, "b".repeat(MAX_ITEM_CHARS)).unwrap();
        let initial = store.thread(ThreadId(1)).unwrap();
        // Selected notes and memory share a budget; neither is silently deselected.
        assert!(add(&store, ThreadId(1), 4, true, "c".repeat(4_000)).is_err());
        assert_eq!(store.thread(initial.id), Some(initial.clone()));
        fs::rename(root.join("threads.json"), root.join("original.json")).unwrap();
        fs::create_dir(root.join("threads.json")).unwrap();
        assert!(
            store
                .apply(ThreadCommand::RemoveMemory {
                    thread: initial.id,
                    expected_revision: initial.revision,
                    id: initial.memory[0].id,
                })
                .is_err()
        );
        assert_eq!(store.thread(initial.id), Some(initial));
        fs::remove_dir_all(root).unwrap();
    }
}
