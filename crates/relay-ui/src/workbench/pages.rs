use super::*;

impl Workbench {
    pub(super) fn home(&self, cx: &mut Context<Self>) -> Div {
        if self.threads.is_empty() {
            return column().child(muted(
                self.thread_error
                    .clone()
                    .unwrap_or_else(|| "Could not load Relay".into()),
            ));
        }
        if self.agent_states[self.selected_thread].messages.is_empty() {
            self.welcome(false, cx)
        } else {
            self.conversation(false, cx)
        }
    }

    pub(super) fn activity(&self, cx: &mut Context<Self>) -> Div {
        let tasks = self.thread_service.activity();
        let list = column()
            .id("task-activity")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(32.))
            .py(px(24.))
            .gap(px(12.))
            .when(tasks.is_empty(), |view| {
                view.child(muted(self.text(Text::InboxEmpty)))
            })
            .children(tasks.into_iter().map(|task| {
                let id = task.thread;
                let status = match task.state.as_str() {
                    "queued" => self.text(Text::TaskQueued),
                    "working" => self.text(Text::TaskWorking),
                    "waiting" => self.text(Text::TaskWaiting),
                    "review" => self.text(Text::TaskReview),
                    "completed" => self.text(Text::TaskCompleted),
                    "interrupted" => self.text(Text::TaskInterrupted),
                    _ => self.text(Text::TaskFailed),
                };
                column()
                    .p(px(16.))
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded(px(12.))
                    .gap(px(8.))
                    .child(
                        Button::new(("open-task", id.0 as usize))
                            .ghost()
                            .label(task.name)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_thread(id, window, cx)
                            })),
                    )
                    .child(muted(status))
                    .child(div().text_size(px(14.)).child(task.summary))
            }));
        column().flex_1().min_h_0().child(list)
    }

    pub(super) fn settings(&self, cx: &mut Context<Self>) -> Div {
        column().flex_1().min_h_0().child(
            column()
                .id("preferences-scroll")
                .flex_1()
                .min_h_0()
                .w_full()
                .max_w(px(CONTENT_WIDTH + 64.))
                .mx_auto()
                .overflow_y_scroll()
                .px(px(32.))
                .py(px(24.))
                .gap(px(18.))
                .child(
                    column()
                        .gap(px(14.))
                        .pb(px(20.))
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(
                            row()
                                .justify_between()
                                .gap(px(24.))
                                .child(
                                    column().flex_1().gap(px(8.)).child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(self.text(Text::Language)),
                                    ),
                                )
                                .child(
                                    row().gap(px(8.)).children(
                                        [Language::SimplifiedChinese, Language::English]
                                            .into_iter()
                                            .map(|language| {
                                                let selected =
                                                    self.settings_snapshot.language == language;
                                                Button::new(language.code())
                                                    .outline()
                                                    .small()
                                                    .label(language.native_name())
                                                    .accessibility_label(language.native_name())
                                                    .rounded(px(8.))
                                                    .when(selected, |button| {
                                                        button.primary().icon(IconName::Check)
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.set_language(language, window, cx)
                                                        },
                                                    ))
                                            }),
                                    ),
                                ),
                        )
                        .when(self.settings_snapshot.saving, |view| {
                            view.child(muted(self.text(Text::SavingSettings)).text_size(px(13.)))
                        })
                        .when_some(self.settings_snapshot.error.as_ref(), |view, error| {
                            view.child(
                                column()
                                    .gap(px(8.))
                                    .text_size(px(13.))
                                    .child(
                                        div()
                                            .text_color(rgb(0x9a542a))
                                            .child(self.text(Text::SettingsError)),
                                    )
                                    .child(muted(error.clone()))
                                    .child(
                                        Button::new("retry-settings-save")
                                            .ghost()
                                            .small()
                                            .label(self.text(Text::Retry))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_language(
                                                    this.settings_snapshot.language,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    ),
                            )
                        }),
                )
                .child(Self::setting_row(
                    self.text(Text::Appearance),
                    self.text(Text::Light),
                    "",
                ))
                .child(self.voice_settings(cx))
                .child(Self::setting_row(
                    self.text(Text::Workspace),
                    self.text(Text::Local),
                    self.text(Text::WorkspaceDetail),
                ))
                .child(Self::setting_row(
                    self.text(Text::AgentConnections),
                    "Codex · ACP",
                    self.text(Text::AgentConnectionsDetail),
                ))
                .child(Self::setting_row("Relay", env!("CARGO_PKG_VERSION"), "")),
        )
    }

    fn setting_row(title: &'static str, value: &'static str, detail: &'static str) -> Div {
        row()
            .justify_between()
            .gap(px(20.))
            .pb(px(22.))
            .border_b_1()
            .border_color(rgb(LINE))
            .child(
                column()
                    .gap(px(8.))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(title.to_owned()),
                    )
                    .when(!detail.is_empty(), |view| {
                        view.child(muted(detail).text_size(px(13.)))
                    }),
            )
            .child(muted(value).text_size(px(13.)))
    }
}
