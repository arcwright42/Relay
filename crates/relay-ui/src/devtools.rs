//! Development-only native menus. Inspection is provided by GPUI Kit.

use crate::{
    i18n::{Text, Translate},
    locale::current_language,
};
use gpui_kit::component::{ToggleInspector, native_menu::NativeMenu};
use gpui_kit::{App, MouseDownEvent, Window};

pub(crate) fn show_menu(event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    cx.stop_propagation();
    NativeMenu::new()
        .menu(
            current_language(cx).text(Text::InspectElement),
            Box::new(ToggleInspector),
        )
        .show(event.position, window, cx);
}
