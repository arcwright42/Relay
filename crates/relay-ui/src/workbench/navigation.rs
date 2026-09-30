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
                    .gap(px(10.))
                    .text_size(px(13.))
                    .child(icon(glyph).size(px(17.)))
                    .child(div().flex_1().min_w_0().truncate().child(name)),
            )
            .w_full()
            .h(px(36.))
            .flex_shrink_0()
            .justify_start()
            .px(px(10.))
            .rounded(px(8.))
            .text_size(px(13.))
            .when(self.page == page, |this| this.bg(rgb(0xeaeaec)))
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
            .w(px(if compact { 212. } else { 228. }))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(SIDEBAR))
            .px(px(12.))
            .pt(px(62.))
            .pb(px(16.))
            .child(
                row()
                    .gap(px(9.))
                    .px(px(10.))
                    .child(
                        Icon::default()
                            .data(include_bytes!("../../../../assets/relay-mark.svg"))
                            .size(px(23.)),
                    )
                    .child(
                        div()
                            .text_size(px(19.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Relay"),
                    ),
            )
            .child(
                div()
                    .mt(px(20.))
                    .mb(px(14.))
                    .rounded(px(9.))
                    .bg(rgb(0xededf0))
                    .child(
                        Input::new(&self.search)
                            .appearance(false)
                            .bordered(false)
                            .h(px(33.))
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
                    .mt(px(22.))
                    .mb(px(5.))
                    .child(muted(self.text(Text::Projects)).text_size(px(12.)))
                    .child(
                        icon_button(
                            "create-project",
                            IconName::Plus,
                            self.text(Text::NewProject),
                        )
                        .size(px(26.))
                        .disabled(self.project_error.is_some())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_project(None, window, cx)),
                        ),
                    ),
            )
            .child(
                column()
                    .id("sidebar-projects")
                    .flex_1()
                    .min_h_0()
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
            .child(
                column()
                    .flex_shrink_0()
                    .pt(px(10.))
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
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> Div {
        let title = match self.page {
            Page::Project(index) => self.projects[index].name.as_str(),
            Page::Home => "",
            Page::Inbox => self.text(Text::Inbox),
            Page::Agents => self.text(Text::Agents),
            Page::Settings => self.text(Text::Settings),
        };
        row()
            .h(px(60.))
            .flex_shrink_0()
            .px(px(28.))
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(14.))
                    .child(title.to_owned())
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .when(matches!(self.page, Page::Project(_)), |view| {
                view.child(
                    row()
                        .gap(px(7.))
                        .child(
                            icon_button(
                                "project-members",
                                IconName::Settings,
                                self.text(Text::ProjectDetails),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    if let Some(project) =
                                        this.projects.get(this.selected_project).cloned()
                                    {
                                        this.edit_project(Some(project), window, cx);
                                    }
                                },
                            )),
                        )
                        .child(
                            icon_button("project-menu", IconName::Layers, self.text(Text::Context))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.show_context(window, cx);
                                })),
                        )
                        .child(
                            icon_button(
                                "project-memory",
                                IconName::FileText,
                                self.text(Text::ProjectMemory),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    if let Some(project) = this.projects.get(this.selected_project)
                                    {
                                        this.show_project_memory(project.id, window, cx);
                                    }
                                },
                            )),
                        ),
                )
            })
    }
}
