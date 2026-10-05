pub(super) mod editor;
use super::*;
use editor::{EditorMode, ThreadEditor};
use relay_core::{ContextItem, ThreadId};
use std::collections::BTreeMap;

struct ContextLibrary {
    workbench: WeakEntity<Workbench>,
    thread: ThreadId,
}

impl Render for ContextLibrary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.workbench
            .update(cx, |workbench, cx| {
                workbench.context_library(self.thread, cx)
            })
            .unwrap_or_else(|_| column())
    }
}

impl Workbench {
    pub(super) fn refresh_threads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.thread_service.revision() == self.thread_revision {
            return;
        }
        let catalog = self.thread_service.snapshot();
        let selected = self.threads.get(self.selected_thread).map(|p| p.id);
        let mut drafts: BTreeMap<_, _> = self
            .threads
            .iter()
            .map(|p| p.id)
            .zip(std::mem::take(&mut self.drafts))
            .collect();
        let mut errors: BTreeMap<_, _> = self
            .threads
            .iter()
            .map(|p| p.id)
            .zip(std::mem::take(&mut self.agent_errors))
            .collect();
        self.thread_revision = catalog.revision;
        self.thread_error = catalog.error;
        self.threads = catalog.threads;
        let placeholder = self.text(Text::AskRelay);
        for thread in &self.threads {
            let draft = drafts.remove(&thread.id).unwrap_or_else(|| {
                let draft = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder(placeholder)
                        .auto_grow(2, 6)
                });
                self._subscriptions.push(cx.subscribe_in(
                    &draft,
                    window,
                    |this, _, event, window, cx| {
                        if this.quick.is_some()
                            && matches!(event, InputEvent::PressEnter { shift: false, .. })
                        {
                            this.quick_send(relay_core::capture::QuickAction::Ask, window, cx);
                        }
                        cx.notify();
                    },
                ));
                draft
            });
            self.drafts.push(draft);
            self.agent_errors.push(errors.remove(&thread.id).flatten());
        }
        self.selected_thread = self
            .threads
            .iter()
            .position(|p| Some(p.id) == selected)
            .unwrap_or(0);
        if matches!(self.page, Page::Thread(_)) {
            self.page = if self.threads.is_empty() {
                Page::Home
            } else {
                Page::Thread(self.selected_thread)
            };
        }
        self.agent_states = self
            .threads
            .iter()
            .map(|p| self.agent_service.snapshot(p.id))
            .collect();
        self.agent_revision = self.agent_service.revision();
        cx.notify();
    }

    pub(super) fn edit_thread(
        &mut self,
        thread: Option<Thread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor(EditorMode::Thread(thread), window, cx);
    }

    fn edit_note(
        &mut self,
        thread: Thread,
        item: Option<ContextItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor(EditorMode::Note { thread, item }, window, cx);
    }

    pub(super) fn open_editor(
        &mut self,
        mode: EditorMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let editor = cx.new(|cx| {
            ThreadEditor::new(
                mode,
                self.settings_snapshot.language,
                self.thread_service.clone(),
                Box::new(move |id, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.refresh_threads(window, cx);
                        if let Some(index) = this.threads.iter().position(|p| p.id == id) {
                            this.navigate(Page::Thread(index), window, cx);
                        }
                    });
                }),
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog, _, cx| {
            let state = editor.read(cx);
            let title = state.title();
            let saving = state.saving;
            let cancel_editor = editor.clone();
            dialog
                .title(title)
                .width(px(610.))
                .close_button(!saving)
                .overlay_closable(false)
                .on_cancel(move |_, _, cx| !cancel_editor.read(cx).saving)
                .child(editor.clone())
        });
    }

    pub(super) fn show_context(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.threads.get(self.selected_thread) else {
            return;
        };
        let id = thread.id;
        let workbench = cx.entity().downgrade();
        let library = cx.new(|_| ContextLibrary {
            workbench,
            thread: id,
        });
        let title = self.text(Text::Context);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(title).width(px(640.)).child(library.clone())
        });
    }

    fn context_library(&self, id: ThreadId, cx: &mut Context<Self>) -> Div {
        let Some(thread) = self.threads.iter().find(|p| p.id == id) else {
            return column();
        };
        let add_thread = thread.clone();
        let edit_thread = thread.clone();
        column()
            .gap(px(16.))
            .child(muted(self.text(Text::ContextIndependent)).text_size(px(13.)))
            .child(muted(self.text(Text::ContextSyncDetail)).text_size(px(13.)))
            .child(
                Button::new("context-thread-instructions")
                    .outline()
                    .justify_start()
                    .icon(IconName::Settings)
                    .label(self.text(Text::ThreadInstructions))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_thread(Some(edit_thread.clone()), window, cx)
                    })),
            )
            .child(
                column()
                    .id("thread-context-list")
                    .max_h(px(340.))
                    .overflow_y_scroll()
                    .gap(px(8.))
                    .when(thread.context.is_empty(), |view| {
                        view.child(muted(self.text(Text::NoContext)).py(px(24.)))
                    })
                    .children(thread.context.iter().map(|item| {
                        let note = item.clone();
                        let editing_thread = thread.clone();
                        let toggle = ThreadCommand::SaveContext {
                            thread: id,
                            expected_revision: thread.revision,
                            id: Some(item.id),
                            name: item.name.clone(),
                            content: item.content.clone(),
                            included: !item.included,
                        };
                        let remove = ThreadCommand::RemoveContext {
                            thread: id,
                            expected_revision: thread.revision,
                            id: item.id,
                        };
                        column()
                            .gap(px(7.))
                            .p(px(14.))
                            .rounded(px(10.))
                            .border_1()
                            .border_color(rgb(LINE))
                            .child(
                                row()
                                    .gap(px(10.))
                                    .child(icon(IconName::FileText))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(item.name.clone()),
                                    )
                                    .child(
                                        Button::new(("include-note", item.id.0))
                                            .ghost()
                                            .small()
                                            .icon(if item.included {
                                                IconName::Check
                                            } else {
                                                IconName::Plus
                                            })
                                            .label(self.text(if item.included {
                                                Text::Included
                                            } else {
                                                Text::NotIncluded
                                            }))
                                            .disabled(self.thread_saving)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.apply_thread_change(
                                                    toggle.clone(),
                                                    false,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(("edit-note", item.id.0))
                                            .ghost()
                                            .small()
                                            .label(self.text(Text::Edit))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.edit_note(
                                                    editing_thread.clone(),
                                                    Some(note.clone()),
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(("remove-note", item.id.0))
                                            .ghost()
                                            .small()
                                            .label(self.text(Text::Remove))
                                            .disabled(self.thread_saving)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.confirm_remove(remove.clone(), window, cx)
                                            })),
                                    ),
                            )
                            .child(
                                muted(item.content.chars().take(100).collect::<String>())
                                    .text_size(px(13.)),
                            )
                    })),
            )
            .child(
                Button::new("add-context-note")
                    .primary()
                    .icon(IconName::Plus)
                    .label(self.text(Text::AddNote))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_note(add_thread.clone(), None, window, cx)
                    })),
            )
    }

    pub(super) fn confirm_remove(
        &self,
        command: ThreadCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let language = self.settings_snapshot.language;
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            let cancelling = weak.clone();
            let cancel_button = weak.clone();
            let command = command.clone();
            dialog
                .title(language.text(Text::RemoveNote))
                .width(px(440.))
                .close_button(false)
                .overlay_closable(false)
                .on_cancel(move |_, _, cx| {
                    cancelling
                        .update(cx, |this, _| !this.thread_saving)
                        .unwrap_or(true)
                })
                .child(muted(language.text(Text::RemoveNoteDetail)))
                .footer(
                    row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("cancel-remove-note")
                                .ghost()
                                .label(language.text(Text::Cancel))
                                .on_click(move |_, window, cx| {
                                    if cancel_button
                                        .update(cx, |this, _| !this.thread_saving)
                                        .unwrap_or(true)
                                    {
                                        window.close_dialog(cx);
                                    }
                                }),
                        )
                        .child(
                            Button::new("confirm-remove-note")
                                .primary()
                                .label(language.text(Text::Remove))
                                .on_click(move |_, window, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.apply_thread_change(command.clone(), true, window, cx)
                                    });
                                }),
                        ),
                )
        });
    }

    fn apply_thread_change(
        &mut self,
        command: ThreadCommand,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread_saving {
            return;
        }
        self.thread_saving = true;
        let service = self.thread_service.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.apply(command) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.thread_saving = false;
                match result {
                    Ok(_) => {
                        this.refresh_threads(window, cx);
                        if close {
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => explain(this.text(Text::ThreadSaveError), error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
