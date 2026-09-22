//! Development-only native menus. Inspection is provided by GPUI Kit.

use gpui_kit::component::{
    ToggleInspector,
    input::{Copy, Cut, Paste, Redo, SelectAll, Undo},
    native_menu::NativeMenu,
};
use gpui_kit::{App, MouseDownEvent, Window};

pub(crate) fn show_menu(event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    cx.stop_propagation();
    NativeMenu::new()
        .menu("检查元素", Box::new(ToggleInspector))
        .show(event.position, window, cx);
}

pub(crate) fn input_menu(menu: NativeMenu, _: &mut Window, _: &mut App) -> NativeMenu {
    // A custom input menu replaces the default menu, so retain editing actions.
    menu.menu("撤销", Box::new(Undo))
        .menu("重做", Box::new(Redo))
        .separator()
        .menu("剪切", Box::new(Cut))
        .menu("复制", Box::new(Copy))
        .menu("粘贴", Box::new(Paste))
        .menu("全选", Box::new(SelectAll))
        .separator()
        .menu("检查元素", Box::new(ToggleInspector))
}
