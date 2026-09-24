mod editor;
use super::*;
use editor::{EditorMode, ProjectEditor};
use relay_core::{ContextItem, ProjectId};
use std::collections::BTreeMap;

struct ContextLibrary {
    workbench: WeakEntity<Workbench>,
    project: ProjectId,
}

impl Render for ContextLibrary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.workbench
            .update(cx, |workbench, cx| {
                workbench.context_library(self.project, cx)
            })
            .unwrap_or_else(|_| column())
    }
}

impl Workbench {
    pub(super) fn refresh_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_service.revision() == self.project_revision {
            return;
        }
        let catalog = self.project_service.snapshot();
        let selected = self.projects.get(self.selected_project).map(|p| p.id);
        let mut drafts: BTreeMap<_, _> = self
            .projects
            .iter()
            .map(|p| p.id)
            .zip(std::mem::take(&mut self.drafts))
            .collect();
        let mut errors: BTreeMap<_, _> = self
            .projects
            .iter()
            .map(|p| p.id)
            .zip(std::mem::take(&mut self.agent_errors))
            .collect();
        self.project_revision = catalog.revision;
        self.project_error = catalog.error;
        self.projects = catalog.projects;
        let placeholder = self.text(Text::AskRelay);
        for project in &self.projects {
            let draft = drafts.remove(&project.id).unwrap_or_else(|| {
                let draft = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder(placeholder)
                        .auto_grow(2, 6)
                });
                self._subscriptions
                    .push(cx.subscribe_in(&draft, window, |_, _, _, _, cx| cx.notify()));
                draft
            });
            self.drafts.push(draft);
            self.agent_errors.push(errors.remove(&project.id).flatten());
        }
        self.selected_project = self
            .projects
            .iter()
            .position(|p| Some(p.id) == selected)
            .unwrap_or(0);
        if matches!(self.page, Page::Project(_)) {
            self.page = if self.projects.is_empty() {
                Page::Home
            } else {
                Page::Project(self.selected_project)
            };
        }
        self.agent_states = self
            .projects
            .iter()
            .map(|p| self.agent_service.snapshot(p.id))
            .collect();
        self.agent_revision = self.agent_service.revision();
        cx.notify();
    }

    pub(super) fn edit_project(
        &mut self,
        project: Option<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor(EditorMode::Project(project), window, cx);
    }

    fn edit_note(
        &mut self,
        project: Project,
        item: Option<ContextItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor(EditorMode::Note { project, item }, window, cx);
    }

    fn open_editor(&mut self, mode: EditorMode, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let editor = cx.new(|cx| {
            ProjectEditor::new(
                mode,
                self.settings_snapshot.language,
                self.project_service.clone(),
                Box::new(move |id, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.refresh_projects(window, cx);
                        if let Some(index) = this.projects.iter().position(|p| p.id == id) {
                            this.navigate(Page::Project(index), window, cx);
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
        let Some(project) = self.projects.get(self.selected_project) else {
            return;
        };
        let id = project.id;
        let workbench = cx.entity().downgrade();
        let library = cx.new(|_| ContextLibrary {
            workbench,
            project: id,
        });
        let title = self.text(Text::Context);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(title).width(px(640.)).child(library.clone())
        });
    }

    fn context_library(&self, id: ProjectId, cx: &mut Context<Self>) -> Div {
        let Some(project) = self.projects.iter().find(|p| p.id == id) else {
            return column();
        };
        let add_project = project.clone();
        let edit_project = project.clone();
        column()
            .gap(px(16.))
            .child(muted(self.text(Text::ContextIndependent)).text_size(px(13.)))
            .child(muted(self.text(Text::ContextSyncDetail)).text_size(px(12.)))
            .child(
                Button::new("context-project-instructions")
                    .outline()
                    .justify_start()
                    .icon(IconName::Settings)
                    .label(self.text(Text::ProjectInstructions))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_project(Some(edit_project.clone()), window, cx)
                    })),
            )
            .child(
                column()
                    .id("project-context-list")
                    .max_h(px(340.))
                    .overflow_y_scroll()
                    .gap(px(8.))
                    .when(project.context.is_empty(), |view| {
                        view.child(muted(self.text(Text::NoContext)).py(px(24.)))
                    })
                    .children(project.context.iter().map(|item| {
                        let note = item.clone();
                        let editing_project = project.clone();
                        let toggle = ProjectCommand::SaveContext {
                            project: id,
                            expected_revision: project.revision,
                            id: Some(item.id),
                            name: item.name.clone(),
                            content: item.content.clone(),
                            included: !item.included,
                        };
                        let remove = ProjectCommand::RemoveContext {
                            project: id,
                            expected_revision: project.revision,
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
                                            .disabled(self.project_saving)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.apply_project_change(
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
                                                    editing_project.clone(),
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
                                            .disabled(self.project_saving)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.confirm_remove(remove.clone(), window, cx)
                                            })),
                                    ),
                            )
                            .child(
                                muted(item.content.chars().take(100).collect::<String>())
                                    .text_size(px(12.)),
                            )
                    })),
            )
            .child(
                Button::new("add-context-note")
                    .primary()
                    .icon(IconName::Plus)
                    .label(self.text(Text::AddNote))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_note(add_project.clone(), None, window, cx)
                    })),
            )
    }

    fn confirm_remove(&self, command: ProjectCommand, window: &mut Window, cx: &mut Context<Self>) {
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
                        .update(cx, |this, _| !this.project_saving)
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
                                        .update(cx, |this, _| !this.project_saving)
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
                                        this.apply_project_change(command.clone(), true, window, cx)
                                    });
                                }),
                        ),
                )
        });
    }

    fn apply_project_change(
        &mut self,
        command: ProjectCommand,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project_saving {
            return;
        }
        self.project_saving = true;
        let service = self.project_service.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.apply(command) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.project_saving = false;
                match result {
                    Ok(_) => {
                        this.refresh_projects(window, cx);
                        if close {
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => explain(this.text(Text::ProjectSaveError), error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
