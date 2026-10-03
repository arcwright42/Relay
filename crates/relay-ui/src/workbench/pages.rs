use super::*;

impl Workbench {
    pub(super) fn home(&self, cx: &mut Context<Self>) -> Div {
        column().flex_1().min_h_0().child(
            column()
                .flex_1()
                .min_h_0()
                .id("home-projects-scroll")
                .overflow_y_scroll()
                .items_center()
                .px(px(32.))
                .child(
                    column()
                        .w_full()
                        .max_w(px(CONTENT_WIDTH))
                        .flex_shrink_0()
                        .my_auto()
                        .pt(px(40.))
                        .pb(px(96.))
                        .gap(px(26.))
                        .child(
                            div()
                                .text_center()
                                .text_size(px(28.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.text(Text::HomePrompt)),
                        )
                        .child(self.routing_composer(cx))
                        .when_some(self.project_error.as_ref(), |view, error| {
                            view.child(div().text_color(rgb(0x9a542a)).child(error.clone()))
                        })
                        .child(
                            column()
                                .gap(px(10.))
                                .child(
                                    row()
                                        .justify_between()
                                        .child(muted(self.text(Text::Projects)).text_size(px(13.)))
                                        .child(
                                            Button::new("home-new-project")
                                                .ghost()
                                                .small()
                                                .icon(icon(IconName::Plus).size(px(14.)))
                                                .label(self.text(Text::NewProject))
                                                .text_size(px(13.))
                                                .disabled(self.project_error.is_some())
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.edit_project(None, window, cx)
                                                })),
                                        ),
                                )
                                .child(row().gap(px(8.)).flex_wrap().children(
                                    self.projects.iter().enumerate().take(3).map(
                                        |(index, project)| {
                                            Button::new(("home-project", index))
                                                .ghost()
                                                .flex_1()
                                                .min_w(px(150.))
                                                .h(px(48.))
                                                .px(px(14.))
                                                .rounded(px(12.))
                                                .border_1()
                                                .border_color(rgb(LINE))
                                                .accessibility_label(project.name.clone())
                                                .when(!project.description.is_empty(), |button| {
                                                    button.tooltip(project.description.clone())
                                                })
                                                .child(
                                                    row()
                                                        .w_full()
                                                        .gap(px(9.))
                                                        .child(
                                                            icon(project_icon(index)).size(px(16.)),
                                                        )
                                                        .child(
                                                            div()
                                                                .min_w_0()
                                                                .flex_1()
                                                                .truncate()
                                                                .text_size(px(13.))
                                                                .child(project.name.clone()),
                                                        ),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.navigate(
                                                            Page::Project(index),
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                ))
                                        },
                                    ),
                                )),
                        ),
                ),
        )
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
                .child(self.routing_settings(cx))
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
