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
struct SavedDefinition {
    id: u64,
    revision: u64,
    name: String,
    description: String,
    instructions: String,
    next_context_id: u64,
    context: Vec<SavedItem>,
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
            included <= MAX_SELECTED_CONTEXT_CHARS,
            "Selected context is limited to {MAX_SELECTED_CONTEXT_CHARS} characters. Deselect some notes before adding more."
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
            self.version == 1,
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
            version: 1,
            next_project_id: next_id,
            projects,
        };
        saved.validate()?;
        super::store::write_json(&root.join("projects.json"), &saved)?;
        Ok(saved)
    }

    fn transaction(&self, command: ProjectCommand) -> Result<ProjectId> {
        let _writer = self.writer.lock().expect("project writer lock");
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
                } => (*project, *expected_revision),
                ProjectCommand::Create(_) => unreachable!(),
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
                ProjectCommand::Create(_) => unreachable!(),
            }
            project.revision = project
                .revision
                .checked_add(1)
                .context("Project revision exhausted")?;
            id
        };
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
}
