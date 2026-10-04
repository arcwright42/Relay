use super::super::*;
use relay_core::{ContextItem, ThreadId, threads::ThreadDraft};

pub(in super::super) enum EditorMode {
    Thread(Option<Thread>),
    Note {
        thread: Thread,
        item: Option<ContextItem>,
    },
}

type OnSaved = Box<dyn Fn(ThreadId, &mut Window, &mut App)>;

pub(super) struct ThreadEditor {
    mode: EditorMode,
    language: Language,
    service: Arc<dyn ThreadService>,
    name: Entity<InputState>,
    description: Entity<InputState>,
    body: Entity<TextareaState>,
    included: bool,
    pub saving: bool,
    error: Option<String>,
    on_saved: OnSaved,
}

impl ThreadEditor {
    pub fn new(
        mode: EditorMode,
        language: Language,
        service: Arc<dyn ThreadService>,
        on_saved: OnSaved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (name, description, body, included) = match &mode {
            EditorMode::Thread(thread) => thread
                .as_ref()
                .map(|p| {
                    (
                        p.name.clone(),
                        p.description.clone(),
                        p.instructions.clone(),
                        true,
                    )
                })
                .unwrap_or((String::new(), String::new(), String::new(), true)),
            EditorMode::Note { item, .. } => item
                .as_ref()
                .map(|item| {
                    (
                        item.name.clone(),
                        String::new(),
                        item.content.clone(),
                        item.included,
                    )
                })
                .unwrap_or((String::new(), String::new(), String::new(), true)),
        };
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let description = cx.new(|cx| InputState::new(window, cx).default_value(description));
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(body)
                .auto_grow(5, 9)
        });
        Self {
            mode,
            language,
            service,
            name,
            description,
            body,
            included,
            saving: false,
            error: None,
            on_saved,
        }
    }

    pub fn title(&self) -> &'static str {
        self.language.text(match self.mode {
            EditorMode::Thread(None) => Text::NewThread,
            EditorMode::Thread(Some(_)) => Text::EditThread,
            EditorMode::Note { item: None, .. } => Text::AddNote,
            EditorMode::Note { item: Some(_), .. } => Text::EditNote,
        })
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.name.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        let command = match &self.mode {
            EditorMode::Thread(thread) => {
                let draft = ThreadDraft {
                    name,
                    description: self.description.read(cx).value().to_string(),
                    instructions: body,
                };
                if let Some(thread) = thread {
                    ThreadCommand::Edit {
                        thread: thread.id,
                        expected_revision: thread.revision,
                        draft,
                    }
                } else {
                    ThreadCommand::Create(draft)
                }
            }
            EditorMode::Note { thread, item } => ThreadCommand::SaveContext {
                thread: thread.id,
                expected_revision: thread.revision,
                id: item.as_ref().map(|item| item.id),
                name,
                content: body,
                included: self.included,
            },
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
                    Ok(id) => {
                        window.close_dialog(cx);
                        (this.on_saved)(id, window, cx);
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

impl Render for ThreadEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = matches!(self.mode, EditorMode::Thread(_));
        let body_label = self.language.text(if thread {
            Text::ThreadInstructions
        } else {
            Text::NoteContent
        });
        column()
            .gap(px(13.))
            .child(muted(self.language.text(Text::Name)).text_size(px(13.)))
            .child(
                Input::new(&self.name)
                    .id("thread-editor-name")
                    .disabled(self.saving)
                    .aria_label(self.language.text(Text::Name))
                    .context_menu(crate::locale::input_menu),
            )
            .when(thread, |view| {
                view.child(muted(self.language.text(Text::Description)).text_size(px(13.)))
                    .child(
                        Input::new(&self.description)
                            .id("thread-editor-description")
                            .disabled(self.saving)
                            .aria_label(self.language.text(Text::Description))
                            .context_menu(crate::locale::input_menu),
                    )
            })
            .child(muted(body_label).text_size(px(13.)))
            .child(
                column().id("thread-editor-body").test_support().child(
                    Textarea::new(&self.body)
                        .h(px(200.))
                        .disabled(self.saving)
                        .aria_label(body_label)
                        .context_menu(crate::locale::input_menu),
                ),
            )
            .child(
                muted(self.language.text(if thread {
                    Text::InstructionsHint
                } else {
                    Text::NoteHint
                }))
                .text_size(px(13.)),
            )
            .when(!thread, |view| {
                view.child(
                    Button::new("note-included")
                        .outline()
                        .icon(if self.included {
                            IconName::Check
                        } else {
                            IconName::Plus
                        })
                        .label(self.language.text(if self.included {
                            Text::Included
                        } else {
                            Text::NotIncluded
                        }))
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.included = !this.included;
                            cx.notify();
                        })),
                )
            })
            .when_some(self.error.as_ref(), |view, error| {
                view.child(
                    column()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_color(rgb(0x9a542a))
                                .child(self.language.text(Text::ThreadSaveError)),
                        )
                        .child(muted(error.clone()).text_size(px(13.))),
                )
            })
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("cancel-thread-edit")
                            .ghost()
                            .label(self.language.text(Text::Cancel))
                            .disabled(self.saving)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save-thread-edit")
                            .primary()
                            .label(self.language.text(if self.saving {
                                Text::SavingSettings
                            } else {
                                Text::Save
                            }))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
