use super::*;

impl Render for FileEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.language;
        let dirty = self.dirty(cx);
        let editable = matches!(self.document.content, FileContent::Text { .. });
        let weak = cx.entity().downgrade();
        column()
            .id("file-editor")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                row()
                    .h(px(64.))
                    .px(px(28.))
                    .pb(px(12.))
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("file-back")
                            .ghost()
                            .size(px(36.))
                            .icon(IconName::ArrowLeft)
                            .tooltip(language.text(Text::FilesBack))
                            .accessibility_label(language.text(Text::FilesBack))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CloseFile))),
                    )
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        self.document
                                            .location
                                            .path
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned(),
                                    ),
                            )
                            .when(dirty, |view| {
                                view.child(
                                    div()
                                        .size(px(6.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(rgb(0x787882)),
                                )
                            }),
                    )
                    .child(
                        row()
                            .gap(px(8.))
                            .when(editable, |view| {
                                view.child(
                                    row()
                                        .gap(px(2.))
                                        .p(px(3.))
                                        .rounded(px(9.))
                                        .bg(rgb(SIDEBAR))
                                        .child(
                                            Button::new("file-preview-mode")
                                                .ghost()
                                                .h(px(30.))
                                                .text_size(px(14.))
                                                .label(language.text(Text::FilesPreview))
                                                .when(self.preview, |button| {
                                                    button.bg(rgb(SURFACE))
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.preview = true;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new("file-toggle-preview")
                                                .ghost()
                                                .h(px(30.))
                                                .text_size(px(14.))
                                                .label(language.text(Text::FilesEdit))
                                                .when(!self.preview, |button| {
                                                    button.bg(rgb(SURFACE))
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.preview = false;
                                                    cx.notify();
                                                })),
                                        ),
                                )
                                .child(
                                    Button::new("file-save-copy")
                                        .ghost()
                                        .size(px(36.))
                                        .icon(IconName::Copy)
                                        .tooltip(language.text(Text::FilesSaveCopy))
                                        .accessibility_label(language.text(Text::FilesSaveCopy))
                                        .disabled(self.saving)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.save_copy(window, cx)
                                        })),
                                )
                                .when(dirty, |view| {
                                    view.child(
                                        Button::new("file-save")
                                            .primary()
                                            .h(px(36.))
                                            .text_size(px(14.))
                                            .label(language.text(if self.saving {
                                                Text::SavingSettings
                                            } else {
                                                Text::Save
                                            }))
                                            .disabled(self.saving || self.external.is_some())
                                            .tooltip("⌘S")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save(window, cx)
                                            })),
                                    )
                                })
                            })
                            .child(
                                Button::new("file-open-system")
                                    .ghost()
                                    .size(px(36.))
                                    .icon(IconName::ExternalLink)
                                    .tooltip(language.text(Text::FilesOpenSystem))
                                    .accessibility_label(language.text(Text::FilesOpenSystem))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.open_with_system(&this.document.location.absolute_path())
                                    })),
                            ),
                    ),
            )
            .when(
                self.external.is_some() || self.error == Some(FileError::Conflict),
                |view| {
                    view.child(
                        column()
                            .mx(px(28.))
                            .mt(px(16.))
                            .p(px(16.))
                            .rounded(px(10.))
                            .bg(rgb(0xfff6e8))
                            .gap(px(12.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(rgb(0x9a542a))
                                    .child(language.text(Text::FilesConflict)),
                            )
                            .child(
                                row()
                                    .gap(px(8.))
                                    .child(
                                        Button::new("file-reload")
                                            .outline()
                                            .h(px(34.))
                                            .text_size(px(14.))
                                            .label(language.text(Text::FilesReload))
                                            .disabled(self.saving)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.reload(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("file-conflict-copy")
                                            .ghost()
                                            .h(px(34.))
                                            .text_size(px(14.))
                                            .label(language.text(Text::FilesSaveCopy))
                                            .disabled(self.saving)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save_copy(window, cx)
                                            })),
                                    ),
                            ),
                    )
                },
            )
            .when_some(
                self.error
                    .as_ref()
                    .filter(|error| **error != FileError::Conflict),
                |view, error| {
                    view.child(
                        div()
                            .px(px(28.))
                            .py(px(14.))
                            .text_size(px(14.))
                            .text_color(rgb(0x9a542a))
                            .child(error_text(language, error)),
                    )
                },
            )
            .child(match &self.document.content {
                FileContent::Text { markdown, .. } if self.preview && *markdown => column()
                    .id("file-markdown-preview")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(860.))
                            .mx_auto()
                            .px(px(36.))
                            .py(px(32.))
                            .text_size(px(16.))
                            .child(
                                TextView::markdown(
                                    "file-markdown",
                                    self.body.read(cx).value().to_string(),
                                )
                                .selectable(true)
                                .on_link_click(
                                    move |link, _, _, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.open_link(link.to_string(), cx)
                                        });
                                    },
                                ),
                            ),
                    )
                    .into_any_element(),
                FileContent::Text { .. } => column()
                    .id("file-text-editor")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .px(px(28.))
                    .py(px(24.))
                    .child(
                        Textarea::new(&self.body)
                            .h(relative(1.))
                            .text_size(px(16.))
                            .font_family("Menlo")
                            .readonly(self.preview)
                            .disabled(self.saving)
                            .aria_label(language.text(Text::FilesContent))
                            .context_menu(crate::locale::input_menu),
                    )
                    .into_any_element(),
                FileContent::Image { .. } => column()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .p(px(32.))
                    .bg(rgb(0xf9f9fa))
                    .when_some(self.image.clone(), |view, image| {
                        view.child(
                            img(image)
                                .max_w_full()
                                .max_h_full()
                                .object_fit(ObjectFit::Contain),
                        )
                    })
                    .into_any_element(),
                FileContent::Binary { preview } => column()
                    .id("file-document-preview")
                    .flex_1()
                    .min_h_0()
                    .p(px(28.))
                    .gap(px(16.))
                    .when_some(preview.clone(), |view, path| {
                        view.child(
                            column()
                                .flex_1()
                                .min_h_0()
                                .items_center()
                                .justify_center()
                                .bg(rgb(0xf9f9fa))
                                .rounded(px(12.))
                                .p(px(16.))
                                .child(
                                    img(path)
                                        .max_w_full()
                                        .max_h_full()
                                        .object_fit(ObjectFit::Contain),
                                ),
                        )
                    })
                    .when(preview.is_none(), |view| {
                        view.child(
                            column()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap(px(20.))
                                .child(
                                    icon(IconName::FileText)
                                        .size(px(48.))
                                        .text_color(rgb(MUTED)),
                                )
                                .child(
                                    div()
                                        .text_size(px(18.))
                                        .child(language.text(Text::FilesSystemPreview)),
                                ),
                        )
                    })
                    .child(
                        row()
                            .justify_center()
                            .gap(px(16.))
                            .when(preview.is_some(), |view| {
                                view.child(
                                    muted(language.text(Text::FilesFirstPage)).text_size(px(13.)),
                                )
                            })
                            .child(
                                Button::new("file-view-document")
                                    .outline()
                                    .h(px(36.))
                                    .text_size(px(14.))
                                    .label(language.text(Text::FilesOpenSystem))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.open_with_system(&this.document.location.absolute_path())
                                    })),
                            ),
                    )
                    .into_any_element(),
            })
    }
}
