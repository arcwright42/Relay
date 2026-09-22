use super::super::*;
use relay_core::{ContextItem, ProjectId, projects::ProjectDraft};

pub(super) enum EditorMode {
    Project(Option<Project>),
    Note {
        project: Project,
        item: Option<ContextItem>,
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
                        p.name.as_str(),
                        p.description.as_str(),
                        p.instructions.as_str(),
                        true,
                    )
                })
                .unwrap_or(("", "", "", true)),
            EditorMode::Note { item, .. } => item
                .as_ref()
                .map(|item| (item.name.as_str(), "", item.content.as_str(), item.included))
                .unwrap_or(("", "", "", true)),
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
            EditorMode::Project(None) => Text::NewProject,
            EditorMode::Project(Some(_)) => Text::EditProject,
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
        let body_label = self.language.text(if project {
            Text::ProjectInstructions
        } else {
            Text::NoteContent
        });
        column()
            .gap(px(13.))
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
                } else {
                    Text::NoteHint
                }))
                .text_size(px(12.)),
            )
            .when(!project, |view| {
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
