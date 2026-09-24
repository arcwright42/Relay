use super::*;

impl Workbench {
    fn nav_button(
        &self,
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        glyph: IconName,
        page: Page,
        cx: &mut Context<Self>,
    ) -> Button {
        let name = name.into();
        Button::new(id)
            .ghost()
            .accessibility_label(name.clone())
            .child(
                row()
                    .w_full()
                    .gap(px(14.))
                    .text_size(px(14.))
                    .child(icon(glyph))
                    .child(name),
            )
            .w_full()
            .h(px(41.))
            .justify_start()
            .px(px(13.))
            .gap(px(14.))
            .rounded(px(9.))
            .text_size(px(14.))
            .when(self.page == page, |this| this.bg(rgb(0xe9e9eb)))
            .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)))
    }

    pub(super) fn sidebar(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let visible: Vec<_> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.name.to_lowercase().contains(&query))
            .collect();
        column()
            .w(px(if compact { 214. } else { 244. }))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(SIDEBAR))
            .px(px(16.))
            .pt(px(62.))
            .pb(px(28.))
            .child(
                row()
                    .gap(px(9.))
                    .px(px(10.))
                    .child(
                        Icon::default()
                            .data(include_bytes!("../../../../assets/relay-mark.svg"))
                            .size(px(27.)),
                    )
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Relay"),
                    ),
            )
            .child(
                div()
                    .mt(px(22.))
                    .mb(px(27.))
                    .rounded(px(9.))
                    .bg(rgb(0xebebed))
                    .child(
                        Input::new(&self.search)
                            .appearance(false)
                            .h(px(37.))
                            .text_size(px(13.))
                            .prefix(
                                icon(IconName::Search)
                                    .size(px(15.))
                                    .text_color(rgb(0x6c6c72)),
                            )
                            .suffix(muted("⌘K").text_size(px(12.)))
                            .aria_label(self.text(Text::SearchProjects))
                            .context_menu(crate::locale::input_menu),
                    ),
            )
            .child(
                column()
                    .gap(px(3.))
                    .child(self.nav_button(
                        "home",
                        self.text(Text::Home),
                        IconName::House,
                        Page::Home,
                        cx,
                    ))
                    .child(self.nav_button(
                        "inbox",
                        self.text(Text::Inbox),
                        IconName::Inbox,
                        Page::Inbox,
                        cx,
                    )),
            )
            .child(
                row()
                    .justify_between()
                    .px(px(10.))
                    .mt(px(24.))
                    .mb(px(7.))
                    .child(muted(self.text(Text::Projects)).text_size(px(12.)))
                    .child(
                        icon_button(
                            "create-project",
                            IconName::Plus,
                            self.text(Text::NewProject),
                        )
                        .disabled(self.project_error.is_some())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_project(None, window, cx)),
                        ),
                    ),
            )
            .child(
                column()
                    .id("sidebar-projects")
                    .max_h(px(380.))
                    .overflow_y_scroll()
                    .gap(px(3.))
                    .children(visible.iter().map(|(index, project)| {
                        self.nav_button(
                            ("project", *index),
                            project.name.clone(),
                            project_icon(*index),
                            Page::Project(*index),
                            cx,
                        )
                    }))
                    .when(visible.is_empty(), |this| {
                        this.child(
                            muted(self.text(Text::NoProjects))
                                .px(px(12.))
                                .py(px(10.))
                                .text_size(px(12.)),
                        )
                    }),
            )
            .child(div().flex_1())
            .child(
                column()
                    .gap(px(3.))
                    .child(self.nav_button(
                        "agents",
                        self.text(Text::Agents),
                        IconName::Box,
                        Page::Agents,
                        cx,
                    ))
                    .child(self.nav_button(
                        "settings",
                        self.text(Text::Settings),
                        IconName::Settings,
                        Page::Settings,
                        cx,
                    )),
            )
            .child(
                row()
                    .mt(px(62.))
                    .px(px(8.))
                    .gap(px(15.))
                    .child(
                        row()
                            .justify_center()
                            .size(px(40.))
                            .rounded_full()
                            .bg(rgb(0xe9e9eb))
                            .text_size(px(18.))
                            .child("A"),
                    )
                    .child(div().text_size(px(14.)).child("Alex")),
            )
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> Div {
        let (title, subtitle, glyph) = match self.page {
            Page::Project(index) => (
                self.projects[index].name.as_str(),
                self.projects[index].description.as_str(),
                project_icon(index),
            ),
            Page::Home => (
                self.text(Text::Home),
                self.text(Text::HomeSubtitle),
                IconName::House,
            ),
            Page::Inbox => (
                self.text(Text::Inbox),
                self.text(Text::InboxSubtitle),
                IconName::Inbox,
            ),
            Page::Agents => (
                self.text(Text::Agents),
                self.text(Text::AgentsSubtitle),
                IconName::Box,
            ),
            Page::Settings => (
                self.text(Text::Settings),
                self.text(Text::SettingsSubtitle),
                IconName::Settings,
            ),
        };
        row()
            .h(px(88.))
            .flex_shrink_0()
            .px(px(36.))
            .justify_between()
            .child(
                row().gap(px(17.)).child(icon(glyph).size(px(21.))).child(
                    column()
                        .gap(px(5.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(15.))
                                .child(title.to_owned()),
                        )
                        .child(muted(subtitle.to_owned()).text_size(px(12.))),
                ),
            )
            .child(
                row()
                    .gap(px(7.))
                    .child(
                        icon_button(
                            "project-members",
                            IconName::Settings,
                            self.text(Text::ProjectDetails),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(project) = this.projects.get(this.selected_project).cloned()
                            {
                                this.edit_project(Some(project), window, cx);
                            }
                        })),
                    )
                    .child(
                        icon_button("project-menu", IconName::Layers, self.text(Text::Context))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_context(window, cx);
                            })),
                    ),
            )
    }
}
