use super::*;
use gpui_kit::component::Selectable;
#[cfg(test)]
mod tests;
mod view;
use relay_core::{memory::*, sessions::*};

pub(super) struct ConnectCoordinator;
impl EventEmitter<ConnectCoordinator> for MemoryView {}

impl Workbench {
    pub fn set_memory_services(
        &mut self,
        service: Arc<dyn MemoryService>,
        archives: Arc<dyn ClientSessionsService>,
        cx: &mut Context<Self>,
    ) {
        self.memory.update(cx, |view, cx| {
            view.service = service;
            view.archives = archives;
            view.refresh_archives(cx);
        });
    }
    pub(super) fn refresh_memory_agent(&mut self, cx: &mut Context<Self>) {
        let status = self
            .threads
            .iter()
            .position(|t| t.id.0 == 0)
            .map(|i| self.agent_states[i].status.clone())
            .unwrap_or(ConnectionStatus::Disconnected);
        self.memory.update(cx, |view, cx| {
            if view.agent_status != status {
                view.agent_status = status;
                cx.notify();
            }
            if view.visible {
                view.refresh_archives(cx);
            }
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Memories,
    Topics,
    Sources,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Memory(u64),
    Source(u64, usize),
}
enum Detail {
    Memory(MemoryDetail),
    Source(MemorySourcePage),
}

pub(super) struct MemoryView {
    service: Arc<dyn MemoryService>,
    archives: Arc<dyn ClientSessionsService>,
    archive_state: ClientSessionsSnapshot,
    archive_revision: u64,
    archive: Option<ClientSessionId>,
    archive_page: usize,
    language: Language,
    agent_status: ConnectionStatus,
    visible: bool,
    overview: MemoryOverview,
    mode: Mode,
    query: MemoryQuery,
    search: Entity<InputState>,
    selection: Option<Selection>,
    detail: Option<Detail>,
    editing: Option<(u64, Entity<InputState>, Entity<TextareaState>)>,
    loading: bool,
    loaded: bool,
    saving: bool,
    generation: u64,
    error: Option<String>,
    load_error: Option<String>,
    _updates: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl MemoryView {
    pub(super) fn new(language: Language, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(language.text(Text::MemorySearch)));
        let subscription = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.query.text = this.search.read(cx).value().to_string();
                this.query.before_memory = None;
                this.query.before_source = None;
                this.reload(window, cx);
            }
        });
        let executor = cx.background_executor().clone();
        let updates = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_secs(3)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        if this.visible {
                            this.refresh(window, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            service: Arc::new(EmptyMemoryService),
            archives: Arc::new(EmptyClientSessions),
            archive_state: ClientSessionsSnapshot::default(),
            archive_revision: 0,
            archive: None,
            archive_page: 0,
            language,
            agent_status: ConnectionStatus::Disconnected,
            visible: false,
            overview: MemoryOverview::default(),
            mode: Mode::Memories,
            query: MemoryQuery::default(),
            search,
            selection: None,
            detail: None,
            editing: None,
            loading: false,
            loaded: false,
            saving: false,
            generation: 0,
            error: None,
            load_error: None,
            _updates: updates,
            _subscriptions: vec![subscription],
        }
    }
    fn text(&self, key: Text) -> &'static str {
        self.language.text(key)
    }
    pub(super) fn set_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.language = language;
        self.search.update(cx, |search, cx| {
            search.set_placeholder(language.text(Text::MemorySearch), window, cx)
        });
        cx.notify();
    }
    pub(super) fn set_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible = visible;
        if visible {
            self.refresh(window, cx);
        }
    }
    fn refresh_archives(&mut self, cx: &mut Context<Self>) {
        let revision = self.archives.revision();
        if self.archive_revision != revision {
            self.archive_revision = revision;
            self.archive_state = self.archives.snapshot();
            cx.notify();
        }
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.loading = false;
        self.error = None;
        self.load_error = None;
        self.overview.memories.clear();
        self.overview.sources.clear();
        self.refresh(window, cx);
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.saving {
            return;
        }
        self.loading = true;
        self.generation += 1;
        let generation = self.generation;
        let selection = self.selection;
        let service = self.service.clone();
        let query = self.query.clone();
        let work = cx.background_executor().spawn(async move {
            let overview = service.overview(&query);
            let detail = selection
                .map(|s| match s {
                    Selection::Memory(id) => service.detail(id).map(Detail::Memory),
                    Selection::Source(id, offset) => service.source(id, offset).map(Detail::Source),
                })
                .transpose();
            (overview, detail)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (overview, detail) = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                this.load_error = None;
                match overview {
                    Ok(data) => {
                        this.overview = data;
                        this.loaded = true;
                    }
                    Err(e) => this.load_error = Some(e),
                }
                match detail {
                    Ok(data) => this.detail = data,
                    Err(e) => {
                        this.detail = None;
                        this.load_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn select(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() || self.saving {
            return;
        }
        self.selection = Some(selection);
        self.detail = None;
        self.archive = None;
        self.reload(window, cx);
    }
    fn command(&mut self, command: MemoryCommand, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.generation += 1;
        self.loading = false;
        self.saving = true;
        self.error = None;
        let forget = matches!(
            command,
            MemoryCommand::ForgetMemory(_) | MemoryCommand::ForgetSource(_)
        );
        let service = self.service.clone();
        let work = cx
            .background_executor()
            .spawn(async move { service.apply(command) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(id) => {
                        this.editing = None;
                        if forget {
                            this.selection = None;
                            this.detail = None;
                            this.archive = None;
                        }
                        if let Some(id) = id {
                            this.selection = Some(Selection::Memory(id));
                            this.detail = None;
                        }
                        this.query.before_memory = None;
                        this.query.before_source = None;
                        this.reload(window, cx);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Detail::Memory(detail)) = &self.detail else {
            return;
        };
        let id = detail.entry.id;
        let title = detail.entry.title.clone();
        let body = detail.body.clone();
        let title = cx.new(|cx| InputState::new(window, cx).default_value(title));
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(body)
                .auto_grow(6, 16)
        });
        self.editing = Some((id, title, body));
        cx.notify();
    }
    fn forget_dialog(&self, source: bool, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let title = self.text(if source {
            Text::MemoryForgetSource
        } else {
            Text::MemoryForget
        });
        let hint = self.text(if source {
            Text::MemoryForgetSourceHint
        } else {
            Text::MemoryForgetHint
        });
        let cancel = self.text(Text::Cancel);
        let confirm = title;
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            dialog.title(title).width(px(440.)).child(
                column().gap(px(18.)).child(hint).child(
                    row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("cancel-forget-memory")
                                .outline()
                                .label(cancel)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-forget-memory")
                                .label(confirm)
                                .on_click(move |_, window, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.command(
                                            if source {
                                                MemoryCommand::ForgetSource(id)
                                            } else {
                                                MemoryCommand::ForgetMemory(id)
                                            },
                                            window,
                                            cx,
                                        )
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                ),
            )
        });
    }
}
