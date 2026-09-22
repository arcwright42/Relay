mod agents;
mod conversation;
mod diagnostics;
mod navigation;
mod pages;
mod projects;
#[cfg(test)]
mod tests;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Root, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::{
    i18n::{Text, Translate},
    preview::Page,
};
use relay_core::{
    Project,
    agents::*,
    projects::{ProjectCommand, ProjectService},
    settings::{Language, SettingsService, SettingsSnapshot},
};
use std::{sync::Arc, time::Duration};

actions!(relay, [FocusSearch, SendMessage]);

const INK: u32 = 0x202124;
const MUTED: u32 = 0x8b8b91;
const SIDEBAR: u32 = 0xf4f4f5;
const LINE: u32 = 0xe8e8eb;
const SURFACE: u32 = 0xfdfdfd;

#[track_caller]
fn row() -> Div {
    div().flex().items_center()
}

#[track_caller]
fn column() -> Div {
    div().flex().flex_col()
}

fn icon(name: IconName) -> Icon {
    Icon::new(name).size(px(18.)).text_color(rgb(INK))
}

#[track_caller]
fn muted(text: impl Into<SharedString>) -> Div {
    div().text_color(rgb(MUTED)).child(text.into())
}

fn icon_button(id: &'static str, name: IconName, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .icon(icon(name))
        .size(px(34.))
        .rounded(px(9.))
        .tooltip(tooltip)
        .accessibility_label(tooltip)
}

fn explain(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = title.into();
    let body = body.into();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title(title.clone()).width(px(460.)).child(
            div()
                .text_size(px(14.))
                .line_height(px(23.))
                .text_color(rgb(0x74747b))
                .child(body.clone()),
        )
    });
}

pub struct Workbench {
    project_service: Arc<dyn ProjectService>,
    project_revision: u64,
    project_error: Option<String>,
    project_saving: bool,
    settings_service: Arc<dyn SettingsService>,
    settings_snapshot: SettingsSnapshot,
    page: Page,
    selected_project: usize,
    projects: Vec<Project>,
    drafts: Vec<Entity<TextareaState>>,
    search: Entity<InputState>,
    focus: FocusHandle,
    agent_service: Arc<dyn AgentService>,
    agent_states: Vec<AgentSnapshot>,
    agent_errors: Vec<Option<String>>,
    agent_revision: u64,
    picker_open: bool,
    conversation_scroll: ScrollHandle,
    _agent_updates: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub fn new(
        agent_service: Arc<dyn AgentService>,
        settings_service: Arc<dyn SettingsService>,
        project_service: Arc<dyn ProjectService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_snapshot = settings_service.snapshot();
        let language = settings_snapshot.language;
        let catalog = project_service.snapshot();
        let projects = catalog.projects;
        let drafts: Vec<_> = projects
            .iter()
            .map(|_| {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder(language.text(Text::AskRelay))
                        .auto_grow(2, 6)
                })
            })
            .collect();
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.text(Text::SearchPlaceholder))
        });
        let mut subscriptions = vec![cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        for draft in &drafts {
            subscriptions.push(cx.subscribe_in(draft, window, |_, _, _, _, cx| cx.notify()));
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let agent_states = projects
            .iter()
            .map(|p| agent_service.snapshot(p.id))
            .collect();
        let agent_revision = agent_service.revision();
        let executor = cx.background_executor().clone();
        let updates = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(100)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.refresh_projects(window, cx);
                        this.refresh_agents(cx);
                        let settings = this.settings_service.snapshot();
                        if this.settings_snapshot != settings {
                            this.settings_snapshot = settings;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            project_service,
            project_revision: catalog.revision,
            project_error: catalog.error,
            project_saving: false,
            settings_service,
            settings_snapshot,
            page: if projects.is_empty() {
                Page::Home
            } else {
                Page::Project(0)
            },
            selected_project: 0,
            agent_errors: vec![None; projects.len()],
            projects,
            drafts,
            search,
            focus,
            agent_service,
            agent_states,
            agent_revision,
            picker_open: false,
            conversation_scroll: ScrollHandle::new(),
            _agent_updates: updates,
            _subscriptions: subscriptions,
        }
    }

    fn text(&self, key: Text) -> &'static str {
        self.settings_snapshot.language.text(key)
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_service.set_language(language);
        self.settings_snapshot = self.settings_service.snapshot();
        crate::apply_language(language, cx);
        // User-owned project names, notes and protocol inputs never change with UI language.
        self.search.update(cx, |search, cx| {
            search.set_placeholder(language.text(Text::SearchPlaceholder), window, cx)
        });
        for draft in &self.drafts {
            draft.update(cx, |draft, cx| {
                draft.set_placeholder(language.text(Text::AskRelay), window, cx)
            });
        }
        cx.notify();
    }

    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_open = false;
        if let Page::Project(index) = page {
            self.selected_project = index;
        }
        self.page = page;
        if window.root::<Root>().flatten().is_none() || !window.has_active_dialog(cx) {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        if window.root::<Root>().flatten().is_some() && window.has_active_dialog(cx) {
            return;
        }
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }

    fn send_message(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        if window.root::<Root>().flatten().is_some() && window.has_active_dialog(cx) {
            return;
        }
        if !matches!(self.page, Page::Project(_))
            || self.drafts[self.selected_project]
                .read(cx)
                .value()
                .trim()
                .is_empty()
        {
            return;
        }
        let text = self.drafts[self.selected_project]
            .read(cx)
            .value()
            .to_string();
        if self.agent_states[self.selected_project].status != ConnectionStatus::Ready {
            self.picker_open = true;
            cx.notify();
            return;
        }
        if self.agent_action(AgentCommand::Send(text), cx) {
            self.drafts[self.selected_project].update(cx, |draft, cx| {
                draft.set_value("", window, cx);
            });
            self.conversation_scroll.scroll_to_bottom();
        }
    }

    fn use_prompt(&mut self, prompt: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.drafts[self.selected_project].update(cx, |draft, cx| {
            // Quick actions are editable drafts, and the previous text remains undoable.
            draft.replace_all(prompt, window, cx);
            draft.focus(window, cx);
        });
        cx.notify();
    }
}

fn project_icon(index: usize) -> IconName {
    match index {
        1 => IconName::Cpu,
        2 => IconName::BriefcaseBusiness,
        _ => IconName::FolderClosed,
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(1280.);
        let content = match self.page {
            Page::Project(_) => {
                if self.agent_states[self.selected_project].messages.is_empty() {
                    self.welcome(compact, cx)
                } else {
                    self.conversation(compact, cx)
                }
            }
            Page::Home => self.home(cx),
            Page::Agents if !self.projects.is_empty() => self.agents(cx),
            Page::Agents => self.home(cx),
            Page::Settings => self.settings(cx),
            Page::Inbox => column()
                .flex_1()
                .items_center()
                .justify_center()
                .pb(px(80.))
                .gap(px(15.))
                .child(
                    icon(IconName::Inbox)
                        .size(px(35.))
                        .text_color(rgb(0x9999a0)),
                )
                .child(
                    div()
                        .text_size(px(26.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(self.text(Text::InboxEmpty)),
                )
                .child(muted(self.text(Text::InboxEmptyDetail)).text_size(px(14.))),
        };
        row()
            .id("relay-workbench")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .items_stretch()
            .font_family(".SystemUIFont")
            .text_size(px(14.))
            .text_color(rgb(INK))
            .bg(rgb(SURFACE))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::send_message))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.picker_open && event.keystroke.key == "escape" {
                    this.picker_open = false;
                    window.focus(&this.focus, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(self.sidebar(compact, cx))
            .child(
                column()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.header(cx))
                    .child(content),
            )
            .children(Root::render_dialog_layer(window, cx))
            .map(|view| {
                #[cfg(feature = "devtools")]
                let view = view.on_mouse_down(MouseButton::Right, crate::devtools::show_menu);
                view
            })
    }
}
