use super::*;
use gpui_kit::component::popover::Popover;

impl Workbench {
    pub(super) fn refresh_agents(&mut self, cx: &mut Context<Self>) {
        let revision = self.agent_service.revision();
        if revision == self.agent_revision {
            return;
        }
        let follow = self.conversation_scroll.offset().y
            <= -self.conversation_scroll.max_offset().y + px(100.);
        self.agent_revision = revision;
        self.agent_states = self
            .projects
            .iter()
            .map(|p| self.agent_service.snapshot(p.id))
            .collect();
        if follow {
            self.conversation_scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub(super) fn agent_action(&mut self, command: AgentCommand, cx: &mut Context<Self>) -> bool {
        let result = self
            .agent_service
            .dispatch(self.projects[self.selected_project].id, command);
        self.agent_errors[self.selected_project] = result.as_ref().err().cloned();
        self.refresh_agents(cx);
        cx.notify();
        result.is_ok()
    }

    pub(super) fn agent_picker(&self, cx: &mut Context<Self>) -> Popover {
        let state = &self.agent_states[self.selected_project];
        let label = state
            .model()
            .map(|model| format!("Codex · {}", model.current_name()))
            .unwrap_or_else(|| "Codex".into());
        let weak = cx.entity().downgrade();
        Popover::new("harness-model-picker")
            .anchor(Anchor::BottomRight)
            .open(self.picker_open)
            .on_open_change(cx.listener(|this, open, _, cx| {
                this.picker_open = *open;
                cx.notify();
            }))
            .trigger(
                Button::new("agent-picker")
                    .ghost()
                    .small()
                    .label(label.clone())
                    .icon(icon(IconName::Sparkles).size(px(14.)))
                    .child(icon(IconName::ChevronDown).size(px(12.)))
                    .text_size(px(12.))
                    .text_color(rgb(0x696970))
                    .accessibility_label(format!("Choose harness and model: {label}")),
            )
            .content(move |_, _, cx| {
                weak.update(cx, |this, cx| this.agent_menu(cx))
                    .unwrap_or_else(|_| column())
            })
    }

    fn agent_menu(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_states[self.selected_project];
        let ready = state.status == ConnectionStatus::Ready && state.pending_config.is_none();
        let mut configs: Vec<_> = state.configs.iter().collect();
        configs.sort_by_key(|config| config.category.as_deref() != Some("model"));
        column()
            .w(px(350.))
            .gap(px(8.))
            .p(px(8.))
            .child(muted("HARNESS").text_size(px(10.)).px(px(8.)).pt(px(4.)))
            .child(
                row()
                    .gap(px(12.))
                    .p(px(10.))
                    .rounded(px(10.))
                    .bg(rgb(0xf4f4f5))
                    .child(icon(IconName::Sparkles).size(px(21.)))
                    .child(
                        column()
                            .flex_1()
                            .gap(px(4.))
                            .child(div().font_weight(FontWeight::MEDIUM).child("Codex"))
                            .child(
                                muted(match state.source {
                                    AgentSource::Managed => "Managed by Relay",
                                    AgentSource::Local(_) => "Local installation",
                                })
                                .text_size(px(11.)),
                            ),
                    )
                    .child(icon(IconName::Check).size(px(16.))),
            )
            .child(
                muted(state.status.label().to_owned())
                    .text_size(px(12.))
                    .px(px(8.)),
            )
            .when(
                matches!(
                    state.status,
                    ConnectionStatus::Disconnected | ConnectionStatus::Failed
                ),
                |view| view.child(self.connect_button("picker-connect", cx).w_full()),
            )
            .when(
                state.status == ConnectionStatus::NeedsAuthentication,
                |view| view.children(self.authentication_buttons(cx)),
            )
            .when(
                state.status.is_busy()
                    && !matches!(
                        state.status,
                        ConnectionStatus::Running | ConnectionStatus::Cancelling
                    ),
                |view| {
                    view.child(
                        Button::new("picker-cancel-setup")
                            .ghost()
                            .label("Cancel setup")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.agent_action(AgentCommand::Disconnect, cx);
                            })),
                    )
                },
            )
            .child(
                column()
                    .id("agent-model-options")
                    .max_h(px(355.))
                    .overflow_y_scroll()
                    .gap(px(4.))
                    .children(
                        configs
                            .into_iter()
                            .enumerate()
                            .map(|(config_index, config)| {
                                column()
                                    .gap(px(3.))
                                    .pt(px(8.))
                                    .child(
                                        muted(config.name.clone())
                                            .text_size(px(11.))
                                            .px(px(8.))
                                            .pb(px(4.)),
                                    )
                                    .children(config.choices.iter().enumerate().map(
                                        |(index, choice)| {
                                            let id = config.id.clone();
                                            let value = choice.id.clone();
                                            let selected = config.current == choice.id;
                                            Button::new((
                                                "agent-config-choice",
                                                config_index * 10_000 + index,
                                            ))
                                            .ghost()
                                            .w_full()
                                            .h_auto()
                                            .min_h(px(36.))
                                            .justify_start()
                                            .px(px(10.))
                                            .py(px(8.))
                                            .rounded(px(8.))
                                            .disabled(!ready)
                                            .accessibility_label(format!(
                                                "{}: {}",
                                                config.name, choice.name
                                            ))
                                            .when_some(
                                                choice.description.clone(),
                                                |button, description| button.tooltip(description),
                                            )
                                            .when(selected, |button| button.bg(rgb(0xf1f1f3)))
                                            .child(
                                                row()
                                                    .w_full()
                                                    .gap(px(8.))
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .text_size(px(13.))
                                                            .child(choice.name.clone()),
                                                    )
                                                    .when(selected, |view| {
                                                        view.child(
                                                            icon(IconName::Check).size(px(14.)),
                                                        )
                                                    }),
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    if this.agent_action(
                                                        AgentCommand::SetConfig {
                                                            id: id.clone(),
                                                            value: value.clone(),
                                                        },
                                                        cx,
                                                    ) {
                                                        this.picker_open = false;
                                                        cx.notify();
                                                    }
                                                }),
                                            )
                                        },
                                    ))
                            }),
                    )
                    .when(
                        state.status == ConnectionStatus::Ready && state.model().is_none(),
                        |view| {
                            view.child(
                                muted("Model selection is managed by this Codex version.")
                                    .text_size(px(12.))
                                    .p(px(8.)),
                            )
                        },
                    ),
            )
            .child(
                row().border_t_1().border_color(rgb(LINE)).pt(px(6.)).child(
                    Button::new("manage-agents")
                        .ghost()
                        .w_full()
                        .justify_start()
                        .icon(icon(IconName::Settings).size(px(15.)))
                        .label("Manage agents")
                        .text_size(px(12.))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Agents, window, cx)
                        })),
                ),
            )
    }

    fn connect_button(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        let state = &self.agent_states[self.selected_project];
        let source = state.source.clone();
        Button::new(id)
            .primary()
            .label(if state.installed {
                "Connect Codex"
            } else {
                "Set up Codex"
            })
            .rounded(px(9.))
            .disabled(state.status.is_busy())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.agent_action(AgentCommand::Connect(source.clone()), cx);
            }))
    }

    fn authentication_buttons(&self, cx: &mut Context<Self>) -> Vec<Button> {
        self.agent_states[self.selected_project]
            .auth_methods
            .iter()
            .enumerate()
            .map(|(index, method)| {
                let id = method.id.clone();
                Button::new(("authenticate", index))
                    .outline()
                    .label(if id == "api-key" {
                        "API Key (from environment)".to_owned()
                    } else {
                        method.name.clone()
                    })
                    .when_some(method.description.clone(), |button, description| {
                        button.tooltip(description)
                    })
                    .rounded(px(8.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.agent_action(AgentCommand::Authenticate(id.clone()), cx);
                    }))
            })
            .collect()
    }

    pub(super) fn connection_notice(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_states[self.selected_project];
        column()
            .w_full()
            .gap(px(8.))
            .when_some(
                self.agent_errors[self.selected_project]
                    .as_ref()
                    .or(state.error.as_ref()),
                |view, error| {
                    view.child(
                        div()
                            .w_full()
                            .max_h(px(100.))
                            .id("connection-error")
                            .overflow_y_scroll()
                            .p(px(12.))
                            .rounded(px(10.))
                            .bg(rgb(0xfff4ed))
                            .text_color(rgb(0x9a542a))
                            .text_size(px(12.))
                            .child(error.clone()),
                    )
                },
            )
            .when(
                state.status == ConnectionStatus::NeedsAuthentication,
                |view| {
                    view.child(
                        row()
                            .gap(px(8.))
                            .flex_wrap()
                            .children(self.authentication_buttons(cx)),
                    )
                },
            )
            .when(state.status != ConnectionStatus::Disconnected, |view| {
                view.child(
                    row()
                        .gap(px(7.))
                        .child(div().size(px(5.)).rounded_full().bg(rgb(
                            if state.status == ConnectionStatus::Ready {
                                0x61977b
                            } else {
                                0x9b9ba2
                            },
                        )))
                        .child(muted(state.status.label().to_owned()).text_size(px(11.)))
                        .when(state.pending_config.is_some(), |view| {
                            view.child(muted("· Applying selection…").text_size(px(11.)))
                        }),
                )
            })
    }

    pub(super) fn permission_cards(&self, cx: &mut Context<Self>) -> Div {
        column().w_full().gap(px(10.)).children(
            self.agent_states[self.selected_project]
                .permissions
                .iter()
                .map(|permission| {
                    column()
                        .gap(px(10.))
                        .p(px(15.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgb(0xe2d6bc))
                        .bg(rgb(0xfffcf5))
                        .child(muted("Codex needs your approval").text_size(px(11.)))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(13.))
                                .child(permission.title.clone()),
                        )
                        .when(!permission.detail.is_empty(), |view| {
                            view.child(
                                div()
                                    .id(("permission-detail", permission.id))
                                    .max_h(px(120.))
                                    .overflow_y_scroll()
                                    .text_size(px(11.))
                                    .child(permission.detail.clone()),
                            )
                        })
                        .child(
                            row().gap(px(8.)).flex_wrap().children(
                                permission
                                    .choices
                                    .iter()
                                    .enumerate()
                                    .map(|(index, choice)| {
                                        let id = permission.id;
                                        let value = choice.id.clone();
                                        Button::new((
                                            "permission-choice",
                                            permission.id as usize * 100 + index,
                                        ))
                                        .outline()
                                        .small()
                                        .label(choice.name.clone())
                                        .when(choice.allows, |button| button.primary())
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.agent_action(
                                                    AgentCommand::AnswerPermission {
                                                        id,
                                                        choice: Some(value.clone()),
                                                    },
                                                    cx,
                                                );
                                            }),
                                        )
                                    }),
                            ),
                        )
                }),
        )
    }

    pub(super) fn agents(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_states[self.selected_project];
        let connected = matches!(
            state.status,
            ConnectionStatus::Ready
                | ConnectionStatus::Running
                | ConnectionStatus::Cancelling
                | ConnectionStatus::NeedsAuthentication
                | ConnectionStatus::Authenticating
        );
        column().flex_1().min_h_0().child(column().id("agent-settings-scroll").flex_1().min_h_0().overflow_y_scroll().px(px(55.)).py(px(34.)).gap(px(22.))
            .child(div().text_size(px(30.)).font_weight(FontWeight::SEMIBOLD).child("Your agents, one workspace."))
            .child(muted(format!("Connect an agent for {}.", self.projects[self.selected_project].name)).text_size(px(16.)))
            .child(column().w_full().max_w(px(790.)).p(px(24.)).gap(px(20.)).rounded(px(16.)).border_1().border_color(rgb(LINE))
                .child(row().gap(px(15.)).child(row().justify_center().size(px(44.)).rounded(px(12.)).bg(rgb(0xf0f0f2)).child(icon(IconName::Sparkles).size(px(23.))))
                    .child(column().flex_1().gap(px(5.)).child(div().text_size(px(19.)).font_weight(FontWeight::SEMIBOLD).child("Codex"))
                        .child(muted("Run locally. Keep your project in Relay.").text_size(px(12.))))
                    .when(!connected, |view| view.child(self.connect_button("setup-codex", cx)))
                    .when(connected || state.status.is_busy(), |view| view.child(Button::new("disconnect-agent").outline().label("Disconnect").on_click(cx.listener(|this, _, _, cx| { this.agent_action(AgentCommand::Disconnect, cx); })))))
                .child(self.connection_notice(cx))
                .child(column().gap(px(10.)).child(muted("Installation").text_size(px(11.)))
                    .child(row().gap(px(10.)).flex_wrap()
                        .child(Button::new("use-managed-codex").outline().label("Managed by Relay").disabled(state.status.is_busy())
                            .when(state.source == AgentSource::Managed, |button| button.bg(rgb(0xededf0)))
                            .on_click(cx.listener(|this, _, _, cx| { this.agent_action(AgentCommand::Connect(AgentSource::Managed), cx); })))
                        .child(Button::new("choose-local-codex").ghost().label("Use local version…").disabled(state.status.is_busy()).on_click(cx.listener(|this, _, _, cx| this.choose_agent_path(false, cx))))
                        .child(Button::new("scan-local-codex").ghost().label(if state.discovering { "Scanning…" } else { "Scan local installs" }).disabled(state.discovering).on_click(cx.listener(|this, _, _, cx| { this.agent_action(AgentCommand::DiscoverLocal, cx); }))))
                    .child(muted(match &state.source { AgentSource::Managed => "Relay prepares a tested Codex version and its runtime on first use.".to_owned(), AgentSource::Local(path) => path.display().to_string() }).text_size(px(12.)))
                    .when_some(state.runtime_version.clone(), |view, version| view.child(muted(version).text_size(px(11.))))
                    .children(state.local_installations.iter().enumerate().map(|(index, path)| {
                        let path = path.clone();
                        Button::new(("local-codex-installation", index)).ghost().justify_start().label(path.display().to_string()).text_size(px(12.)).disabled(state.status.is_busy())
                            .on_click(cx.listener(move |this, _, _, cx| { this.agent_action(AgentCommand::Connect(AgentSource::Local(path.clone())), cx); }))
                    })))
                .child(column().gap(px(8.)).pt(px(16.)).border_t_1().border_color(rgb(LINE))
                    .child(row().justify_between().child(div().text_size(px(13.)).child("Project working folder"))
                        .child(Button::new("choose-working-folder").ghost().small().label("Choose folder…").disabled(state.status.is_busy()).on_click(cx.listener(|this, _, _, cx| this.choose_agent_path(true, cx)))))
                    .child(muted(state.working_directory.display().to_string()).text_size(px(11.)))
                    .child(muted("Codex works in this folder. Conversations remain part of the project.").text_size(px(12.))))
                .when(state.status == ConnectionStatus::Ready, |view| view.child(row().justify_between().pt(px(12.)).border_t_1().border_color(rgb(LINE))
                    .child(div().text_size(px(13.)).child("Harness & model"))
                    .child(self.agent_picker(cx))))
                .child(Button::new("back-to-project").ghost().label("Back to project").on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Project(this.selected_project), window, cx))))))
    }

    fn choose_agent_path(&mut self, directory: bool, cx: &mut Context<Self>) {
        let project = self.projects[self.selected_project].id;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(
                if directory {
                    "Choose project working folder"
                } else {
                    "Choose Codex executable"
                }
                .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    let command = if directory {
                        AgentCommand::SetWorkingDirectory(path)
                    } else {
                        AgentCommand::Connect(AgentSource::Local(path))
                    };
                    let result = this.agent_service.dispatch(project, command);
                    if let Some(index) = this.projects.iter().position(|p| p.id == project) {
                        this.agent_errors[index] = result.err();
                    }
                    this.refresh_agents(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
