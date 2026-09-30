use anyhow::{Context, Result, bail, ensure};
use relay_core::{ProjectId, projects::*};
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
    fn new(id: u64, draft: ProjectDraft) -> Self {
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

    fn project(&self) -> Project {
        Project {
            id: ProjectId(self.id),
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
            "Invalid project identifier or revision."
        );
        validate_name(&self.name)?;
        ensure!(
            self.description.chars().count() <= 1_000,
            "Description is limited to 1,000 characters."
        );
        ensure!(
            self.instructions.chars().count() <= MAX_INSTRUCTIONS_CHARS,
            "Project instructions are limited to {MAX_INSTRUCTIONS_CHARS} characters."
        );
        ensure!(
            self.context.len() <= MAX_CONTEXT_ITEMS,
            "A project can contain up to {MAX_CONTEXT_ITEMS} notes."
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
            "A project can contain up to {MAX_MEMORY_ITEMS} memories."
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
            "Selected context and project memory are limited to {MAX_SELECTED_CONTEXT_CHARS} characters. Reduce memory or deselect some notes before adding more."
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
    next_project_id: u64,
    projects: Vec<SavedDefinition>,
}

impl SavedCatalog {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.version, 1..=3),
            "Unsupported project catalog version {}. The file has been preserved.",
            self.version
        );
        let mut ids = BTreeSet::new();
        ensure!(
            !self.projects.is_empty(),
            "Project catalog cannot be empty."
        );
        for project in &self.projects {
            ensure!(
                project.id < self.next_project_id && ids.insert(project.id),
                "Invalid project identifier."
            );
            project.validate()?;
            ensure!(
                self.version >= 3 || project.memory.iter().all(|m| m.client_source.is_none()),
                "Native memory sources require project catalog v3."
            );
        }
        Ok(())
    }
}

struct State {
    saved: Option<SavedCatalog>,
    view: ProjectCatalog,
}

/// Disk writes are serialized, but never hold the read lock during I/O.
/// Call `apply` on a background worker; readers see only successfully saved revisions.
pub struct ProjectStore {
    root: PathBuf,
    state: Mutex<State>,
    writer: Mutex<()>,
}

impl ProjectStore {
    pub fn new(root: PathBuf) -> Self {
        let loaded = Self::load_or_migrate(&root);
        let view = match &loaded {
            Ok(saved) => ProjectCatalog {
                revision: 1,
                projects: saved
                    .projects
                    .iter()
                    .map(SavedDefinition::project)
                    .collect(),
                error: None,
            },
            Err(error) => ProjectCatalog {
                revision: 1,
                projects: vec![],
                error: Some(format!(
                    "Could not read projects: {error:#}. Existing files have been preserved."
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
        match fs::read(root.join("projects.json")) {
            Ok(bytes) => {
                let saved: SavedCatalog =
                    serde_json::from_slice(&bytes).context("Reading project catalog")?;
                saved.validate()?;
                return Ok(saved);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut projects = Vec::new();
        // These are the only product IDs in the preview version. Probe ID 9001 is not a user project.
        for (id, name) in [(1, "Product Design"), (2, "Agent Infra"), (3, "Personal")] {
            if root
                .join(format!("projects/{id}/conversation.json"))
                .try_exists()?
            {
                projects.push(SavedDefinition::new(
                    id,
                    ProjectDraft {
                        name: name.into(),
                        ..Default::default()
                    },
                ));
            }
        }
        let mut next_id = 1;
        // Reserve every existing directory, including probe/unknown IDs, so none can be overwritten.
        match fs::read_dir(root.join("projects")) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(id) = entry?.file_name().to_string_lossy().parse::<u64>() {
                        next_id =
                            next_id.max(id.checked_add(1).context("Project identifier exhausted")?);
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if projects.is_empty() {
            projects.push(SavedDefinition::new(
                next_id,
                ProjectDraft {
                    name: "My project".into(),
                    ..Default::default()
                },
            ));
            next_id = next_id
                .checked_add(1)
                .context("Project identifier exhausted")?;
        }
        let saved = SavedCatalog {
            version: 2,
            next_project_id: next_id,
            projects,
        };
        saved.validate()?;
        super::store::write_json(&root.join("projects.json"), &saved)?;
        Ok(saved)
    }

    fn transaction(&self, command: ProjectCommand) -> Result<ProjectId> {
        let _writer = self.writer.lock().expect("project writer lock");
        let command = match command {
            ProjectCommand::CreateAtRevision {
                expected_catalog_revision,
                draft,
            } => {
                ensure!(
                    self.revision() == expected_catalog_revision,
                    "The project list changed. Review the destination and try again."
                );
                ProjectCommand::Create(draft)
            }
            command => command,
        };
        let mut saved = self
            .state
            .lock()
            .expect("project catalog lock")
            .saved
            .clone()
            .context(
                "Resolve the project catalog error and restart Relay before editing projects",
            )?;
        let id = if let ProjectCommand::Create(draft) = command {
            ensure!(
                saved.projects.len() < 128,
                "A workspace can contain up to 128 projects."
            );
            let mut id = saved.next_project_id;
            while self.root.join(format!("projects/{id}")).try_exists()? {
                id = id.checked_add(1).context("Project identifier exhausted")?;
            }
            saved.next_project_id = id.checked_add(1).context("Project identifier exhausted")?;
            saved.projects.push(SavedDefinition::new(id, draft));
            ProjectId(id)
        } else {
            let (id, expected) = match &command {
                ProjectCommand::Edit {
                    project,
                    expected_revision,
                    ..
                }
                | ProjectCommand::SaveContext {
                    project,
                    expected_revision,
                    ..
                }
                | ProjectCommand::RemoveContext {
                    project,
                    expected_revision,
                    ..
                }
                | ProjectCommand::SaveMemory {
                    project,
                    expected_revision,
                    ..
                }
                | ProjectCommand::RemoveMemory {
                    project,
                    expected_revision,
                    ..
                } => (*project, *expected_revision),
                ProjectCommand::Create(_) | ProjectCommand::CreateAtRevision { .. } => {
                    unreachable!()
                }
            };
            let project = saved
                .projects
                .iter_mut()
                .find(|p| p.id == id.0)
                .context("Unknown project")?;
            ensure!(
                project.revision == expected,
                "This project changed while you were editing. Reopen the editor and try again."
            );
            match command {
                ProjectCommand::Edit { draft, .. } => {
                    project.name = draft.name.trim().into();
                    project.description = draft.description;
                    project.instructions = draft.instructions;
                }
                ProjectCommand::SaveContext {
                    id,
                    name,
                    content,
                    included,
                    ..
                } => {
                    let item = if let Some(id) = id {
                        project
                            .context
                            .iter_mut()
                            .find(|item| item.id == id.0)
                            .context("Unknown note")?
                    } else {
                        let id = project.next_context_id;
                        project.next_context_id =
                            id.checked_add(1).context("Context identifier exhausted")?;
                        project.context.push(SavedItem {
                            id,
                            name: String::new(),
                            content: String::new(),
                            included,
                        });
                        project.context.last_mut().expect("new note")
                    };
                    item.name = name.trim().into();
                    item.content = content;
                    item.included = included;
                }
                ProjectCommand::RemoveContext { id, .. } => {
                    let length = project.context.len();
                    project.context.retain(|item| item.id != id.0);
                    if length == project.context.len() {
                        bail!("Unknown note");
                    }
                }
                ProjectCommand::SaveMemory {
                    id,
                    kind,
                    name,
                    content,
                    source,
                    ..
                } => {
                    let item = if let Some(id) = id {
                        project
                            .memory
                            .iter_mut()
                            .find(|item| item.id == id.0)
                            .context("Unknown memory")?
                    } else {
                        let id = project.next_memory_id;
                        project.next_memory_id =
                            id.checked_add(1).context("Memory identifier exhausted")?;
                        project.memory.push(SavedMemory {
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
                        project.memory.last_mut().expect("new memory")
                    };
                    item.kind = kind.code().into();
                    item.name = name.trim().into();
                    item.content = content;
                }
                ProjectCommand::RemoveMemory { id, .. } => {
                    let length = project.memory.len();
                    project.memory.retain(|item| item.id != id.0);
                    ensure!(length != project.memory.len(), "Unknown memory");
                }
                ProjectCommand::Create(_) | ProjectCommand::CreateAtRevision { .. } => {
                    unreachable!()
                }
            }
            project.revision = project
                .revision
                .checked_add(1)
                .context("Project revision exhausted")?;
            id
        };
        // Older clients must reject native provenance instead of silently discarding it.
        saved.version = saved.version.max(
            if saved
                .projects
                .iter()
                .any(|p| p.memory.iter().any(|m| m.client_source.is_some()))
            {
                3
            } else {
                2
            },
        );
        saved.validate()?;
        super::store::write_json(&self.root.join("projects.json"), &saved)?;
        let mut state = self.state.lock().expect("project catalog lock");
        state.view.projects = saved
            .projects
            .iter()
            .map(SavedDefinition::project)
            .collect();
        state.view.revision += 1;
        state.saved = Some(saved);
        Ok(id)
    }
}

impl ProjectService for ProjectStore {
    fn revision(&self) -> u64 {
        self.state
            .lock()
            .expect("project catalog lock")
            .view
            .revision
    }
    fn project(&self, id: ProjectId) -> Option<Project> {
        self.state
            .lock()
            .expect("project catalog lock")
            .saved
            .as_ref()?
            .projects
            .iter()
            .find(|p| p.id == id.0)
            .map(SavedDefinition::project)
    }
    fn snapshot(&self) -> ProjectCatalog {
        self.state
            .lock()
            .expect("project catalog lock")
            .view
            .clone()
    }
    fn apply(&self, command: ProjectCommand) -> Result<ProjectId, String> {
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
        let store = ProjectStore::new(root.clone());
        let source = MemorySource::ClientSession {
            client: "codex".into(),
            session_id: "native-session".into(),
            message_id: 123,
        };
        store
            .apply(ProjectCommand::SaveMemory {
                project: ProjectId(1),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Decision,
                name: "Use local clients".into(),
                content: "Keep the native harness and its configuration".into(),
                source: Some(source.clone()),
            })
            .unwrap();
        let memory = store.project(ProjectId(1)).unwrap().memory[0].clone();
        store
            .apply(ProjectCommand::SaveMemory {
                project: ProjectId(1),
                expected_revision: 2,
                id: Some(memory.id),
                kind: MemoryKind::Fact,
                name: memory.name,
                content: "Updated reviewed fact".into(),
                source: None,
            })
            .unwrap();
        drop(store);
        let restored = ProjectStore::new(root.clone())
            .project(ProjectId(1))
            .unwrap();
        assert_eq!(restored.memory[0].source, Some(source));
        let context = crate::context::Snapshot::from_project(&restored)
            .delivery(None)
            .text
            .unwrap();
        assert!(context.contains("\"client_source\":{\"client\":\"codex\""));
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("projects.json")).unwrap()).unwrap();
        assert_eq!(saved["version"], 3);
        fs::remove_dir_all(root).unwrap();
    }
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("relay-projects-{}", crate::installer::unique_id()))
    }
    fn add(
        store: &ProjectStore,
        id: ProjectId,
        revision: u64,
        included: bool,
        content: String,
    ) -> Result<ProjectId, String> {
        store.apply(ProjectCommand::SaveContext {
            project: id,
            expected_revision: revision,
            id: None,
            name: "Reference".into(),
            content,
            included,
        })
    }

    #[test]
    fn project_data_survives_restart_without_any_agent_session() {
        let root = root();
        let store = ProjectStore::new(root.clone());
        let id = store
            .apply(ProjectCommand::Create(ProjectDraft {
                name: "用户项目".into(),
                instructions: "Rust only".into(),
                description: "Local work".into(),
            }))
            .unwrap();
        add(
            &store,
            id,
            1,
            true,
            "Decisions remain in the project".into(),
        )
        .unwrap();
        let current = store.project(id).unwrap();
        assert_eq!(current.revision, 2);
        assert!(
            store
                .apply(ProjectCommand::Edit {
                    project: id,
                    expected_revision: 1,
                    draft: ProjectDraft::default()
                })
                .unwrap_err()
                .contains("changed")
        );
        drop(store);
        let store = ProjectStore::new(root.clone());
        assert_eq!(store.project(id), Some(current));
        assert_eq!(store.project(ProjectId(1)).unwrap().context.len(), 0);
        assert!(
            !root
                .join(format!("projects/{}/conversation.json", id.0))
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_preserves_legacy_ids_and_does_not_import_preview_materials_or_probe() {
        let root = root();
        for id in [1, 3, 9001] {
            fs::create_dir_all(root.join(format!("projects/{id}"))).unwrap();
            fs::write(
                root.join(format!("projects/{id}/conversation.json")),
                "untouched history",
            )
            .unwrap();
        }
        let store = ProjectStore::new(root.clone());
        let projects = store.snapshot().projects;
        assert_eq!(
            projects.iter().map(|p| p.id).collect::<Vec<_>>(),
            [ProjectId(1), ProjectId(3)]
        );
        assert!(
            projects
                .iter()
                .all(|p| p.context.is_empty() && p.instructions.is_empty())
        );
        let id = store
            .apply(ProjectCommand::Create(ProjectDraft {
                name: "New".into(),
                ..Default::default()
            }))
            .unwrap();
        assert!(id.0 > 9001);
        assert_eq!(
            fs::read_to_string(root.join("projects/1/conversation.json")).unwrap(),
            "untouched history"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_or_future_catalog_is_never_overwritten() {
        for content in [
            "broken json",
            r#"{"version":99,"next_project_id":2,"projects":[]}"#,
        ] {
            let root = root();
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("projects.json"), content).unwrap();
            let store = ProjectStore::new(root.clone());
            assert!(store.snapshot().error.is_some());
            assert!(
                store
                    .apply(ProjectCommand::Create(ProjectDraft {
                        name: "New".into(),
                        ..Default::default()
                    }))
                    .is_err()
            );
            assert_eq!(
                fs::read_to_string(root.join("projects.json")).unwrap(),
                content
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn write_failure_does_not_publish_unsaved_context() {
        let root = root();
        let store = ProjectStore::new(root.clone());
        let initial = store.project(ProjectId(1)).unwrap();
        fs::rename(root.join("projects.json"), root.join("original.json")).unwrap();
        fs::create_dir(root.join("projects.json")).unwrap();
        assert!(add(&store, ProjectId(1), 1, true, "must not leak".into()).is_err());
        assert_eq!(store.project(ProjectId(1)).unwrap(), initial);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn limits_count_unicode_and_selection_without_silently_truncating() {
        let root = root();
        let store = ProjectStore::new(root.clone());
        let id = ProjectId(1);
        assert!(add(&store, id, 1, true, "中".repeat(MAX_ITEM_CHARS + 1)).is_err());
        add(&store, id, 1, true, "中".repeat(MAX_ITEM_CHARS)).unwrap();
        add(&store, id, 2, true, "文".repeat(MAX_ITEM_CHARS)).unwrap();
        assert!(add(&store, id, 3, true, "字".repeat(MAX_ITEM_CHARS)).is_err());
        add(&store, id, 3, false, "字".repeat(MAX_ITEM_CHARS)).unwrap();
        assert_eq!(
            store.project(id).unwrap().context[0]
                .content
                .chars()
                .count(),
            MAX_ITEM_CHARS
        );
        let first = store.project(id).unwrap().context[0].id;
        store
            .apply(ProjectCommand::RemoveContext {
                project: id,
                expected_revision: 4,
                id: first,
            })
            .unwrap();
        add(&store, id, 5, true, "Replacement".into()).unwrap();
        assert!(store.project(id).unwrap().context.last().unwrap().id > first);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_memory_is_durable_isolated_and_keeps_its_source_on_edit() {
        let root = root();
        let store = ProjectStore::new(root.clone());
        let other = store
            .apply(ProjectCommand::Create(ProjectDraft {
                name: "Another project".into(),
                ..Default::default()
            }))
            .unwrap();
        let save = |revision, id, content: &str, source| ProjectCommand::SaveMemory {
            project: ProjectId(1),
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
        let memory = store.project(ProjectId(1)).unwrap().memory[0].clone();
        assert!(store.project(other).unwrap().memory.is_empty());
        assert!(
            store
                .apply(save(1, Some(memory.id), "Stale edit", None))
                .is_err()
        );
        store
            .apply(save(2, Some(memory.id), "Use versioned local JSON", None))
            .unwrap();
        let expected = store.project(ProjectId(1)).unwrap();
        assert_eq!(expected.memory[0].source, source);
        drop(store);
        let restored = ProjectStore::new(root.clone());
        assert_eq!(restored.project(ProjectId(1)), Some(expected));
        restored
            .apply(ProjectCommand::RemoveMemory {
                project: ProjectId(1),
                expected_revision: 3,
                id: memory.id,
            })
            .unwrap();
        restored
            .apply(save(4, None, "Replacement decision", None))
            .unwrap();
        assert!(restored.project(ProjectId(1)).unwrap().memory[0].id > memory.id);
        assert!(restored.project(other).unwrap().memory.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_catalog_loads_unchanged_and_upgrades_only_when_memory_is_saved() {
        let root = root();
        drop(ProjectStore::new(root.clone()));
        let path = root.join("projects.json");
        let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        old["version"] = 1.into();
        for project in old["projects"].as_array_mut().unwrap() {
            project.as_object_mut().unwrap().remove("memory");
            project.as_object_mut().unwrap().remove("next_memory_id");
        }
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&path, &bytes).unwrap();
        let store = ProjectStore::new(root.clone());
        assert!(store.project(ProjectId(1)).unwrap().memory.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        store
            .apply(ProjectCommand::SaveMemory {
                project: ProjectId(1),
                expected_revision: 1,
                id: None,
                kind: MemoryKind::Fact,
                name: "Project fact".into(),
                content: "Memory lives in the project".into(),
                source: None,
            })
            .unwrap();
        let current: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current["version"], 2);
        assert_eq!(current["projects"][0]["memory"][0]["id"], 1);
        // An unrecognized memory kind cannot be silently discarded on the next write.
        let mut corrupt = current;
        corrupt["projects"][0]["memory"][0]["kind"] = "future_kind".into();
        let bytes = serde_json::to_vec(&corrupt).unwrap();
        fs::write(&path, &bytes).unwrap();
        let unreadable = ProjectStore::new(root.clone());
        assert!(unreadable.snapshot().error.is_some());
        assert!(
            unreadable
                .apply(ProjectCommand::Create(ProjectDraft {
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
        let store = ProjectStore::new(root.clone());
        let save = |content: String| ProjectCommand::SaveMemory {
            project: ProjectId(1),
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
        assert_eq!(store.project(ProjectId(1)).unwrap().revision, 1);
        store.apply(save("中".repeat(MAX_MEMORY_CHARS))).unwrap();
        assert_eq!(
            store.project(ProjectId(1)).unwrap().memory[0]
                .content
                .chars()
                .count(),
            MAX_MEMORY_CHARS
        );
        add(&store, ProjectId(1), 2, true, "a".repeat(MAX_ITEM_CHARS)).unwrap();
        add(&store, ProjectId(1), 3, true, "b".repeat(MAX_ITEM_CHARS)).unwrap();
        let initial = store.project(ProjectId(1)).unwrap();
        // Selected notes and memory share a budget; neither is silently deselected.
        assert!(add(&store, ProjectId(1), 4, true, "c".repeat(4_000)).is_err());
        assert_eq!(store.project(initial.id), Some(initial.clone()));
        fs::rename(root.join("projects.json"), root.join("original.json")).unwrap();
        fs::create_dir(root.join("projects.json")).unwrap();
        assert!(
            store
                .apply(ProjectCommand::RemoveMemory {
                    project: initial.id,
                    expected_revision: initial.revision,
                    id: initial.memory[0].id,
                })
                .is_err()
        );
        assert_eq!(store.project(initial.id), Some(initial));
        fs::remove_dir_all(root).unwrap();
    }
}
