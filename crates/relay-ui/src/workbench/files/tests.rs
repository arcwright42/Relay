use super::FilesView;
use gpui_kit::TestAppContext;
use relay_core::{files::*, settings::Language};
use std::{path::PathBuf, sync::Arc};

struct TestFiles;
impl FileService for TestFiles {
    fn list(&self, directory: PathBuf) -> Result<FileListing, FileError> {
        Ok(FileListing {
            workspace: PathBuf::from("/personal/files"),
            directory: directory.clone(),
            entries: vec![FileEntry {
                path: directory.join("result.md"),
                name: "result.md".into(),
                kind: FileKind::Markdown,
                stamp: FileStamp {
                    bytes: 6,
                    modified: None,
                },
            }],
            truncated: false,
        })
    }
    fn read(&self, location: FileLocation) -> Result<FileDocument, FileError> {
        Ok(FileDocument {
            location,
            stamp: FileStamp {
                bytes: 6,
                modified: None,
            },
            version: "v1".into(),
            content: FileContent::Text {
                text: "# File".into(),
                markdown: true,
            },
        })
    }
    fn apply(&self, _: FileCommand) -> Result<FileLocation, FileError> {
        Err(FileError::Conflict)
    }
    fn resolve_link(&self, _: &str) -> Result<FileLocation, FileError> {
        Ok(FileLocation {
            workspace: "/personal/files".into(),
            path: "result.md".into(),
        })
    }
}

#[gpui_kit::test]
fn changing_folders_while_loading_cannot_publish_an_old_listing(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        let mut view = FilesView::new(Language::SimplifiedChinese, window, cx);
        view.set_service(Arc::new(TestFiles));
        view.set_visible(true, window, cx);
        view.change_directory("reports".into(), window, cx);
        view
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(
                view.listing.as_ref().unwrap().directory,
                PathBuf::from("reports")
            );
            assert_eq!(
                view.listing.as_ref().unwrap().workspace,
                PathBuf::from("/personal/files")
            );
            assert_eq!(view.listing.as_ref().unwrap().entries[0].name, "result.md");
        })
        .unwrap();
}

#[gpui_kit::test]
fn a_file_link_opens_in_the_personal_space_without_a_project(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        let mut view = FilesView::new(Language::SimplifiedChinese, window, cx);
        view.set_service(Arc::new(TestFiles));
        view.open_link("/personal/files/result.md".into(), window, cx);
        view
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            let location = view.active.as_ref().unwrap();
            assert_eq!(location.workspace, PathBuf::from("/personal/files"));
            assert_eq!(location.path, PathBuf::from("result.md"));
            assert_eq!(
                view.editors
                    .get(location)
                    .unwrap()
                    .read(cx)
                    .document
                    .location,
                *location
            );
            assert!(view.error.is_none());
        })
        .unwrap();
}

#[gpui_kit::test]
fn rapid_file_selection_only_opens_the_most_recent_file(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        let mut view = FilesView::new(Language::English, window, cx);
        view.set_service(Arc::new(TestFiles));
        for path in ["first.md", "second.md"] {
            let location = FileLocation {
                workspace: "/personal/files".into(),
                path: path.into(),
            };
            view.active = Some(location.clone());
            view.open_file(location, window, cx);
        }
        view
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            let location = view.active.as_ref().unwrap();
            assert_eq!(location.path, PathBuf::from("second.md"));
            assert!(!view.opening);
            assert_eq!(
                view.editors
                    .get(location)
                    .unwrap()
                    .read(cx)
                    .document
                    .location,
                *location
            );
            assert_eq!(view.editors.len(), 1);
        })
        .unwrap();
}
