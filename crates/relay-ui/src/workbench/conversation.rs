use super::*;

impl Workbench {
    fn composer(&self, cx: &mut Context<Self>) -> Div {
        let draft = &self.drafts[self.selected_project];
        let is_empty = draft.read(cx).value().trim().is_empty();
        let state = &self.agent_states[self.selected_project];
        let running = matches!(
            state.status,
            ConnectionStatus::Running | ConnectionStatus::Cancelling
        );
        composer_surface()
            .when(self.routing.pending_send.is_some(), |view| {
                view.child(
                    row()
                        .justify_between()
                        .mb(px(8.))
                        .child(muted(self.text(Text::RoutingWaiting)).text_size(px(12.)))
                        .child(
                            Button::new("cancel-routed-send")
                                .ghost()
                                .small()
                                .label(self.text(Text::Cancel))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.routing.pending_send = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                Textarea::new(draft)
                    .appearance(false)
                    .bordered(false)
                    .text_size(px(15.))
                    .h(px(68.))
                    .aria_label(self.text(Text::MessageAgent))
                    .context_menu(crate::locale::input_menu),
            )
            .child(
                row()
                    .justify_between()
                    .mt(px(4.))
                    .child(
                        icon_button(
                            "composer-context",
                            IconName::Plus,
                            self.text(Text::AddContext),
                        )
                        .rounded_full()
                        .on_click(cx.listener(|this, _, window, cx| this.show_context(window, cx))),
                    )
                    .child(
                        row()
                            .gap(px(12.))
                            .child(self.agent_picker(cx))
                            .when(!running, |view| {
                                view.child(
                                    Button::new("send")
                                        .primary()
                                        .icon(icon(IconName::ArrowUp).text_color(rgb(0xffffff)))
                                        .size(px(32.))
                                        .rounded_full()
                                        .disabled(
                                            is_empty
                                                || state.status.is_busy()
                                                || state.pending_config.is_some(),
                                        )
                                        .tooltip(self.text(Text::SendTooltip))
                                        .accessibility_label(self.text(Text::SendMessage))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send_message(&SendMessage, window, cx)
                                        })),
                                )
                            })
                            .when(running, |view| {
                                view.child(
                                    Button::new("stop-response")
                                        .primary()
                                        .icon(icon(IconName::Square).text_color(rgb(0xffffff)))
                                        .size(px(32.))
                                        .rounded_full()
                                        .disabled(state.status == ConnectionStatus::Cancelling)
                                        .tooltip(self.text(Text::StopResponse))
                                        .accessibility_label(self.text(Text::StopResponse))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.agent_action(AgentCommand::Cancel, cx);
                                        })),
                                )
                            }),
                    ),
            )
    }

    pub(super) fn conversation(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_states[self.selected_project];
        column()
            .flex_1()
            .min_h_0()
            .items_center()
            .px(px(if compact { 28. } else { 48. }))
            .pb(px(24.))
            .child(
                column()
                    .id("conversation-history")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.conversation_scroll)
                    .items_center()
                    .pb(px(22.))
                    .child(
                        column()
                            .w_full()
                            .max_w(px(CONTENT_WIDTH))
                            .gap(px(27.))
                            .py(px(20.))
                            .children(state.messages.iter().map(|message| {
                                let user = message.role == MessageRole::User;
                                column()
                                    .w_full()
                                    .gap(px(9.))
                                    .when(user, |view| view.items_end())
                                    .child(
                                        muted(if user { self.text(Text::You) } else { "Codex" })
                                            .text_size(px(11.)),
                                    )
                                    .children(message.tools.iter().map(|tool| {
                                        row()
                                            .gap(px(8.))
                                            .text_size(px(12.))
                                            .text_color(rgb(MUTED))
                                            .child(
                                                icon(if tool.status == "completed" {
                                                    IconName::Check
                                                } else {
                                                    IconName::CircleDashed
                                                })
                                                .size(px(13.)),
                                            )
                                            .child(if tool.title.is_empty() {
                                                self.text(Text::Working).to_owned()
                                            } else {
                                                tool.title.clone()
                                            })
                                    }))
                                    .when(!message.text.is_empty(), |view| {
                                        view.child(
                                            div()
                                                .max_w_full()
                                                .text_size(px(15.))
                                                .line_height(px(25.))
                                                .when(user, |view| {
                                                    view.px(px(17.))
                                                        .py(px(12.))
                                                        .rounded(px(16.))
                                                        .bg(rgb(0xf1f1f3))
                                                })
                                                .child(
                                                    gpui_kit::component::text::TextView::markdown(
                                                        ("chat-message", message.id),
                                                        message.text.clone(),
                                                    )
                                                    .selectable(true),
                                                ),
                                        )
                                    })
                                    .when(
                                        message.status == MessageStatus::Streaming
                                            && message.text.is_empty()
                                            && message.tools.is_empty(),
                                        |view| {
                                            view.child(
                                                muted(self.text(Text::Thinking)).text_size(px(13.)),
                                            )
                                        },
                                    )
                                    .when(message.status == MessageStatus::Interrupted, |view| {
                                        view.child(
                                            muted(
                                                self.text(
                                                    match message
                                                        .metrics
                                                        .as_ref()
                                                        .and_then(|metrics| metrics.outcome)
                                                    {
                                                        Some(TurnOutcome::Cancelled) => {
                                                            Text::ResponseCancelled
                                                        }
                                                        Some(TurnOutcome::Refused) => {
                                                            Text::ResponseRefused
                                                        }
                                                        Some(TurnOutcome::Failed) => {
                                                            Text::ResponseFailed
                                                        }
                                                        _ => Text::Interrupted,
                                                    },
                                                ),
                                            )
                                            .text_size(px(11.)),
                                        )
                                    })
                                    .when(message.metrics.is_some(), |view| {
                                        let id = message.id;
                                        view.child(
                                            Button::new(("turn-diagnostics", id))
                                                .ghost()
                                                .small()
                                                .label(self.text(Text::ResponseDetails))
                                                .text_color(rgb(MUTED))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.show_diagnostics(id, window, cx)
                                                    },
                                                )),
                                        )
                                    })
                                    .when(
                                        !user
                                            && message.status == MessageStatus::Complete
                                            && !message.text.trim().is_empty(),
                                        |view| {
                                            let message = message.clone();
                                            view.child(
                                                Button::new(("remember-reply", message.id))
                                                    .ghost()
                                                    .small()
                                                    .label(self.text(Text::RememberReply))
                                                    .text_color(rgb(MUTED))
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.remember_reply(
                                                                message.clone(),
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        },
                                    )
                            })),
                    ),
            )
            .child(
                column()
                    .w_full()
                    .max_w(px(CONTENT_WIDTH))
                    .gap(px(10.))
                    .flex_shrink_0()
                    .child(self.permission_cards(cx))
                    .child(self.composer(cx))
                    .when(self.needs_connection_notice(), |view| {
                        view.child(self.connection_notice(cx))
                    }),
            )
    }

    fn quick_action(
        &self,
        id: usize,
        glyph: IconName,
        title: &'static str,
        prompt: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(("quick-action", id))
            .ghost()
            .icon(icon(glyph).size(px(17.)))
            .label(title)
            .h(px(34.))
            .px(px(12.))
            .rounded(px(17.))
            .text_color(rgb(0x73737a))
            .text_size(px(12.))
            .on_click(cx.listener(move |this, _, window, cx| this.use_prompt(prompt, window, cx)))
    }

    pub(super) fn welcome(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        column()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .px(px(if compact { 28. } else { 48. }))
            .pb(px(96.))
            .child(
                column()
                    .w_full()
                    .max_w(px(CONTENT_WIDTH))
                    .items_center()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.text(Text::WelcomePrompt)),
                    )
                    .child(div().w_full().mt(px(26.)).child(self.composer(cx)))
                    .when(self.needs_connection_notice(), |view| {
                        view.child(div().w_full().mt(px(10.)).child(self.connection_notice(cx)))
                    })
                    .child(
                        row()
                            .justify_center()
                            .gap(px(12.))
                            .mt(px(18.))
                            .flex_wrap()
                            .child(self.quick_action(
                                1,
                                IconName::ChartNoAxesColumn,
                                self.text(Text::Analyze),
                                self.text(Text::AnalyzePrompt),
                                cx,
                            ))
                            .child(self.quick_action(
                                2,
                                IconName::Scale,
                                self.text(Text::Compare),
                                self.text(Text::ComparePrompt),
                                cx,
                            ))
                            .child(self.quick_action(
                                3,
                                IconName::Sparkles,
                                self.text(Text::PlanProject),
                                self.text(Text::PlanPrompt),
                                cx,
                            )),
                    ),
            )
    }
}
