mod conversation;
mod navigation;
mod pages;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Root, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::preview::{Page, preview_projects};
use relay_core::{ContextKind, Project};

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
    page: Page,
    selected_project: usize,
    projects: Vec<Project>,
    drafts: Vec<Entity<TextareaState>>,
    search: Entity<InputState>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let projects = preview_projects();
        let drafts: Vec<_> = projects
            .iter()
            .map(|_| {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("Ask Relay…")
                        .auto_grow(2, 6)
                })
            })
            .collect();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search…"));
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
        Self {
            page: Page::Project(0),
            selected_project: 0,
            projects,
            drafts,
            search,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if let Page::Project(index) = page {
            self.selected_project = index;
        }
        self.page = page;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }

    fn send_message(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.page, Page::Project(_))
            || self.drafts[self.selected_project]
                .read(cx)
                .value()
                .trim()
                .is_empty()
        {
            return;
        }
        explain(
            "Connect your project agent",
            "Your message is ready. Connect a local agent to start the conversation. Your draft will stay in this project.",
            window,
            cx,
        );
    }

    fn use_prompt(&mut self, prompt: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.drafts[self.selected_project].update(cx, |draft, cx| {
            // Quick actions are editable drafts, and the previous text remains undoable.
            draft.replace_all(prompt, window, cx);
            draft.focus(window, cx);
        });
        cx.notify();
    }

    fn show_context(&self, kind: Option<ContextKind>, window: &mut Window, cx: &mut Context<Self>) {
        let project = &self.projects[self.selected_project];
        let items: Vec<_> = project
            .context
            .iter()
            .filter(|item| kind.is_none_or(|kind| kind == item.kind))
            .cloned()
            .collect();
        let title: SharedString = match kind {
            Some(kind) => format!("{} · {}", project.name.clone(), context_label(kind)).into(),
            None => format!("{} · Context", project.name).into(),
        };
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(title.clone()).width(px(480.)).child(
                column()
                    .gap(px(12.))
                    .child(
                        muted("Project context stays with you when you change agents.")
                            .text_size(px(13.)),
                    )
                    .child(
                        column()
                            .id("context-preview-list")
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .gap(px(4.))
                            .children(items.iter().map(|item| {
                                row()
                                    .gap(px(12.))
                                    .py(px(9.))
                                    .child(icon(context_icon(item.kind)))
                                    .child(item.name.clone())
                            }))
                            .when(items.is_empty(), |this| {
                                this.child(muted("No context in this project yet.").py(px(20.)))
                            }),
                    )
                    .child(
                        muted("Sample library · File import will be connected next.")
                            .text_size(px(12.))
                            .pt(px(8.)),
                    ),
            )
        });
    }
}

fn project_icon(index: usize) -> IconName {
    match index {
        1 => IconName::Cpu,
        2 => IconName::BriefcaseBusiness,
        _ => IconName::FolderClosed,
    }
}

fn context_icon(kind: ContextKind) -> IconName {
    match kind {
        ContextKind::Web => IconName::PanelsTopLeft,
        ContextKind::Document => IconName::FileText,
        ContextKind::Image => IconName::Image,
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(1280.);
        let content = match self.page {
            Page::Project(_) => self.welcome(compact, cx),
            Page::Home => self.home(cx),
            Page::Agents => self.agents(),
            Page::Settings => self.settings(),
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
                        .child("All clear."),
                )
                .child(muted("Ideas you capture along the way will land here.").text_size(px(14.))),
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

fn context_label(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::Web => "Web pages",
        ContextKind::Document => "Documents",
        ContextKind::Image => "Images",
    }
}
