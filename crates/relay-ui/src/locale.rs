use crate::i18n::{Text, Translate};
use gpui_kit::{App, Global, Menu, MenuItem, actions};
use gpui_kit::{
    Window,
    component::{
        input::{Copy, Cut, Paste, Redo, SelectAll, Undo},
        native_menu::NativeMenu,
    },
};
use relay_core::settings::Language;

actions!(relay, [Quit]);

pub(crate) struct UiLanguage(pub Language);
impl Global for UiLanguage {}

pub(crate) fn current_language(cx: &App) -> Language {
    cx.try_global::<UiLanguage>()
        .map(|locale| locale.0)
        .unwrap_or_default()
}

pub(crate) fn input_menu(menu: NativeMenu, _: &mut Window, cx: &mut App) -> NativeMenu {
    let language = current_language(cx);
    // A custom menu replaces the default, so retain all editing actions.
    let menu = menu
        .menu(language.text(Text::Undo), Box::new(Undo))
        .menu(language.text(Text::Redo), Box::new(Redo))
        .separator()
        .menu(language.text(Text::Cut), Box::new(Cut))
        .menu(language.text(Text::Copy), Box::new(Copy))
        .menu(language.text(Text::Paste), Box::new(Paste))
        .menu(language.text(Text::SelectAll), Box::new(SelectAll));
    #[cfg(feature = "devtools")]
    let menu = menu.separator().menu(
        language.text(Text::InspectElement),
        Box::new(gpui_kit::component::ToggleInspector),
    );
    menu
}

/// Refresh component strings and native menus along with Relay's own views.
pub fn apply_language(language: Language, cx: &mut App) {
    cx.set_global(UiLanguage(language));
    gpui_kit::component::set_locale(language.code());
    let menus =
        vec![Menu::new("Relay").items([MenuItem::action(language.text(Text::QuitRelay), Quit)])];
    #[cfg(feature = "devtools")]
    let menus = {
        let mut menus = menus;
        menus.push(
            Menu::new(language.text(Text::Developer)).items([MenuItem::action(
                language.text(Text::InspectElement),
                gpui_kit::component::ToggleInspector,
            )]),
        );
        menus
    };
    cx.set_menus(menus);
}
