use super::*;

impl Workbench {
    pub(super) fn home(&self, cx: &mut Context<Self>) -> Div {
        column()
            .flex_1()
            .justify_center()
            .px(px(55.))
            .pb(px(80.))
            .gap(px(13.))
            .child(
                div()
                    .text_size(px(31.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Welcome back, Alex."),
            )
            .child(
                muted("A fresh thought, or a familiar project?")
                    .text_size(px(18.))
                    .mb(px(22.)),
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
            }))
    }

    pub(super) fn agents(&self) -> Div {
        column().flex_1().justify_center().px(px(55.)).pb(px(80.)).gap(px(13.))
            .child(div().text_size(px(30.)).font_weight(FontWeight::SEMIBOLD).child("Your agents, one workspace."))
            .child(muted("Keep your context. Choose who helps you.").text_size(px(18.)).mb(px(22.)))
            .children([("Claude Code", "C"), ("Codex", "C"), ("OpenCode", "O")].into_iter().enumerate().map(|(index, (name, initial))| {
                row().gap(px(18.)).p(px(20.)).rounded(px(13.)).border_1().border_color(rgb(LINE))
                    .child(row().justify_center().size(px(39.)).rounded(px(11.)).bg(rgb(0xf0f0f2)).font_weight(FontWeight::MEDIUM).child(initial))
                    .child(column().flex_1().gap(px(5.)).child(div().font_weight(FontWeight::MEDIUM).child(name)).child(muted("Not connected").text_size(px(12.))))
                    .child(Button::new(("connect", index)).outline().label("Connect").rounded(px(8.)).on_click(move |_, window, cx| explain(format!("Connect {name}"), "Agent connection setup is the next step. Relay will use Agent Client Protocol to connect your local agent to this workspace.", window, cx)))
            }))
    }

    pub(super) fn settings(&self) -> Div {
        column()
            .flex_1()
            .justify_center()
            .px(px(55.))
            .pb(px(80.))
            .gap(px(18.))
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .mb(px(12.))
                    .child("A space that feels like you."),
            )
            .child(Self::setting_row(
                "Appearance",
                "Light",
                "A quiet canvas for your work.",
            ))
            .child(Self::setting_row(
                "Workspace",
                "Preview",
                "Drafts are kept per project for this app session.",
            ))
            .child(Self::setting_row(
                "Agent connections",
                "Not connected",
                "Your context belongs to the project.",
            ))
            .child(Self::setting_row(
                "Relay",
                env!("CARGO_PKG_VERSION"),
                "Made for the way you work.",
            ))
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
