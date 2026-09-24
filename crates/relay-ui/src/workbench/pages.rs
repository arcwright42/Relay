use super::*;

impl Workbench {
    pub(super) fn home(&self, cx: &mut Context<Self>) -> Div {
        column().flex_1().min_h_0().child(
            column()
                .flex_1()
                .id("home-projects-scroll")
                .overflow_y_scroll()
                .justify_center()
                .px(px(55.))
                .pb(px(80.))
                .gap(px(13.))
                .child(
                    div()
                        .text_size(px(31.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.text(Text::WelcomeBack)),
                )
                .child(
                    muted(self.text(Text::HomePrompt))
                        .text_size(px(18.))
                        .mb(px(22.)),
                )
                .child(self.routing_composer(cx))
                .when_some(self.project_error.as_ref(), |view, error| {
                    view.child(div().text_color(rgb(0x9a542a)).child(error.clone()))
                })
                .child(
                    Button::new("home-new-project")
                        .outline()
                        .icon(IconName::Plus)
                        .label(self.text(Text::NewProject))
                        .disabled(self.project_error.is_some())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_project(None, window, cx)),
                        ),
                )
                .children(self.projects.iter().enumerate().map(|(index, project)| {
                    row()
                        .id(("home-project", index))
                        .cursor_pointer()
                        .gap(px(20.))
                        .p(px(22.))
                        .rounded(px(13.))
                        .border_1()
                        .border_color(rgb(LINE))
                        .hover(|this| this.bg(rgb(0xf5f5f6)))
                        .child(icon(project_icon(index)).size(px(25.)))
                        .child(
                            column()
                                .flex_1()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(project.name.clone()),
                                )
                                .child(muted(project.description.clone()).text_size(px(13.))),
                        )
                        .child(
                            icon(IconName::ArrowUpRight)
                                .size(px(18.))
                                .text_color(rgb(MUTED)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate(Page::Project(index), window, cx)
                        }))
                })),
        )
    }

    pub(super) fn settings(&self, cx: &mut Context<Self>) -> Div {
        column().flex_1().min_h_0().child(
            column()
                .id("preferences-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(55.))
                .py(px(34.))
                .gap(px(18.))
                .child(
                    div()
                        .text_size(px(30.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .mb(px(12.))
                        .child(self.text(Text::SettingsTitle)),
                )
                .child(
                    column()
                        .gap(px(14.))
                        .p(px(22.))
                        .rounded(px(14.))
                        .border_1()
                        .border_color(rgb(LINE))
                        .mb(px(10.))
                        .child(
                            row()
                                .justify_between()
                                .gap(px(24.))
                                .child(
                                    column()
                                        .flex_1()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(self.text(Text::Language)),
                                        )
                                        .child(
                                            muted(self.text(Text::LanguageDetail))
                                                .text_size(px(12.)),
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
                            view.child(muted(self.text(Text::SavingSettings)).text_size(px(12.)))
                        })
                        .when_some(self.settings_snapshot.error.as_ref(), |view, error| {
                            view.child(
                                column()
                                    .gap(px(8.))
                                    .text_size(px(12.))
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
                    self.text(Text::AppearanceDetail),
                ))
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
                .child(Self::setting_row(
                    "Relay",
                    env!("CARGO_PKG_VERSION"),
                    self.text(Text::AboutDetail),
                )),
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
                    .child(muted(detail).text_size(px(12.))),
            )
            .child(muted(value).text_size(px(13.)))
    }
}
