use super::*;
#[cfg(test)]
mod tests;
mod view;
use relay_core::memory::*;

impl Workbench {
    pub fn set_memory_service(&mut self, service: Arc<dyn MemoryService>, cx: &mut Context<Self>) {
        self.memory.update(cx, |view, cx| {
            view.service = service;
            cx.notify();
        });
    }
}

pub(super) struct MemoryView {
    service: Arc<dyn MemoryService>,
    language: Language,
    visible: bool,
    overview: MemoryOverview,
    query: MemoryQuery,
    search: Entity<InputState>,
    selection: Option<u64>,
    detail: Option<MemoryDetail>,
    loading: bool,
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
            language,
            visible: false,
            overview: MemoryOverview::default(),
            query: MemoryQuery::default(),
            search,
            selection: None,
            detail: None,
            loading: false,
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
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.loading = false;
        self.error = None;
        self.load_error = None;
        self.overview.memories.clear();
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
            let detail = selection.map(|id| service.detail(id)).transpose();
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
    fn select(&mut self, selection: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.selection = Some(selection);
        self.detail = None;
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
        let forget = matches!(command, MemoryCommand::ForgetMemory(_));
        let service = self.service.clone();
        let work = cx
            .background_executor()
            .spawn(async move { service.apply(command) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        if forget {
                            this.selection = None;
                            this.detail = None;
                        }
                        this.query.before_memory = None;
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
    fn forget_dialog(&self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let title = self.text(Text::MemoryForget);
        let hint = self.text(Text::MemoryProviderForgetHint);
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
                                        this.command(MemoryCommand::ForgetMemory(id), window, cx)
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                ),
            )
        });
    }
}
