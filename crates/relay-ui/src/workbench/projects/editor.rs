use super::super::*;
use relay_core::{
    ContextItem, MemoryItem, MemoryKind, MemorySource, ProjectId, projects::ProjectDraft,
};

pub(in super::super) enum EditorMode {
    Project(Option<Project>),
    Note {
        project: Project,
        item: Option<ContextItem>,
    },
    Memory {
        project: Project,
        item: Option<MemoryItem>,
        message: Option<Box<ChatMessage>>,
    },
}

type OnSaved = Box<dyn Fn(ProjectId, &mut Window, &mut App)>;

pub(super) struct ProjectEditor {
    mode: EditorMode,
    language: Language,
    service: Arc<dyn ProjectService>,
    name: Entity<InputState>,
    description: Entity<InputState>,
    body: Entity<TextareaState>,
    included: bool,
    memory_kind: MemoryKind,
    pub saving: bool,
    error: Option<String>,
    on_saved: OnSaved,
}

impl ProjectEditor {
    pub fn new(
        mode: EditorMode,
        language: Language,
        service: Arc<dyn ProjectService>,
        on_saved: OnSaved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (name, description, body, included) = match &mode {
            EditorMode::Project(project) => project
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
            EditorMode::Memory { item, message, .. } => {
                if let Some(item) = item {
                    (item.name.clone(), String::new(), item.content.clone(), true)
                } else if let Some(message) = message {
                    let name = message
                        .text
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("")
                        .trim()
                        .trim_start_matches('#')
                        .trim()
                        .chars()
                        .take(60)
                        .collect();
                    (name, String::new(), message.text.clone(), true)
                } else {
                    (String::new(), String::new(), String::new(), true)
                }
            }
        };
        let memory_kind = match &mode {
            EditorMode::Memory {
                item: Some(item), ..
            } => item.kind,
            _ => MemoryKind::Fact,
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
            memory_kind,
            saving: false,
            error: None,
            on_saved,
        }
    }

    pub fn title(&self) -> &'static str {
        self.language.text(match self.mode {
            EditorMode::Project(None) => Text::NewProject,
            EditorMode::Project(Some(_)) => Text::EditProject,
            EditorMode::Note { item: None, .. } => Text::AddNote,
            EditorMode::Note { item: Some(_), .. } => Text::EditNote,
            EditorMode::Memory { item: None, .. } => Text::AddMemory,
            EditorMode::Memory { item: Some(_), .. } => Text::EditMemory,
        })
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.name.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        let command = match &self.mode {
            EditorMode::Project(project) => {
                let draft = ProjectDraft {
                    name,
                    description: self.description.read(cx).value().to_string(),
                    instructions: body,
                };
                if let Some(project) = project {
                    ProjectCommand::Edit {
                        project: project.id,
                        expected_revision: project.revision,
                        draft,
                    }
                } else {
                    ProjectCommand::Create(draft)
                }
            }
            EditorMode::Note { project, item } => ProjectCommand::SaveContext {
                project: project.id,
                expected_revision: project.revision,
                id: item.as_ref().map(|item| item.id),
                name,
                content: body,
                included: self.included,
            },
            EditorMode::Memory {
                project,
                item,
                message,
            } => ProjectCommand::SaveMemory {
                project: project.id,
                expected_revision: project.revision,
                id: item.as_ref().map(|item| item.id),
                kind: self.memory_kind,
                name,
                content: body,
                source: item
                    .as_ref()
                    .and_then(|item| item.source.clone())
                    .or_else(|| {
                        message.as_ref().map(|message| MemorySource::Message {
                            message_id: message.id,
                        })
                    }),
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

impl Render for ProjectEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let project = matches!(self.mode, EditorMode::Project(_));
        let memory = matches!(self.mode, EditorMode::Memory { .. });
        let body_label = self.language.text(if project {
            Text::ProjectInstructions
        } else {
            Text::NoteContent
        });
        column()
            .gap(px(13.))
            .when(memory, |view| {
                let source = match &self.mode {
                    EditorMode::Memory {
                        item: Some(item), ..
                    } => item.source.clone(),
                    EditorMode::Memory {
                        message: Some(message),
                        ..
                    } => Some(MemorySource::Message {
                        message_id: message.id,
                    }),
                    _ => None,
                };
                let label = match source {
                    Some(MemorySource::Message { message_id }) => format!(
                        "{} #{message_id}",
                        self.language.text(Text::MemoryFromReply)
                    ),
                    None => self.language.text(Text::MemoryManual).into(),
                };
                view.child(
                    row().gap(px(8.)).children(
                        [
                            (MemoryKind::Fact, Text::MemoryFact, "memory-kind-fact"),
                            (
                                MemoryKind::Decision,
                                Text::MemoryDecision,
                                "memory-kind-decision",
                            ),
                        ]
                        .into_iter()
                        .map(|(kind, text, id)| {
                            Button::new(id)
                                .outline()
                                .small()
                                .label(self.language.text(text))
                                .when(self.memory_kind == kind, |button| button.bg(rgb(0xeaeaec)))
                                .disabled(self.saving)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.memory_kind = kind;
                                    cx.notify();
                                }))
                        }),
                    ),
                )
                .child(muted(label).text_size(px(11.)))
            })
            .child(muted(self.language.text(Text::Name)).text_size(px(12.)))
            .child(
                Input::new(&self.name)
                    .id("project-editor-name")
                    .disabled(self.saving)
                    .aria_label(self.language.text(Text::Name))
                    .context_menu(crate::locale::input_menu),
            )
            .when(project, |view| {
                view.child(muted(self.language.text(Text::Description)).text_size(px(12.)))
                    .child(
                        Input::new(&self.description)
                            .id("project-editor-description")
                            .disabled(self.saving)
                            .aria_label(self.language.text(Text::Description))
                            .context_menu(crate::locale::input_menu),
                    )
            })
            .child(muted(body_label).text_size(px(12.)))
            .child(
                column().id("project-editor-body").test_support().child(
                    Textarea::new(&self.body)
                        .h(px(200.))
                        .disabled(self.saving)
                        .aria_label(body_label)
                        .context_menu(crate::locale::input_menu),
                ),
            )
            .child(
                muted(self.language.text(if project {
                    Text::InstructionsHint
                } else if memory {
                    Text::MemoryEditorHint
                } else {
                    Text::NoteHint
                }))
                .text_size(px(12.)),
            )
            .when(!project && !memory, |view| {
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
                                .child(self.language.text(Text::ProjectSaveError)),
                        )
                        .child(muted(error.clone()).text_size(px(12.))),
                )
            })
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("cancel-project-edit")
                            .ghost()
                            .label(self.language.text(Text::Cancel))
                            .disabled(self.saving)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save-project-edit")
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
