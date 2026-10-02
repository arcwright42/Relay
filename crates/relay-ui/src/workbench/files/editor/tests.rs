use super::FileEditor;
use gpui_kit::TestAppContext;
use relay_core::{files::*, settings::Language};
use std::sync::Arc;

fn document(version: &str, text: &str) -> FileDocument {
    FileDocument {
        location: FileLocation {
            workspace: "/personal/files".into(),
            path: "result.md".into(),
        },
        stamp: FileStamp {
            bytes: text.len() as u64,
            modified: None,
        },
        version: version.into(),
        content: FileContent::Text {
            text: text.into(),
            markdown: true,
        },
    }
}

#[gpui_kit::test]
fn an_agent_edit_retains_the_users_draft_and_exposes_the_disk_conflict(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        FileEditor::new(
            document("v1", "Original"),
            Arc::new(EmptyFiles),
            Language::English,
            window,
            cx,
        )
    });
    window
        .update(cx, |editor, window, cx| {
            editor
                .body
                .update(cx, |body, cx| body.set_value("My unsaved edit", window, cx));
            editor.sync_disk(document("v2", "Agent's new content"), window, cx);
            assert_eq!(editor.body.read(cx).value().as_ref(), "My unsaved edit");
            assert_eq!(editor.document.version, "v1");
            assert_eq!(editor.external.as_ref().unwrap().version, "v2");
            assert!(editor.dirty(cx));
        })
        .unwrap();
}

#[gpui_kit::test]
fn clean_previews_follow_agent_changes_and_matching_drafts_become_clean(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        FileEditor::new(
            document("v1", "Original"),
            Arc::new(EmptyFiles),
            Language::English,
            window,
            cx,
        )
    });
    window
        .update(cx, |editor, window, cx| {
            editor.sync_disk(document("v2", "New Agent content"), window, cx);
            assert_eq!(editor.body.read(cx).value().as_ref(), "New Agent content");
            assert!(!editor.dirty(cx));
            editor
                .body
                .update(cx, |body, cx| body.set_value("Same content", window, cx));
            editor.sync_disk(document("v3", "Same content"), window, cx);
            assert_eq!(editor.document.version, "v3");
            assert!(!editor.dirty(cx));
            assert!(editor.external.is_none());
        })
        .unwrap();
}

#[gpui_kit::test]
fn a_failed_save_retains_the_edit_instead_of_clearing_it(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        FileEditor::new(
            document("v1", "Original"),
            Arc::new(EmptyFiles),
            Language::English,
            window,
            cx,
        )
    });
    window
        .update(cx, |editor, window, cx| {
            editor
                .body
                .update(cx, |body, cx| body.set_value("Unsent edit\r\n", window, cx));
            editor.save(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |editor, _, cx| {
            assert!(editor.dirty(cx));
            assert_eq!(editor.body.read(cx).value().as_ref(), "Unsent edit\r\n");
            assert_eq!(editor.error, Some(FileError::Unavailable));
            assert!(!editor.saving);
        })
        .unwrap();
}
