#![cfg(feature = "devtools")]

use gpui_kit::{
    AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, TestAppContext, TestSupportExt, Window, div, px,
    test::TestWindowExt,
};
use std::{cell::Cell, rc::Rc};

struct Content {
    clicks: Rc<Cell<usize>>,
}

impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        div().size_full().child(
            div()
                .id("content")
                .size(px(64.))
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                .test_support(),
        )
    }
}

// Exercise GPUI's real pointer dispatch, including capture, picking, and hit testing.
// The minimal Inspector renderer keeps this test independent of theme/font assets.
fn close_inspector(cx: &mut TestAppContext, select_first: bool) {
    cx.update(|cx| {
        cx.set_inspector_renderer(Box::new(|_, _, _| {
            div()
                .size_full()
                .flex()
                .justify_end()
                .child(
                    div()
                        .id("inspector-close")
                        .size(px(32.))
                        .on_click(|_, window, cx| window.toggle_inspector(cx))
                        .test_support(),
                )
                .into_any_element()
        }));
    });
    let clicks = Rc::new(Cell::new(0));
    let handle = cx.add_window(|_, _| Content {
        clicks: clicks.clone(),
    });
    cx.update_window(handle.into(), |_, window, cx| {
        // Reopening must also restore picking and leave the close control usable.
        for _ in 0..2 {
            window.toggle_inspector(cx);
            window.render_frame(cx);
            assert!(window.is_inspector_picking(cx));
            if select_first {
                window.click("content", cx);
                assert!(!window.is_inspector_picking(cx));
                assert_eq!(clicks.get(), 0, "picking must not activate application UI");
            }
            window.click("inspector-close", cx);
            assert!(
                window.try_find("inspector-close").is_none(),
                "the close button must remove Inspector, including during picking"
            );
            assert!(!window.is_inspector_picking(cx));
        }
        window.click("content", cx);
        assert_eq!(clicks.get(), 1, "normal input must resume after closing");
    })
    .unwrap();
}

#[gpui_kit::test]
fn inspector_close_works_while_picking(cx: &mut TestAppContext) {
    close_inspector(cx, false);
}

#[gpui_kit::test]
fn inspector_close_works_after_selection(cx: &mut TestAppContext) {
    close_inspector(cx, true);
}
