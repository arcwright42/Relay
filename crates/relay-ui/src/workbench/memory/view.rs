use super::*;

impl MemoryView {
    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let provider = self.service.provider();
        let progress = &self.overview.progress;
        column()
            .gap(px(8.))
            .child(div().font_weight(FontWeight::MEDIUM).child(provider.name))
            .child(muted(provider.description).text_size(px(12.)))
            .child(
                muted(progress.provider_status.clone().unwrap_or_else(|| {
                    self.text(if self.load_error.is_some() {
                        Text::MemoryReadFailed
                    } else {
                        Text::Loading
                    })
                    .into()
                }))
                .text_size(px(12.)),
            )
            .child(
                Button::new("memory-retry")
                    .outline()
                    .small()
                    .disabled(self.saving || !provider.capabilities.retry || progress.failed == 0)
                    .label(self.text(Text::MemoryRetry))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.command(MemoryCommand::RetryFailed, window, cx)
                    })),
            )
            .child(muted(self.text(Text::MemoryDeliveryRetryHint)).text_size(px(12.)))
    }

    fn detail_view(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = column()
            .id("memory-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(LINE));
        let Some(detail) = &self.detail else {
            return panel.child(muted(self.text(if self.loading {
                Text::Loading
            } else {
                Text::MemoryChoose
            })));
        };
        let id = detail.entry.id;
        panel
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(detail.entry.title.clone()),
            )
            .child(
                muted(format!(
                    "{} · {}",
                    self.text(Text::MemoryRecorded),
                    detail.entry.kind
                ))
                .text_size(px(12.)),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .line_height(px(23.))
                    .child(detail.body.clone()),
            )
            .child(
                row()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(detail.concepts.iter().map(|t| {
                        muted(t.clone())
                            .px(px(8.))
                            .py(px(4.))
                            .bg(rgb(SIDEBAR))
                            .rounded(px(6.))
                    })),
            )
            .when(
                self.service.provider().capabilities.forget_memory,
                |panel| {
                    panel.child(
                        Button::new("memory-forget")
                            .ghost()
                            .small()
                            .disabled(self.saving)
                            .label(self.text(Text::MemoryForget))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.forget_dialog(id, window, cx)
                            })),
                    )
                },
            )
    }
}

impl Render for MemoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let next = self.overview.next_memory;
        let list = column()
            .id("memory-list")
            .w(px(240.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .gap(px(6.))
            .when(self.overview.memories.is_empty(), |v| {
                v.child(
                    muted(self.text(if self.loading {
                        Text::Loading
                    } else {
                        Text::MemoryEmpty
                    }))
                    .p(px(12.))
                    .text_size(px(13.)),
                )
            })
            .children(self.overview.memories.iter().map(|memory| {
                let id = memory.id;
                Button::new(("memory-entry", id))
                    .ghost()
                    .h_auto()
                    .w_full()
                    .justify_start()
                    .p(px(10.))
                    .disabled(self.saving)
                    .when(self.selection == Some(id), |b| b.bg(rgb(0xededf0)))
                    .child(
                        column()
                            .w_full()
                            .whitespace_normal()
                            .text_left()
                            .gap(px(6.))
                            .child(
                                div()
                                    .line_clamp(2)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(memory.title.clone()),
                            )
                            .child(muted(memory.kind.clone()).text_size(px(12.)))
                            .child(
                                muted(memory.preview.chars().take(100).collect::<String>())
                                    .line_clamp(3)
                                    .text_size(px(12.)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
            }))
            .child(
                row()
                    .gap(px(6.))
                    .mt(px(8.))
                    .child(
                        Button::new("memory-first-page")
                            .outline()
                            .small()
                            .disabled(self.query.before_memory.is_none() || self.saving)
                            .label(self.text(Text::MemoryFirst))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.query.before_memory = None;
                                this.reload(window, cx);
                            })),
                    )
                    .child(
                        Button::new("memory-next-page")
                            .outline()
                            .small()
                            .disabled(next.is_none() || self.saving)
                            .label(self.text(Text::MemoryNext))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.query.before_memory = next;
                                this.reload(window, cx);
                            })),
                    ),
            );
        column()
            .flex_1()
            .min_h_0()
            .h_full()
            .px(px(24.))
            .pb(px(18.))
            .gap(px(12.))
            .child(self.toolbar(cx))
            .when_some(
                self.error.clone().or(self.load_error.clone()).or(self
                    .overview
                    .progress
                    .error
                    .clone()),
                |v, error| {
                    v.child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0xb04435))
                            .child(error),
                    )
                },
            )
            .child(
                Input::new(&self.search)
                    .id("memory-search")
                    .disabled(self.saving)
                    .w_full(),
            )
            .child(
                row()
                    .id("memory-browser")
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .gap(px(12.))
                    .child(list)
                    .child(self.detail_view(cx)),
            )
    }
}
