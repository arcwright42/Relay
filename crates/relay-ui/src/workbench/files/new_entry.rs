use super::*;

type OnCreated = Box<dyn Fn(FileLocation, &mut Window, &mut App)>;

struct NewEntry {
    parent: FileLocation,
    directory: bool,
    content: Option<String>,
    language: Language,
    service: Arc<dyn FileService>,
    name: Entity<InputState>,
    saving: bool,
    error: Option<FileError>,
    on_created: OnCreated,
}

impl FilesView {
    pub(super) fn new_entry(
        &mut self,
        directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(listing) = &self.listing else {
            return;
        };
        let parent = FileLocation {
            workspace: listing.workspace.clone(),
            path: self.directory.clone(),
        };
        let weak = cx.entity().downgrade();
        show(
            parent,
            directory,
            None,
            self.language,
            self.service.clone(),
            Box::new(move |location, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.change_directory(
                        location
                            .path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new(""))
                            .to_path_buf(),
                        window,
                        cx,
                    );
                    this.error = None;
                    this.refresh(window, cx);
                    if !directory {
                        this.active = Some(location.clone());
                        this.open_file(location, window, cx);
                    }
                    cx.notify();
                });
            }),
            window,
            cx,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    parent: FileLocation,
    directory: bool,
    content: Option<String>,
    language: Language,
    service: Arc<dyn FileService>,
    on_created: OnCreated,
    window: &mut Window,
    cx: &mut App,
) {
    let title = language.text(if directory {
        Text::FilesNewFolder
    } else if content.is_some() {
        Text::FilesSaveCopy
    } else {
        Text::FilesNew
    });
    let editor = cx.new(|cx| NewEntry {
        parent,
        directory,
        content,
        language,
        service,
        name: cx.new(|cx| {
            InputState::new(window, cx).placeholder(if directory {
                language.text(Text::Name)
            } else {
                "notes.md"
            })
        }),
        saving: false,
        error: None,
        on_created,
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let saving = editor.read(cx).saving;
        let cancel_editor = editor.clone();
        dialog
            .title(title)
            .width(px(440.))
            .close_button(!saving)
            .overlay_closable(false)
            .on_cancel(move |_, _, cx| !cancel_editor.read(cx).saving)
            .child(editor.clone())
    });
}

impl NewEntry {
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\', '\0'])
            || name.starts_with(".relay-save-")
        {
            self.error = Some(FileError::InvalidPath);
            cx.notify();
            return;
        }
        let command = FileCommand::Create {
            location: FileLocation {
                path: self.parent.path.join(name),
                ..self.parent.clone()
            },
            directory: self.directory,
            content: self.content.clone().unwrap_or_default(),
        };
        self.saving = true;
        self.error = None;
        let service = self.service.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.apply(command) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(location) => {
                        window.close_dialog(cx);
                        (this.on_created)(location, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for NewEntry {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        column()
            .gap(px(14.))
            .child(muted(self.language.text(Text::Name)).text_size(px(13.)))
            .child(
                Input::new(&self.name)
                    .disabled(self.saving)
                    .aria_label(self.language.text(Text::Name))
                    .context_menu(crate::locale::input_menu),
            )
            .when_some(self.error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(0x9a542a))
                        .child(error_text(self.language, error)),
                )
            })
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("files-cancel-create")
                            .ghost()
                            .label(self.language.text(Text::Cancel))
                            .disabled(self.saving)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("files-confirm-create")
                            .primary()
                            .label(self.language.text(if self.saving {
                                Text::SavingSettings
                            } else {
                                Text::Save
                            }))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                    ),
            )
    }
}
