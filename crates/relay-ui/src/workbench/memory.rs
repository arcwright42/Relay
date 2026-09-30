use super::projects::editor::EditorMode;
use super::*;
use relay_core::{MemoryItem, MemoryKind, MemorySource, ProjectId};

struct MemoryLibrary {
    workbench: WeakEntity<Workbench>,
    project: ProjectId,
}

impl Render for MemoryLibrary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.workbench
            .update(cx, |workbench, cx| {
                workbench.memory_library(self.project, cx)
            })
            .unwrap_or_else(|_| column())
    }
}

impl Workbench {
    pub(super) fn show_project_memory(
        &self,
        id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workbench = cx.entity().downgrade();
        let library = cx.new(|_| MemoryLibrary {
            workbench,
            project: id,
        });
        let title = self.text(Text::ProjectMemory);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(title).width(px(640.)).child(library.clone())
        });
    }

    pub(super) fn remember_reply(
        &mut self,
        message: ChatMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if message.role != MessageRole::Assistant
            || message.status != MessageStatus::Complete
            || message.text.trim().is_empty()
        {
            return;
        }
        let Some(project) = self.projects.get(self.selected_project).cloned() else {
            return;
        };
        self.open_editor(
            EditorMode::Memory {
                project,
                item: None,
                message: Some(Box::new(message)),
            },
            window,
            cx,
        );
    }

    fn edit_memory(
        &mut self,
        project: Project,
        item: Option<MemoryItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor(
            EditorMode::Memory {
                project,
                item,
                message: None,
            },
            window,
            cx,
        );
    }

    fn memory_library(&self, id: ProjectId, cx: &mut Context<Self>) -> Div {
        let Some(project) = self.projects.iter().find(|p| p.id == id) else {
            return column();
        };
        let add_project = project.clone();
        column()
            .gap(px(16.))
            .child(muted(self.text(Text::MemoryHint)).text_size(px(12.)))
            .child(
                column()
                    .id("project-memory-list")
                    .max_h(px(340.))
                    .overflow_y_scroll()
                    .gap(px(8.))
                    .when(project.memory.is_empty(), |view| {
                        view.child(muted(self.text(Text::NoMemory)).py(px(24.)))
                    })
                    .children(project.memory.iter().map(|item| {
                        let memory = item.clone();
                        let editing_project = project.clone();
                        let remove = ProjectCommand::RemoveMemory {
                            project: id,
                            expected_revision: project.revision,
                            id: item.id,
                        };
                        let source = match &item.source {
                            Some(MemorySource::Message { message_id }) => {
                                format!("{} #{message_id}", self.text(Text::MemoryFromReply))
                            }
                            None => self.text(Text::MemoryManual).into(),
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
                                    .child(
                                        muted(self.text(match item.kind {
                                            MemoryKind::Fact => Text::MemoryFact,
                                            MemoryKind::Decision => Text::MemoryDecision,
                                        }))
                                        .text_size(px(12.)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(item.name.clone()),
                                    )
                                    .child(
                                        Button::new(("edit-memory", item.id.0))
                                            .ghost()
                                            .small()
                                            .label(self.text(Text::Edit))
                                            .disabled(self.project_saving)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.edit_memory(
                                                    editing_project.clone(),
                                                    Some(memory.clone()),
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(("remove-memory", item.id.0))
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
                                muted(item.content.chars().take(220).collect::<String>())
                                    .text_size(px(12.)),
                            )
                            .child(muted(source).text_size(px(11.)))
                    })),
            )
            .child(
                Button::new("add-project-memory")
                    .primary()
                    .icon(IconName::Plus)
                    .label(self.text(Text::AddMemory))
                    .disabled(self.project_saving)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_memory(add_project.clone(), None, window, cx)
                    })),
            )
    }
}
