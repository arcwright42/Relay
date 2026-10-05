use super::*;

struct Fixture {
    root: PathBuf,
    files: FileStore,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("relay-files-{}", crate::installer::unique_id()));
        fs::create_dir_all(&root).unwrap();
        let files = FileStore::new(root.clone());
        Self { root, files }
    }
    fn location(&self, path: &str) -> FileLocation {
        FileLocation {
            workspace: self.files.list(PathBuf::new()).unwrap().workspace,
            path: path.into(),
        }
    }
    fn create(&self, path: &str, content: &str) -> FileDocument {
        let location = self.location(path);
        self.files
            .apply(FileCommand::Create {
                location: location.clone(),
                directory: false,
                content: content.into(),
            })
            .unwrap();
        self.files.read(location).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn text(document: &FileDocument) -> &str {
    let FileContent::Text { text, .. } = &document.content else {
        panic!("expected text")
    };
    text
}

#[test]
fn personal_files_survive_restart_without_any_thread_or_agent_session() {
    let fixture = Fixture::new();
    let doc = fixture.create("notes.md", "# 私人文件\r\n保留换行\r\n");
    assert!(!fixture.root.join("threads.json").exists());
    assert_eq!(fixture.files.list(PathBuf::new()).unwrap().entries.len(), 1);
    fixture
        .files
        .apply(FileCommand::Save {
            location: doc.location.clone(),
            expected_version: doc.version,
            content: "# 已编辑\r\n正文\r\n".into(),
        })
        .unwrap();
    let restarted = FileStore::new(fixture.root.clone());
    assert_eq!(
        text(&restarted.read(doc.location).unwrap()),
        "# 已编辑\r\n正文\r\n"
    );
}

#[test]
fn an_agent_write_is_visible_and_stale_editor_save_preserves_the_agent_version() {
    let fixture = Fixture::new();
    let doc = fixture.create("result.md", "Old result");
    fs::write(doc.location.absolute_path(), "New Agent result").unwrap();
    let listing = fixture.files.list(PathBuf::new()).unwrap();
    assert_eq!(listing.entries[0].name, "result.md");
    assert_eq!(
        text(&fixture.files.read(doc.location.clone()).unwrap()),
        "New Agent result"
    );
    assert_eq!(
        fixture
            .files
            .apply(FileCommand::Save {
                location: doc.location.clone(),
                expected_version: doc.version,
                content: "My old draft".into()
            })
            .unwrap_err(),
        FileError::Conflict
    );
    assert_eq!(
        fs::read_to_string(doc.location.absolute_path()).unwrap(),
        "New Agent result"
    );
    assert!(!fs::read_dir(&doc.location.workspace).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".relay-save-")
    }));
}

#[test]
fn imports_and_new_files_do_not_overwrite_duplicate_names() {
    let fixture = Fixture::new();
    let doc = fixture.create("reference.txt", "Original");
    let source = fixture.root.join("source.txt");
    fs::write(&source, "Imported").unwrap();
    assert_eq!(
        fixture
            .files
            .apply(FileCommand::Import {
                location: doc.location.clone(),
                source: source.clone()
            })
            .unwrap_err(),
        FileError::AlreadyExists
    );
    assert_eq!(
        fixture
            .files
            .apply(FileCommand::Create {
                location: doc.location.clone(),
                directory: false,
                content: "Replacement".into()
            })
            .unwrap_err(),
        FileError::AlreadyExists
    );
    let imported = fixture.location("imported.txt");
    fixture
        .files
        .apply(FileCommand::Import {
            location: imported.clone(),
            source,
        })
        .unwrap();
    assert_eq!(text(&fixture.files.read(imported).unwrap()), "Imported");
    assert_eq!(text(&fixture.files.read(doc.location).unwrap()), "Original");
}

#[test]
fn directories_are_first_and_files_generated_in_subfolders_are_browsable() {
    let fixture = Fixture::new();
    fixture.create("a.txt", "a");
    let directory = fixture.location("reports");
    fixture
        .files
        .apply(FileCommand::Create {
            location: directory.clone(),
            directory: true,
            content: String::new(),
        })
        .unwrap();
    fs::write(
        directory.absolute_path().join("agent.md"),
        "# Agent deliverable",
    )
    .unwrap();
    assert_eq!(
        fixture.files.list(PathBuf::new()).unwrap().entries[0].kind,
        FileKind::Directory
    );
    let listing = fixture.files.list("reports".into()).unwrap();
    assert_eq!(listing.entries[0].path, PathBuf::from("reports/agent.md"));
}

#[test]
fn a_forged_file_location_cannot_redirect_an_edit_to_another_directory() {
    let fixture = Fixture::new();
    let doc = fixture.create("result.md", "Personal file");
    let next = fixture.root.join("other-workspace");
    fs::create_dir(&next).unwrap();
    fs::write(next.join("result.md"), "Other file").unwrap();
    let wrong_location = FileLocation {
        workspace: next.clone(),
        ..doc.location.clone()
    };
    assert_eq!(
        fixture
            .files
            .apply(FileCommand::Save {
                location: wrong_location,
                expected_version: doc.version,
                content: "Draft".into(),
            })
            .unwrap_err(),
        FileError::WorkspaceChanged
    );
    assert_eq!(
        fs::read_to_string(doc.location.absolute_path()).unwrap(),
        "Personal file"
    );
    assert_eq!(
        fs::read_to_string(next.join("result.md")).unwrap(),
        "Other file"
    );
}

#[test]
fn traversal_and_symlinks_cannot_read_or_write_outside_the_personal_space() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside.txt");
    fs::write(&outside, "Private").unwrap();
    let location = fixture.location("../outside.txt");
    assert_eq!(
        fixture.files.read(location).unwrap_err(),
        FileError::InvalidPath
    );
    #[cfg(unix)]
    {
        let link = fixture.location("linked.txt");
        std::os::unix::fs::symlink(&outside, link.absolute_path()).unwrap();
        assert_eq!(
            fixture.files.read(link.clone()).unwrap_err(),
            FileError::OutsideWorkspace
        );
        assert_eq!(
            fixture
                .files
                .apply(FileCommand::Create {
                    location: link,
                    directory: false,
                    content: "Oops".into()
                })
                .unwrap_err(),
            FileError::OutsideWorkspace
        );
        assert!(
            fixture
                .files
                .list(PathBuf::new())
                .unwrap()
                .entries
                .is_empty()
        );
    }
    assert_eq!(fs::read_to_string(outside).unwrap(), "Private");
}

#[test]
fn markdown_links_support_unicode_spaces_line_numbers_and_safe_relative_paths() {
    let fixture = Fixture::new();
    let doc = fixture.create("结果 报告.md", "# Report");
    for link in [
        doc.location.absolute_path().to_string_lossy().into_owned(),
        format!("file://{}:12:3", doc.location.absolute_path().display()),
        "./结果%20报告.md#section".into(),
    ] {
        assert_eq!(fixture.files.resolve_link(&link).unwrap(), doc.location);
    }
    assert_eq!(
        fixture.files.resolve_link("../outside.txt").unwrap_err(),
        FileError::OutsideWorkspace
    );
}

#[test]
fn binary_or_large_files_are_not_silently_decoded_and_saved_as_text() {
    let fixture = Fixture::new();
    let binary = fixture.location("data.bin");
    fs::write(binary.absolute_path(), [0, 0xff, 0x01]).unwrap();
    let doc = fixture.files.read(binary.clone()).unwrap();
    assert!(matches!(doc.content, FileContent::Binary { .. }));
    assert_eq!(
        fixture
            .files
            .apply(FileCommand::Save {
                location: binary.clone(),
                expected_version: doc.version,
                content: "Text".into()
            })
            .unwrap_err(),
        FileError::NotText
    );
    let large = fixture.location("large.txt");
    fs::write(
        large.absolute_path(),
        vec![b'x'; MAX_EDIT_BYTES as usize + 1],
    )
    .unwrap();
    assert!(matches!(
        fixture.files.read(large).unwrap().content,
        FileContent::Binary { .. }
    ));
    assert_eq!(fs::read(binary.absolute_path()).unwrap(), [0, 0xff, 0x01]);
}

#[test]
fn all_conversations_share_the_personal_space_and_custom_workfolders_do_not_change_it() {
    use relay_core::{
        agents::{AgentCommand, AgentService},
        threads::{ThreadCommand, ThreadDraft, ThreadService},
    };
    use std::sync::Arc;
    let fixture = Fixture::new();
    let doc = fixture.create("shared.md", "Shared deliverable");
    let threads = Arc::new(crate::ThreadStore::new(fixture.root.clone()));
    let first = threads
        .apply(ThreadCommand::Create(ThreadDraft {
            name: "First conversation".into(),
            ..Default::default()
        }))
        .unwrap();
    let second = threads
        .apply(ThreadCommand::Create(ThreadDraft {
            name: "Second conversation".into(),
            ..Default::default()
        }))
        .unwrap();
    let ids: Vec<_> = threads
        .snapshot()
        .threads
        .iter()
        .map(|thread| thread.id)
        .collect();
    let agents = crate::AgentRuntime::new(fixture.root.clone(), threads);
    for id in ids {
        assert_eq!(
            agents.snapshot(id).working_directory,
            doc.location.workspace
        );
    }
    let custom = fixture.root.join("code-repository");
    fs::create_dir(&custom).unwrap();
    let custom = custom.canonicalize().unwrap();
    agents
        .dispatch(first, AgentCommand::SetWorkingDirectory(custom.clone()))
        .unwrap();
    assert_eq!(agents.snapshot(first).working_directory, custom);
    assert_eq!(
        fixture.files.list(PathBuf::new()).unwrap().workspace,
        doc.location.workspace
    );
    fs::write(
        agents
            .snapshot(second)
            .working_directory
            .join("analysis.md"),
        "Another conversation",
    )
    .unwrap();
    assert_eq!(fixture.files.list(PathBuf::new()).unwrap().entries.len(), 2);
    assert_eq!(
        text(&fixture.files.read(doc.location).unwrap()),
        "Shared deliverable"
    );
    agents.shutdown();
}

#[test]
fn saved_legacy_default_folders_upgrade_while_user_selected_folders_are_preserved() {
    use relay_core::{
        agents::{AgentCommand, AgentService},
        threads::{ThreadCommand, ThreadDraft, ThreadService},
    };
    use std::sync::Arc;
    let fixture = Fixture::new();
    let threads = Arc::new(crate::ThreadStore::new(fixture.root.clone()));
    let first = threads.snapshot().threads[0].id;
    let legacy = fixture.root.join(format!("projects/{}/workspace", first.0));
    let custom = fixture.root.join("custom-repository");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir(&custom).unwrap();
    fs::write(legacy.join("old.md"), "An existing deliverable").unwrap();
    let second = threads
        .apply(ThreadCommand::Create(ThreadDraft {
            name: "Custom workfolder".into(),
            ..Default::default()
        }))
        .unwrap();
    let agents = crate::AgentRuntime::new(fixture.root.clone(), threads.clone());
    agents
        .dispatch(first, AgentCommand::SetWorkingDirectory(legacy.clone()))
        .unwrap();
    agents
        .dispatch(second, AgentCommand::SetWorkingDirectory(custom.clone()))
        .unwrap();
    agents.shutdown();
    let listing = fixture.files.list(PathBuf::new()).unwrap();
    let restarted = crate::AgentRuntime::new(fixture.root.clone(), threads);
    assert_eq!(
        restarted.snapshot(first).working_directory,
        listing.workspace
    );
    assert_eq!(
        restarted.snapshot(second).working_directory,
        custom.canonicalize().unwrap()
    );
    assert_eq!(listing.entries[0].name, "old.md");
    assert_eq!(
        fs::read_to_string(legacy.join("old.md")).unwrap(),
        "An existing deliverable"
    );
    restarted.shutdown();
}

#[test]
fn legacy_files_merge_without_thread_folders_overwrites_or_duplicate_reimports() {
    let fixture = Fixture::new();
    let first = fixture.root.join("projects/1/workspace/reports");
    let second = fixture.root.join("projects/2/workspace/reports");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("result.md"), "First report").unwrap();
    fs::write(second.join("result.md"), "Second report").unwrap();
    fs::write(first.join("same.md"), "Shared copy").unwrap();
    fs::write(second.join("same.md"), "Shared copy").unwrap();
    let personal = fixture.root.join("files/reports");
    fs::create_dir_all(&personal).unwrap();
    fs::write(personal.join("result.md"), "Personal original").unwrap();
    let listing = fixture.files.list("reports".into()).unwrap();
    assert_eq!(listing.entries.len(), 4);
    assert_eq!(
        fs::read_to_string(personal.join("result.md")).unwrap(),
        "Personal original"
    );
    assert_eq!(
        fs::read_to_string(personal.join("result (2).md")).unwrap(),
        "First report"
    );
    assert_eq!(
        fs::read_to_string(personal.join("result (3).md")).unwrap(),
        "Second report"
    );
    assert_eq!(
        fs::read_to_string(second.join("result.md")).unwrap(),
        "Second report"
    );
    fs::write(personal.join("same.md"), "User's new edit").unwrap();
    let restarted = FileStore::new(fixture.root.clone());
    assert_eq!(restarted.list("reports".into()).unwrap().entries.len(), 4);
    assert_eq!(
        fs::read_to_string(personal.join("same.md")).unwrap(),
        "User's new edit"
    );
}
