use super::*;
use gpui_kit::component::{Selectable, popover::Popover};
use relay_core::{
    ProjectId,
    capture::{FetchState, QuickAction, Selection, WebFetchService, compose},
};

pub struct OpenProject(pub Option<ProjectId>);
impl EventEmitter<OpenProject> for Workbench {}
pub struct RequestAccessibility;
impl EventEmitter<RequestAccessibility> for Workbench {}

pub(super) struct QuickEntry {
    selection: Selection,
    fetch: FetchState,
    generation: u64,
    sent: bool,
    saving: bool,
    notice: Option<String>,
    action: Option<QuickAction>,
    more: bool,
    translation_target: &'static str,
    target_open: bool,
    message_start: usize,
    questions: Vec<(usize, String)>,
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
            action: None,
            more: false,
            translation_target: if self.settings_snapshot.language == Language::English {
                "English"
            } else {
                "中文（简体）"
            },
            target_open: false,
            message_start: 0,
            questions: Vec::new(),
        });
        let placeholder = if self.quick.as_ref().unwrap().selection.text.is_empty() {
            "未读取选区 · 粘贴或提问"
        } else {
            "问问 Relay"
        };
        for draft in &self.drafts {
            draft.update(cx, |draft, cx| {
                draft.set_placeholder(placeholder, window, cx);
                draft.set_submit_on_enter(true, cx)
            });
        }
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
        if self.agent_states[self.selected_project].status.is_busy() {
            return;
        }
        let quick = self.quick.as_ref().expect("quick view");
        let action = if quick.sent {
            action
        } else {
            quick.action.unwrap_or(action)
        };
        let question = self.drafts[self.selected_project]
            .read(cx)
            .value()
            .to_string();
        if question.trim().is_empty() && (quick.sent || quick.selection.text.trim().is_empty()) {
            return;
        }
        if self.quick.as_ref().unwrap().action.is_none() {
            self.quick.as_mut().unwrap().action = Some(action);
            window.resize(size(
                px(if action == QuickAction::Search {
                    520.
                } else {
                    480.
                }),
                px(match action {
                    QuickAction::Explain => 360.,
                    QuickAction::Translate => 460.,
                    _ => 580.,
                }),
            ));
        }
        if self.agent_states[self.selected_project].status != ConnectionStatus::Ready {
            self.picker_open = true;
            cx.notify();
            return;
        }
        let quick = self.quick.as_ref().unwrap();
        let first = !quick.sent;
        let message_start = self.agent_states[self.selected_project].messages.len();
        let display_question = question.clone();
        let prompt = if quick.sent {
            question
        } else {
            let language = if action == QuickAction::Translate {
                quick.translation_target
            } else {
                match self.settings_snapshot.language {
                    Language::English => "English",
                    Language::SimplifiedChinese => "简体中文",
                }
            };
            format!(
                "默认回复语言：{language}。\n\n{}",
                compose(action, &question, &quick.selection, &quick.fetch)
            )
        };
        if self.agent_action(AgentCommand::Send(prompt), cx) {
            let quick = self.quick.as_mut().expect("quick view");
            quick.sent = true;
            if first {
                quick.message_start = message_start;
            }
            if !display_question.trim().is_empty() {
                quick.questions.push((message_start, display_question));
            }
            self.drafts[self.selected_project].update(cx, |draft, cx| {
                draft.set_value("", window, cx);
                draft.set_placeholder("继续提问", window, cx);
            });
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

    fn translation_picker(&self, cx: &mut Context<Self>) -> Popover {
        let quick = self.quick.as_ref().unwrap();
        let weak = cx.entity().downgrade();
        Popover::new("quick-language-menu")
            .anchor(Anchor::TopRight)
            .open(quick.target_open)
            .on_open_change(cx.listener(|this, open, _, cx| {
                this.quick.as_mut().unwrap().target_open = *open;
                cx.notify();
            }))
            .trigger(
                Button::new("quick-language")
                    .ghost()
                    .small()
                    .label(quick.translation_target)
                    .icon(IconName::ChevronDown)
                    .disabled(
                        self.agent_states[self.selected_project].status != ConnectionStatus::Ready,
                    ),
            )
            .content(move |_, _, cx| {
                weak.update(cx, |_, cx| {
                    column().w(px(170.)).gap(px(4.)).p(px(6.)).children(
                        [
                            "中文（简体）",
                            "English",
                            "日本語",
                            "한국어",
                            "Français",
                            "Deutsch",
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, language)| {
                            Button::new(("quick-target", index))
                                .ghost()
                                .small()
                                .label(language)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let quick = this.quick.as_mut().unwrap();
                                    quick.target_open = false;
                                    quick.translation_target = language;
                                    if !quick.sent {
                                        this.quick_send(QuickAction::Translate, window, cx);
                                        return;
                                    }
                                    let prompt = format!(
                                        "目标语言：{language}。\n\n{}",
                                        compose(
                                            QuickAction::Translate,
                                            "",
                                            &quick.selection,
                                            &quick.fetch
                                        )
                                    );
                                    let start =
                                        this.agent_states[this.selected_project].messages.len();
                                    if this.agent_states[this.selected_project].status
                                        == ConnectionStatus::Ready
                                        && this.agent_action(AgentCommand::Send(prompt), cx)
                                    {
                                        let quick = this.quick.as_mut().unwrap();
                                        quick.message_start = start;
                                        quick.questions.clear();
                                    }
                                    cx.notify();
                                }))
                        }),
                    )
                })
                .unwrap_or_else(|_| column())
            })
    }

    fn quick_input(&self, cx: &mut Context<Self>) -> Div {
        row()
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(12.))
            .bg(rgba(0xffffffbb))
            .border_1()
            .border_color(rgba(0x00000012))
            .when(!self.projects.is_empty(), |view| {
                view.child(
                    div().flex_1().min_w_0().child(
                        Textarea::new(&self.drafts[self.selected_project])
                            .appearance(false)
                            .bordered(false)
                            .h(px(28.))
                            .text_size(px(14.))
                            .aria_label("问问 Relay")
                            .context_menu(crate::locale::input_menu),
                    ),
                )
                .child(
                    Button::new("quick-ask")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowUp)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.quick_send(QuickAction::Ask, window, cx)
                        })),
                )
            })
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
        let close = Button::new("quick-close")
            .ghost()
            .small()
            .icon(IconName::X)
            .on_click(|_, window, _| window.remove_window());
        let expand = Button::new("quick-expand")
            .ghost()
            .small()
            .label(self.text(Text::QuickExpand))
            .on_click(cx.listener(|this, _, window, cx| {
                cx.emit(OpenProject(
                    this.projects.get(this.selected_project).map(|p| p.id),
                ));
                window.minimize_window();
            }));
        let surface = column()
            .size_full()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(rgba(0x00000020))
            .bg(rgba(0xeeeff2bd));
        if quick.action.is_none() {
            return surface
                .child(
                    row()
                        .h(px(52.))
                        .flex_shrink_0()
                        .px(px(8.))
                        .gap(px(3.))
                        .child(
                            div()
                                .id("quick-drag")
                                .px(px(5.))
                                .cursor_move()
                                .text_color(rgb(0x99999f))
                                .child("⠿")
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.start_window_move()
                                }),
                        )
                        .child(div().px(px(7.)).font_weight(FontWeight::BOLD).child("R"))
                        .children(
                            [
                                (QuickAction::Search, Text::QuickSearch),
                                (QuickAction::Explain, Text::QuickExplain),
                                (QuickAction::Translate, Text::QuickTranslate),
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
                            Button::new("quick-more")
                                .ghost()
                                .small()
                                .icon(IconName::ChevronDown)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let quick = this.quick.as_mut().unwrap();
                                    quick.more = !quick.more;
                                    window.resize(size(
                                        px(640.),
                                        px(if quick.more { 310. } else { 54. }),
                                    ));
                                    cx.notify();
                                })),
                        )
                        .when(quick.selection.accessibility_missing, |view| {
                            view.child(
                                Button::new("quick-permission")
                                    .ghost()
                                    .small()
                                    .label("授权读取")
                                    .tooltip(self.text(Text::QuickAccessibility))
                                    .on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(RequestAccessibility)),
                                    ),
                            )
                        })
                        .child(self.quick_input(cx))
                        .child(close),
                )
                .when(quick.more, |view| {
                    view.child(
                        column()
                            .p(px(12.))
                            .gap(px(10.))
                            .child(row().gap(px(5.)).flex_wrap().children(
                                self.projects.iter().enumerate().map(|(index, project)| {
                                    Button::new(("quick-project", index))
                                        .ghost()
                                        .small()
                                        .label(project.name.clone())
                                        .selected(index == self.selected_project)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.navigate(Page::Project(index), window, cx)
                                        }))
                                }),
                            ))
                            .child(
                                row()
                                    .gap(px(6.))
                                    .child(
                                        Button::new("quick-save")
                                            .ghost()
                                            .small()
                                            .label(self.text(Text::QuickSave))
                                            .disabled(!has_input || quick.saving)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.quick_save(cx)),
                                            ),
                                    )
                                    .when(quick.selection.accessibility_missing, |view| {
                                        view.child(
                                            Button::new("quick-accessibility")
                                                .ghost()
                                                .small()
                                                .label(self.text(Text::QuickAccessibility))
                                                .on_click(cx.listener(|_, _, _, cx| {
                                                    cx.emit(RequestAccessibility)
                                                })),
                                        )
                                    })
                                    .when(!has_project, |view| {
                                        view.child(muted(self.text(Text::QuickNoProject)))
                                    })
                                    .when(quick.selection.text.is_empty(), |view| {
                                        view.child(
                                            muted(self.text(Text::QuickNoSelection))
                                                .text_size(px(12.)),
                                        )
                                    })
                                    .child(
                                        muted(self.text(match quick.fetch {
                                            FetchState::Skipped => Text::QuickFetchSkipped,
                                            FetchState::Loading => Text::QuickFetching,
                                            FetchState::Ready(_) => Text::QuickFetched,
                                            FetchState::Failed(_) => Text::QuickFetchFailed,
                                        }))
                                        .text_size(px(11.)),
                                    )
                                    .when_some(quick.notice.clone(), |view, text| {
                                        view.child(muted(text))
                                    })
                                    .child(expand),
                            ),
                    )
                });
        }
        let action = quick.action.unwrap();
        let title = match action {
            QuickAction::Search => "AI 搜索",
            QuickAction::Explain => "解释",
            QuickAction::Translate => "翻译",
            QuickAction::Summarize => "总结",
            QuickAction::Ask => "问问 Relay",
        };
        let mut body = column()
            .id("quick-response")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.conversation_scroll)
            .px(px(20.))
            .pb(px(12.))
            .gap(px(14.));
        if action == QuickAction::Translate {
            body = body.child(
                row()
                    .justify_between()
                    .p(px(10.))
                    .rounded(px(10.))
                    .bg(rgba(0xffffff80))
                    .child("自动检测")
                    .child("→")
                    .child(self.translation_picker(cx)),
            );
        }
        if quick.selection.text.is_empty() {
            body = body.child(div().p(px(10.)).rounded(px(8.)).bg(rgba(0xffe5bfaa))
                .text_size(px(12.)).child("未读取到选中文字。当前问题不会自动获得你正在看的内容，请粘贴材料或重新选中后按快捷键。"));
        }
        if !quick.selection.text.is_empty() {
            body = body.child(
                div()
                    .id("quick-selection")
                    .max_h(px(88.))
                    .overflow_y_scroll()
                    .border_l_2()
                    .border_color(rgb(0xb6b7bc))
                    .pl(px(10.))
                    .text_color(rgb(0x777981))
                    .text_size(px(13.))
                    .child(quick.selection.text.clone()),
            );
        }
        if has_project {
            let state = &self.agent_states[self.selected_project];
            for (index, message) in state
                .messages
                .iter()
                .enumerate()
                .skip(quick.message_start)
                .filter(|_| quick.sent)
            {
                if message.role == MessageRole::User {
                    if let Some((_, question)) = quick
                        .questions
                        .iter()
                        .find(|(position, _)| *position == index)
                    {
                        body = body.child(
                            div()
                                .rounded(px(10.))
                                .p(px(10.))
                                .bg(rgba(0xffffff70))
                                .child(question.clone()),
                        );
                    }
                    continue;
                }
                body = body
                    .child(
                        gpui_kit::component::text::TextView::markdown(
                            ("quick-answer", message.id),
                            message.text.clone(),
                        )
                        .selectable(true),
                    )
                    .children(
                        message
                            .tools
                            .iter()
                            .map(|tool| muted(tool.title.clone()).text_size(px(12.))),
                    );
            }
            if quick.sent
                && let Some(message) = state
                    .messages
                    .iter()
                    .skip(quick.message_start)
                    .rev()
                    .find(|m| m.role == MessageRole::Assistant && !m.text.is_empty())
            {
                let answer = message.text.clone();
                body = body.child(
                    Button::new("quick-copy")
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .label("复制")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(answer.clone()))
                        }),
                );
            }
            if busy {
                body = body.child(muted("正在处理…").text_size(px(12.)));
            }
            if !quick.sent {
                body = body.child(muted("连接 Agent 后点击继续。")).child(
                    Button::new("quick-continue")
                        .small()
                        .label("继续")
                        .disabled(state.status != ConnectionStatus::Ready)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.quick_send(action, window, cx)
                        })),
                );
            }
            body = body
                .child(self.connection_notice(cx))
                .child(self.permission_cards(cx));
        }
        surface
            .child(
                row()
                    .flex_shrink_0()
                    .h(px(54.))
                    .px(px(18.))
                    .gap(px(6.))
                    .child(
                        div()
                            .id("quick-title")
                            .flex_1()
                            .cursor_move()
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(17.))
                            .child(title)
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.start_window_move()
                            }),
                    )
                    .child(expand)
                    .child(close),
            )
            .child(body)
            .when(has_project, |view| {
                view.child(
                    row()
                        .px(px(16.))
                        .pb(px(6.))
                        .justify_between()
                        .child(
                            muted(self.projects[self.selected_project].name.clone())
                                .text_size(px(11.)),
                        )
                        .child(self.agent_picker(cx))
                        .when(busy, |view| {
                            view.child(
                                Button::new("quick-stop")
                                    .ghost()
                                    .small()
                                    .label("停止")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.agent_action(AgentCommand::Cancel, cx);
                                    })),
                            )
                        }),
                )
            })
            .child(row().flex_shrink_0().p(px(14.)).child(self.quick_input(cx)))
    }
}
