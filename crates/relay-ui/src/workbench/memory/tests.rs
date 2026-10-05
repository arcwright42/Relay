use super::MemoryView;
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
fn memory_ui_offers_only_provider_operations(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let service = Arc::new(MemoryFixtures::default());
    let mut entity = None;
    let handle = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            let mut view = MemoryView::new(Language::SimplifiedChinese, window, cx);
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
        for id in [
            "memory-connect",
            "memory-pause",
            "memory-topics-tab",
            "memory-sources-tab",
            "memory-edit",
            "memory-confirm",
        ] {
            assert!(window.try_find(id).is_none(), "{id}");
        }
        window.click(("memory-entry", 1_u64), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("memory-confirm").is_none());
        assert!(window.try_find(("memory-evidence", 10_u64)).is_none());
        assert!(window.try_find("memory-edit").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    entity.update(cx, |view, _| assert!(view.detail.is_some()));
    assert!(service.commands.lock().unwrap().is_empty());
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
fn offline_delivery_does_not_imply_extracted_memories(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| {
        let mut view = MemoryView::new(Language::SimplifiedChinese, window, cx);
        view.service = Arc::new(MemoryFixtures::pending());
        view.set_visible(true, window, cx);
        view
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.overview.progress.pending, 3873);
            assert_eq!(view.overview.progress.accepted, 0);
            assert!(view.overview.memories.is_empty());
            assert!(
                view.overview
                    .progress
                    .provider_status
                    .as_deref()
                    .unwrap()
                    .contains("offline")
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn observation_deletion_can_be_cancelled_and_retry_uses_the_provider(cx: &mut TestAppContext) {
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
            window.click("memory-retry", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(matches!(
            service.commands.lock().unwrap().as_slice(),
            [MemoryCommand::RetryFailed]
        ));
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("memory-entry", 1_u64), cx);
        })
        .unwrap();
        cx.run_until_parked();
        entity.update(cx, |view, _| {
            assert!(view.detail.as_ref().is_some_and(|d| d.entry.id == 1))
        });
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
        });
        assert!(
            service
                .commands
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, MemoryCommand::ForgetMemory(1)))
        );
    }
}
