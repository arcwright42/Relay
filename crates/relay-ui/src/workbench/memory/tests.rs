use super::{Detail, MemoryView};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, Render, Styled, TestAppContext, Window,
    component::Root, div, prelude::ParentElement, test::TestWindowExt,
};
use relay_core::{memory::MemoryCommand, settings::Language};
use std::sync::Arc;
#[path = "../../../examples/fixtures/memory.rs"]
mod fixture;
use fixture::MemoryFixtures;

struct MemoryHarness(Entity<MemoryView>);
impl Render for MemoryHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(self.0.clone())
            .children(Root::render_dialog_layer(window, cx))
    }
}

#[gpui_kit::test]
fn changing_memory_search_during_load_cannot_restore_old_results(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let window = cx.add_window(|window, cx| {
        let mut view = MemoryView::new(Language::SimplifiedChinese, window, cx);
        view.service = Arc::new(MemoryFixtures::default());
        view.set_visible(true, window, cx);
        view.query.text = "no-match".into();
        view.reload(window, cx);
        view
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.overview.memories.is_empty());
            assert_eq!(view.overview.progress.pending, 3850);
            assert!(!view.loading);
        })
        .unwrap();
}

#[gpui_kit::test]
fn imported_history_is_readable_without_implying_extracted_memories(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        let mut view = MemoryView::new(Language::SimplifiedChinese, window, cx);
        view.service = Arc::new(MemoryFixtures::pending());
        view.set_visible(true, window, cx);
        view
    });
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert_eq!(view.overview.progress.pending, 3873);
            assert_eq!(view.overview.progress.done, 0);
            assert_eq!(view.overview.progress.candidates, 0);
            assert_eq!(view.overview.progress.confirmed, 0);
            assert!(view.overview.topics.is_empty());
            assert!(view.overview.memories.is_empty());
            view.mode = super::Mode::Sources;
            cx.notify();
            view.select(super::Selection::Source(10, 0), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window.update(cx, |view, _, _| {
        assert!(matches!(view.detail, Some(Detail::Source(ref p)) if p.source.state == "pending" && !p.body.is_empty()));
    }).unwrap();
}

#[gpui_kit::test]
fn memory_review_revisions_and_forget_use_native_service(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    for language in [Language::SimplifiedChinese, Language::English] {
        let service = Arc::new(MemoryFixtures::default());
        let mut entity = None;
        let handle = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = MemoryView::new(language, window, cx);
                view.service = service.clone();
                view.set_visible(true, window, cx);
                view
            });
            entity = Some(view.clone());
            let harness = cx.new(|_| MemoryHarness(view));
            Root::new(harness, window, cx)
        });
        let entity = entity.unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("assign-client-session").is_none());
            window.click(("memory-entry", 1_u64), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(("memory-evidence", 10_u64)).visible());
            window.click("memory-confirm", cx);
        })
        .unwrap();
        cx.run_until_parked();
        entity.update(cx, |view, cx| {
            assert!(
                matches!(view.detail,Some(Detail::Memory(ref d)) if d.entry.status=="confirmed")
            );
            cx.notify();
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("memory-confirm").is_none());
            window.click("memory-edit", cx);
            entity.update(cx, |view, cx| {
                let (_, _, body) = view.editing.as_ref().unwrap();
                body.update(cx, |body, cx| body.set_value("用户修订的记忆", window, cx));
                view.refresh(window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            entity.update(cx, |view, cx| {
                assert_eq!(
                    view.editing.as_ref().unwrap().2.read(cx).value().as_ref(),
                    "用户修订的记忆"
                )
            });
            window.render_frame(cx);
            window.click("memory-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("memory-forget", cx);
            window.render_frame(cx);
            window.click("cancel-forget-memory", cx);
            assert!(
                !service
                    .commands
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|c| matches!(c, MemoryCommand::ForgetMemory(_)))
            );
            window.click("memory-forget", cx);
            window.render_frame(cx);
            window.click("confirm-forget-memory", cx);
        })
        .unwrap();
        cx.run_until_parked();
        entity.update(cx, |view, _| {
            assert!(view.overview.memories.is_empty());
            assert!(view.detail.is_none());
            assert_eq!(view.overview.sources.len(), 2);
        });
        assert!(
            service
                .commands
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c,MemoryCommand::Revise {body,..} if body=="用户修订的记忆"))
        );
    }
}

#[gpui_kit::test]
fn topics_filter_evidence_and_failure_details_without_assigning_threads(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let mut entity = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            let mut view = MemoryView::new(Language::SimplifiedChinese, window, cx);
            view.service = Arc::new(MemoryFixtures::default());
            view.set_visible(true, window, cx);
            view
        });
        entity = Some(view.clone());
        Root::new(view, window, cx)
    });
    let entity = entity.unwrap();
    cx.run_until_parked();
    entity.update(cx, |view, _| {
        view.archive = Some(relay_core::sessions::ClientSessionId(
            "previous-archive".into(),
        ));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("memory-topics-tab", cx);
        window.render_frame(cx);
        window.click("memory-topic-relay", cx);
    })
    .unwrap();
    cx.run_until_parked();
    entity.update(cx, |view, _| {
        assert_eq!(view.query.topic.as_deref(), Some("relay"));
        assert!(
            view.archive.is_none(),
            "Changing topic must close unrelated provenance"
        );
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("memory-sources-tab", cx);
        window.render_frame(cx);
        window.click(("memory-source", 11_u64), cx);
    })
    .unwrap();
    cx.run_until_parked();
    entity.update(cx, |view, _| {
        assert!(matches!(view.detail,Some(Detail::Source(ref p)) if p.source.error.is_some()));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("memory-open-archive").visible());
        assert!(window.try_find("client-session-binding").is_none());
    })
    .unwrap();
}
