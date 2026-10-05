mod agents;
mod conversation;
mod diagnostics;
mod files;
pub use files::SaveFile;
mod navigation;
mod pages;
mod quick;
mod threads;
pub use quick::{OpenAgentSettings, OpenThread, RequestAccessibility, ResizeQuick};
mod memory;
mod submission;
mod voice;
pub use voice::OpenMicrophoneSettings;
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
    Thread,
    agents::*,
    settings::{Language, SettingsService, SettingsSnapshot},
    threads::{ThreadCommand, ThreadService},
    voice::{EmptyVoiceService, VoiceService, VoiceSnapshot},
};
use std::{sync::Arc, time::Duration};

actions!(relay, [FocusSearch, SendMessage]);

const INK: u32 = 0x202124;
const MUTED: u32 = 0x77777f;
const SIDEBAR: u32 = 0xf7f7f8;
const LINE: u32 = 0xe8e8eb;
const SURFACE: u32 = 0xffffff;
const CONTENT_WIDTH: f32 = 720.;

#[track_caller]
fn row() -> Div {
    div().flex().items_center()
}

#[track_caller]
fn column() -> Div {
    div().flex().flex_col()
}

fn composer_surface() -> Div {
    column()
        .w_full()
        .p(px(14.))
        .rounded(px(24.))
        .border_1()
        .border_color(rgb(0xe1e1e4))
        .bg(rgb(SURFACE))
        .shadow(vec![BoxShadow {
            inset: false,
            color: rgba(0x0000000a).into(),
            offset: point(px(0.), px(2.)),
            blur_radius: px(8.),
            spread_radius: px(0.),
        }])
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
    quick: Option<quick::QuickEntry>,
    thread_service: Arc<dyn ThreadService>,
    thread_revision: u64,
    thread_error: Option<String>,
    thread_saving: bool,
    settings_service: Arc<dyn SettingsService>,
    settings_snapshot: SettingsSnapshot,
    voice_service: Arc<dyn VoiceService>,
    voice_snapshot: VoiceSnapshot,
    pending_send: Option<(relay_core::ThreadId, String)>,
    page: Page,
    files: Entity<files::FilesView>,
    selected_thread: usize,
    threads: Vec<Thread>,
    drafts: Vec<Entity<TextareaState>>,
    search: Entity<InputState>,
    focus: FocusHandle,
    agent_service: Arc<dyn AgentService>,
    agent_states: Vec<AgentSnapshot>,
    agent_errors: Vec<Option<String>>,
    agent_revision: u64,
    memory: Entity<memory::MemoryView>,
    picker_open: bool,
    conversation_scroll: ScrollHandle,
    _agent_updates: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub fn new(
        agent_service: Arc<dyn AgentService>,
        settings_service: Arc<dyn SettingsService>,
        thread_service: Arc<dyn ThreadService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_snapshot = settings_service.snapshot();
        let language = settings_snapshot.language;
        let catalog = thread_service.snapshot();
        let threads = catalog.threads;
        let files = cx.new(|cx| files::FilesView::new(language, window, cx));
        let drafts: Vec<_> = threads
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
        let memory = cx.new(|cx| memory::MemoryView::new(language, window, cx));
        let mut subscriptions = vec![cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        subscriptions.push(cx.subscribe_in(
            &memory,
            window,
            |this, _, _: &memory::ConnectCoordinator, window, cx| {
                this.navigate(Page::Home, window, cx);
                if let Some(index) = this.threads.iter().position(|t| t.id.0 == 0) {
                    let state = &this.agent_states[index];
                    if matches!(
                        state.status,
                        ConnectionStatus::Disconnected | ConnectionStatus::Failed
                    ) && let Err(error) = this.agent_service.dispatch(
                        this.threads[index].id,
                        AgentCommand::Connect(state.source.clone()),
                    ) {
                        this.agent_errors[index] = Some(error);
                    }
                }
                cx.notify();
            },
        ));
        for draft in &drafts {
            subscriptions.push(
                cx.subscribe_in(draft, window, |this, _, event, window, cx| {
                    if this.quick.is_some()
                        && matches!(event, InputEvent::PressEnter { shift: false, .. })
                    {
                        this.quick_send(relay_core::capture::QuickAction::Ask, window, cx);
                    }
                    cx.notify();
                }),
            );
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let agent_states = threads
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
                        this.refresh_threads(window, cx);
                        this.refresh_agents(cx);
                        this.refresh_memory_agent(cx);
                        this.advance_pending_send(window, cx);
                        let settings = this.settings_service.snapshot();
                        if this.settings_snapshot != settings {
                            this.settings_snapshot = settings;
                            cx.notify();
                        }
                        let voice = this.voice_service.snapshot();
                        if this.voice_snapshot != voice {
                            this.voice_snapshot = voice;
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
            quick: None,
            thread_service,
            thread_revision: catalog.revision,
            thread_error: catalog.error,
            thread_saving: false,
            settings_service,
            settings_snapshot,
            voice_service: Arc::new(EmptyVoiceService),
            voice_snapshot: VoiceSnapshot::default(),
            pending_send: None,
            page: Page::Home,
            files,
            selected_thread: 0,
            agent_errors: vec![None; threads.len()],
            threads,
            drafts,
            search,
            focus,
            agent_service,
            agent_states,
            agent_revision,
            memory,
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
        self.files
            .update(cx, |files, cx| files.set_language(language, window, cx));
        // User-owned thread names, notes and protocol inputs never change with UI language.
        self.search.update(cx, |search, cx| {
            search.set_placeholder(language.text(Text::SearchPlaceholder), window, cx)
        });
        self.memory
            .update(cx, |memory, cx| memory.set_language(language, window, cx));
        for draft in &self.drafts {
            draft.update(cx, |draft, cx| {
                draft.set_placeholder(language.text(Text::AskRelay), window, cx)
            });
        }
        cx.notify();
    }

    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_open = false;
        self.pending_send = None;
        if let Page::Thread(index) = page {
            self.selected_thread = index;
        }
        if page == Page::Home {
            self.selected_thread = self.threads.iter().position(|t| t.id.0 == 0).unwrap_or(0);
        }
        self.page = page;
        if page == Page::Memory {
            self.refresh_memory_agent(cx);
        }
        self.memory.update(cx, |memory, cx| {
            memory.set_visible(page == Page::Memory, window, cx)
        });
        self.files.update(cx, |files, cx| {
            files.set_visible(page == Page::Files, window, cx)
        });
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
        if self.quick.is_some() {
            self.quick_send(relay_core::capture::QuickAction::Ask, window, cx);
            return;
        }
        if !matches!(self.page, Page::Thread(_) | Page::Home)
            || self.drafts[self.selected_thread]
                .read(cx)
                .value()
                .trim()
                .is_empty()
        {
            return;
        }
        let text = self.drafts[self.selected_thread]
            .read(cx)
            .value()
            .to_string();
        if self.agent_states[self.selected_thread].status != ConnectionStatus::Ready {
            if matches!(
                self.agent_states[self.selected_thread].status,
                ConnectionStatus::Disconnected | ConnectionStatus::Failed
            ) {
                self.pending_send = Some((self.threads[self.selected_thread].id, text));
                self.agent_action(
                    AgentCommand::Connect(self.agent_states[self.selected_thread].source.clone()),
                    cx,
                );
            } else {
                self.picker_open = true;
            }
            cx.notify();
            return;
        }
        if self.agent_action(AgentCommand::Send(text), cx) {
            self.drafts[self.selected_thread].update(cx, |draft, cx| {
                draft.set_value("", window, cx);
            });
            self.conversation_scroll.scroll_to_bottom();
        }
    }

    fn use_prompt(&mut self, prompt: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.drafts[self.selected_thread].update(cx, |draft, cx| {
            // Quick actions are editable drafts, and the previous text remains undoable.
            draft.replace_all(prompt, window, cx);
            draft.focus(window, cx);
        });
        cx.notify();
    }
}

fn thread_icon(index: usize) -> IconName {
    match index {
        1 => IconName::Cpu,
        2 => IconName::BriefcaseBusiness,
        _ => IconName::FolderClosed,
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(1280.);
        let content = if self.quick.is_some() {
            self.quick_view(cx)
        } else {
            match self.page {
                Page::Thread(_) => {
                    if self.agent_states[self.selected_thread].messages.is_empty() {
                        self.welcome(compact, cx)
                    } else {
                        self.conversation(compact, cx)
                    }
                }
                Page::Home => self.home(cx),
                Page::Agents if !self.threads.is_empty() => self.agents(cx),
                Page::Agents => self.home(cx),
                Page::Settings => self.settings(cx),
                Page::Memory => column().flex_1().min_h_0().child(self.memory.clone()),
                Page::Files => column().flex_1().min_h_0().child(self.files.clone()),
                Page::Inbox => self.activity(cx),
            }
        };
        row()
            .id("relay-workbench")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .items_stretch()
            .font_family(".SystemUIFont")
            .text_size(px(16.))
            .text_color(rgb(INK))
            .when(self.quick.is_none(), |view| view.bg(rgb(SURFACE)))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::send_message))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.picker_open && event.keystroke.key == "escape" {
                    this.picker_open = false;
                    window.focus(&this.focus, cx);
                    cx.stop_propagation();
                    cx.notify();
                } else if this.quick.is_some() && event.keystroke.key == "escape" {
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .when(self.quick.is_none(), |view| {
                view.child(self.sidebar(compact, cx))
            })
            .child(
                column()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .when(self.quick.is_none(), |view| view.child(self.header(cx)))
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
