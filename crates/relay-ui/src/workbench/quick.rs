use super::*;
use gpui_kit::component::Selectable;
use relay_core::{
    ProjectId,
    capture::{FetchState, QuickAction, Selection, WebFetchService, compose},
};

pub struct OpenProject(pub Option<ProjectId>);
impl EventEmitter<OpenProject> for Workbench {}

pub(super) struct QuickEntry {
    selection: Selection,
    fetch: FetchState,
    generation: u64,
    sent: bool,
    saving: bool,
    notice: Option<String>,
}

impl Workbench {
    pub fn current_project(&self) -> Option<ProjectId> {
        matches!(self.page, Page::Project(_))
            .then(|| {
                self.projects
                    .get(self.selected_project)
                    .map(|project| project.id)
            })
            .flatten()
    }
    pub fn set_quick_notice(&mut self, notice: String, cx: &mut Context<Self>) {
        if let Some(quick) = self.quick.as_mut() {
            quick.notice = Some(notice);
            cx.notify();
        }
    }

    pub fn open_project(
        &mut self,
        project: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_projects(window, cx);
        if let Some(index) = self.projects.iter().position(|p| p.id == project) {
            self.navigate(Page::Project(index), window, cx);
            self.conversation_scroll.scroll_to_bottom();
        }
    }

    pub fn capture(
        &mut self,
        selection: Selection,
        fetcher: Arc<dyn WebFetchService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.quick.as_ref().map_or(1, |quick| quick.generation + 1);
        let url = selection.url.clone();
        self.quick = Some(QuickEntry {
            selection,
            fetch: if url.is_some() {
                FetchState::Loading
            } else {
                FetchState::Skipped
            },
            generation,
            sent: false,
            saving: false,
            notice: None,
        });
        if !self.projects.is_empty() {
            self.navigate(Page::Project(self.selected_project), window, cx);
        }
        if let Some(url) = url {
            let task = cx
                .background_executor()
                .spawn(async move { fetcher.fetch(&url) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(quick) = this
                        .quick
                        .as_mut()
                        .filter(|q| q.generation == generation && !q.sent)
                    {
                        quick.fetch = match result {
                            Ok(body) => FetchState::Ready(body),
                            Err(error) => FetchState::Failed(error),
                        };
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn quick_send(
        &mut self,
        action: QuickAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.projects.is_empty() {
            return;
        }
        let quick = self.quick.as_ref().expect("quick view");
        let question = self.drafts[self.selected_project]
            .read(cx)
            .value()
            .to_string();
        if question.trim().is_empty() && (quick.sent || quick.selection.text.trim().is_empty()) {
            return;
        }
        if self.agent_states[self.selected_project].status != ConnectionStatus::Ready {
            self.picker_open = true;
            cx.notify();
            return;
        }
        let prompt = if quick.sent {
            question
        } else {
            let language = match self.settings_snapshot.language {
                Language::English => "English",
                Language::SimplifiedChinese => "简体中文",
            };
            format!(
                "默认回复语言：{language}。\n\n{}",
                compose(action, &question, &quick.selection, &quick.fetch)
            )
        };
        if self.agent_action(AgentCommand::Send(prompt), cx) {
            self.quick.as_mut().expect("quick view").sent = true;
            self.drafts[self.selected_project]
                .update(cx, |draft, cx| draft.set_value("", window, cx));
            self.conversation_scroll.scroll_to_bottom();
        }
    }

    fn quick_save(&mut self, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            return;
        }
        let project = self.projects[self.selected_project].clone();
        let quick = self.quick.as_mut().expect("quick view");
        if quick.saving {
            return;
        }
        let mut content = quick.selection.text.clone();
        if let Some(url) = &quick.selection.url {
            content.push_str(&format!("\n\n来源：{url}"));
        }
        if content.trim().is_empty() {
            return;
        }
        let name: String = quick.selection.text.chars().take(60).collect();
        let generation = quick.generation;
        quick.saving = true;
        let service = self.project_service.clone();
        let task = cx.background_executor().spawn(async move {
            service.apply(ProjectCommand::SaveContext {
                project: project.id,
                expected_revision: project.revision,
                id: None,
                name: if name.trim().is_empty() {
                    "Web selection".into()
                } else {
                    name
                },
                content,
                included: false,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let saved = this.text(Text::QuickSaved).to_owned();
                if let Some(quick) = this.quick.as_mut().filter(|q| q.generation == generation) {
                    quick.saving = false;
                    quick.notice = Some(result.map_or_else(|error| error, |_| saved));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn quick_view(&self, cx: &mut Context<Self>) -> Div {
        let quick = self.quick.as_ref().expect("quick view");
        let has_project = !self.projects.is_empty();
        let has_input = has_project
            && (!quick.selection.text.trim().is_empty()
                || !self.drafts[self.selected_project]
                    .read(cx)
                    .value()
                    .trim()
                    .is_empty());
        let busy = has_project && self.agent_states[self.selected_project].status.is_busy();
        let actions = row()
            .flex_wrap()
            .gap(px(4.))
            .children(
                [
                    (QuickAction::Search, Text::QuickSearch),
                    (QuickAction::Explain, Text::QuickExplain),
                    (QuickAction::Translate, Text::QuickTranslate),
                    (QuickAction::Summarize, Text::Summarize),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (action, label))| {
                    Button::new(("quick-action", index))
                        .ghost()
                        .small()
                        .label(self.text(label))
                        .disabled(!has_input || busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.quick_send(action, window, cx)
                        }))
                }),
            )
            .child(
                Button::new("quick-save")
                    .ghost()
                    .small()
                    .label(self.text(Text::QuickSave))
                    .disabled(!has_project || quick.selection.text.is_empty() || quick.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.quick_save(cx))),
            );
        let material = column().mx(px(16.)).my(px(10.)).p(px(12.)).gap(px(8.))
            .rounded(px(10.)).bg(rgb(0xf4f4f5))
            .child(div().id("quick-selection").max_h(px(100.)).overflow_y_scroll()
                .child(if quick.selection.text.is_empty() { self.text(Text::QuickNoSelection).to_owned() } else { quick.selection.text.clone() }))
            .when_some(quick.selection.url.clone(), |view, url| view.child(muted(url).text_size(px(11.))))
            .child(muted(self.text(match &quick.fetch {
                FetchState::Skipped => Text::QuickFetchSkipped,
                FetchState::Loading => Text::QuickFetching,
                FetchState::Ready(_) => Text::QuickFetched,
                FetchState::Failed(_) => Text::QuickFetchFailed,
            })).text_size(px(11.)))
            .when(quick.selection.accessibility_missing, |view| view.child(
                Button::new("quick-accessibility").ghost().small().label(self.text(Text::QuickAccessibility))
                    .on_click(|_, _, cx| cx.open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"))))
            .child(actions);
        column()
            .size_full()
            .min_h_0()
            .child(
                row()
                    .justify_between()
                    .p(px(14.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Relay"))
                    .child(
                        Button::new("quick-expand")
                            .ghost()
                            .small()
                            .label(self.text(Text::QuickExpand))
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.minimize_window();
                                cx.emit(OpenProject(
                                    this.projects.get(this.selected_project).map(|p| p.id),
                                ));
                            })),
                    ),
            )
            .child(row().px(px(16.)).gap(px(6.)).flex_wrap().children(
                self.projects.iter().enumerate().map(|(index, project)| {
                    Button::new(("quick-project", index))
                        .ghost()
                        .small()
                        .label(project.name.clone())
                        .selected(index == self.selected_project)
                        .disabled(quick.sent)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate(Page::Project(index), window, cx)
                        }))
                }),
            ))
            .when(!has_project, |view| {
                view.child(div().p(px(16.)).child(self.text(Text::QuickNoProject)))
            })
            .when(!quick.sent, |view| view.child(material))
            .when_some(quick.notice.clone(), |view, notice| {
                view.child(div().px(px(16.)).text_size(px(12.)).child(notice))
            })
            .when(has_project, |view| view.child(self.conversation(true, cx)))
    }
}
