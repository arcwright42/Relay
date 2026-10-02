use super::*;

impl Render for FilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if let Some(location) = &self.active {
            if let Some(editor) = self.editors.get(location) {
                column()
                    .flex_1()
                    .min_h_0()
                    .child(editor.clone())
                    .into_any_element()
            } else {
                column()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(16.))
                    .when(self.error.is_none(), |view| {
                        view.child(muted(self.language.text(Text::FilesLoading)))
                    })
                    .child(
                        Button::new("file-back-loading")
                            .ghost()
                            .h(px(36.))
                            .text_size(px(14.))
                            .icon(IconName::ArrowLeft)
                            .label(self.language.text(Text::FilesBack))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_active(window, cx)),
                            ),
                    )
                    .into_any_element()
            }
        } else {
            self.browser(window.viewport_size().width >= px(1100.), cx)
                .into_any_element()
        };
        column()
            .id("files-workspace")
            .test_support()
            .size_full()
            .min_h_0()
            .min_w_0()
            .text_size(px(16.))
            .on_action(cx.listener(Self::save_active))
            .when_some(self.error.as_ref(), |view, error| {
                view.child(
                    div()
                        .px(px(28.))
                        .py(px(12.))
                        .text_size(px(14.))
                        .text_color(rgb(0x9a542a))
                        .child(error_text(self.language, error)),
                )
            })
            .child(content)
    }
}

impl FilesView {
    fn browser(&self, wide: bool, cx: &mut Context<Self>) -> Div {
        let language = self.language;
        let query = self.search.read(cx).value().trim().to_lowercase();
        let entries: Vec<_> = self
            .listing
            .as_ref()
            .map(|listing| {
                listing
                    .entries
                    .iter()
                    .filter(|entry| entry.name.to_lowercase().contains(&query))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut breadcrumbs = vec![(language.text(Text::FilesRoot).to_owned(), PathBuf::new())];
        let mut parent = PathBuf::new();
        for component in self.directory.components() {
            parent.push(component.as_os_str());
            breadcrumbs.push((
                component.as_os_str().to_string_lossy().into_owned(),
                parent.clone(),
            ));
        }
        column()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .px(px(28.))
            .pb(px(20.))
            .child(
                row()
                    .gap(px(12.))
                    .pb(px(24.))
                    .justify_between()
                    .child(
                        div().w(px(if wide { 280. } else { 220. })).min_w_0().child(
                            Input::new(&self.search)
                                .h(px(38.))
                                .text_size(px(14.))
                                .prefix(icon(IconName::Search).size(px(17.)).text_color(rgb(MUTED)))
                                .aria_label(language.text(Text::FilesSearch))
                                .context_menu(crate::locale::input_menu),
                        ),
                    )
                    .child(
                        row()
                            .gap(px(8.))
                            .child(
                                Button::new("files-new-folder")
                                    .ghost()
                                    .h(px(36.))
                                    .text_size(px(14.))
                                    .icon(IconName::FolderPlus)
                                    .tooltip(language.text(Text::FilesNewFolder))
                                    .accessibility_label(language.text(Text::FilesNewFolder))
                                    .disabled(self.listing.is_none() || self.mutating)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_entry(true, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("files-new-file")
                                    .outline()
                                    .h(px(36.))
                                    .text_size(px(14.))
                                    .icon(IconName::Plus)
                                    .label(language.text(Text::FilesNew))
                                    .disabled(self.listing.is_none() || self.mutating)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_entry(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("files-import")
                                    .primary()
                                    .h(px(36.))
                                    .text_size(px(14.))
                                    .icon(IconName::Upload)
                                    .label(language.text(Text::FilesImport))
                                    .disabled(self.listing.is_none() || self.mutating)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_files(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                row()
                    .gap(px(4.))
                    .pb(px(16.))
                    .justify_between()
                    .child(
                        row().flex_1().min_w_0().gap(px(4.)).children(
                            breadcrumbs.into_iter().enumerate().map(
                                |(index, (name, directory))| {
                                    let last = directory == self.directory;
                                    row()
                                        .min_w_0()
                                        .when(index > 0, |view| {
                                            view.child(
                                                icon(IconName::ChevronRight)
                                                    .size(px(15.))
                                                    .text_color(rgb(MUTED)),
                                            )
                                        })
                                        .child(
                                            Button::new(("files-breadcrumb", index))
                                                .ghost()
                                                .h(px(32.))
                                                .text_size(px(15.))
                                                .text_color(rgb(if last { INK } else { MUTED }))
                                                .label(name)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.change_directory(
                                                            directory.clone(),
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                )),
                                        )
                                },
                            ),
                        ),
                    )
                    .child(
                        row()
                            .gap(px(4.))
                            .child(
                                Button::new("files-refresh")
                                    .ghost()
                                    .h(px(32.))
                                    .icon(IconName::RefreshCw)
                                    .tooltip(language.text(Text::FilesRefresh))
                                    .accessibility_label(language.text(Text::FilesRefresh))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                    ),
                            )
                            .when_some(self.listing.as_ref(), |view, listing| {
                                let workspace = listing.workspace.join(&self.directory);
                                view.child(
                                    Button::new("files-reveal-workspace")
                                        .ghost()
                                        .h(px(32.))
                                        .icon(IconName::ExternalLink)
                                        .tooltip(language.text(Text::FilesInFinder))
                                        .accessibility_label(language.text(Text::FilesInFinder))
                                        .on_click(move |_, _, cx| cx.reveal_path(&workspace)),
                                )
                            }),
                    ),
            )
            .child(
                row()
                    .h(px(38.))
                    .px(px(12.))
                    .gap(px(14.))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child(div().flex_1().child(language.text(Text::Name)))
                    .when(wide, |view| {
                        view.child(div().w(px(150.)).child(language.text(Text::FilesType)))
                    })
                    .child(
                        div()
                            .w(px(100.))
                            .text_right()
                            .pr(px(12.))
                            .child(language.text(Text::FilesSize)),
                    ),
            )
            .child(
                column()
                    .id("files-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(px(4.))
                    .when(entries.is_empty(), |view| {
                        view.child(self.empty_browser(!query.is_empty(), cx))
                    })
                    .children(entries.iter().map(|entry| self.file_row(entry, wide, cx))),
            )
            .when(
                self.listing
                    .as_ref()
                    .is_some_and(|listing| listing.truncated),
                |view| {
                    view.child(
                        muted(language.text(Text::FilesListLimit))
                            .pt(px(12.))
                            .text_size(px(13.)),
                    )
                },
            )
    }

    fn empty_browser(&self, filtered: bool, cx: &mut Context<Self>) -> Div {
        let loading = self.loading && self.listing.is_none();
        column()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .gap(px(18.))
            .pb(px(60.))
            .child(
                div()
                    .size(px(64.))
                    .rounded(px(18.))
                    .bg(rgb(SIDEBAR))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(if filtered {
                            IconName::Search
                        } else {
                            IconName::FolderClosed
                        })
                        .size(px(30.))
                        .text_color(rgb(0x74747c)),
                    ),
            )
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.language.text(if loading {
                        Text::FilesLoading
                    } else if filtered {
                        Text::FilesNoMatches
                    } else {
                        Text::FilesEmpty
                    })),
            )
            .when(!loading && !filtered && self.error.is_none(), |view| {
                view.child(
                    Button::new("files-empty-import")
                        .outline()
                        .h(px(38.))
                        .text_size(px(14.))
                        .icon(IconName::Upload)
                        .label(self.language.text(Text::FilesImport))
                        .disabled(self.listing.is_none() || self.mutating)
                        .on_click(cx.listener(|this, _, window, cx| this.import_files(window, cx))),
                )
            })
    }

    fn file_row(&self, entry: &FileEntry, wide: bool, cx: &mut Context<Self>) -> Button {
        let location = self.listing.as_ref().map(|listing| FileLocation {
            workspace: listing.workspace.clone(),
            path: entry.path.clone(),
        });
        let dirty = location
            .as_ref()
            .and_then(|location| self.editors.get(location))
            .is_some_and(|editor| editor.read(cx).dirty(cx));
        let entry = entry.clone();
        Button::new(format!("files-entry-{}", entry.name))
            .ghost()
            .w_full()
            .h(px(56.))
            .justify_start()
            .px(px(12.))
            .rounded(px(8.))
            .accessibility_label(entry.name.clone())
            .child(
                row()
                    .w_full()
                    .min_w_0()
                    .gap(px(14.))
                    .child(
                        div()
                            .size(px(32.))
                            .flex_shrink_0()
                            .rounded(px(8.))
                            .bg(rgb(if entry.kind == FileKind::Directory {
                                0xf0f2f5
                            } else {
                                0xf8f8f9
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(file_icon(entry.kind))
                                    .size(px(20.))
                                    .text_color(rgb(0x686870)),
                            ),
                    )
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(16.))
                                    .text_color(rgb(INK))
                                    .child(entry.name.clone()),
                            )
                            .when(dirty, |view| {
                                view.child(div().size(px(6.)).rounded_full().bg(rgb(0x787882)))
                            }),
                    )
                    .when(wide, |view| {
                        view.child(
                            muted(self.language.text(file_type(entry.kind)))
                                .w(px(150.))
                                .text_size(px(14.)),
                        )
                    })
                    .child(
                        muted(if entry.kind == FileKind::Directory {
                            "—".to_owned()
                        } else {
                            file_size(entry.stamp.bytes)
                        })
                        .w(px(100.))
                        .pr(px(12.))
                        .text_right()
                        .text_size(px(14.)),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.select_entry(&entry, window, cx)))
    }
}

fn file_type(kind: FileKind) -> Text {
    match kind {
        FileKind::Directory => Text::FilesFolderType,
        FileKind::Markdown => Text::FilesMarkdownType,
        FileKind::Text => Text::FilesTextType,
        FileKind::Image => Text::FilesImageType,
        FileKind::Document => Text::FilesDocumentType,
        FileKind::Other => Text::FilesOtherType,
    }
}
