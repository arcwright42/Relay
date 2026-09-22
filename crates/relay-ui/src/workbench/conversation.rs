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
        column().w_full().p(px(13.)).rounded(px(18.)).border_1().border_color(rgb(0xdfdfe4)).bg(rgb(SURFACE))
            .child(Textarea::new(draft).appearance(false).bordered(false).text_size(px(17.)).h(px(65.)).aria_label("Message your project agent").map(|input| {
                #[cfg(feature = "devtools")]
                let input = input.context_menu(crate::devtools::input_menu);
                input
            }))
            .child(row().justify_between().mt(px(4.))
                .child(row().gap(px(6.))
                    .child(icon_button("add", IconName::Plus, "Add to your message").rounded(px(18.)).bg(rgb(0xf4f4f5)).on_click(cx.listener(|this, _, window, cx| this.show_context(None, window, cx))))
                    .child(icon_button("attach", IconName::Paperclip, "Attach a file").on_click(|_, window, cx| explain("Attach a file", "File attachments will be connected in the next step. Your project will keep its files independently of the agent you choose.", window, cx)))
                    .child(icon_button("image", IconName::Image, "Add an image").on_click(|_, window, cx| explain("Add an image", "Images and screenshots will become part of this project’s context. Image capture and import will be connected next.", window, cx)))
                    .child(Button::new("composer-context").ghost().icon(icon(IconName::Layers).size(px(16.))).label("Add context").text_size(px(12.)).text_color(rgb(0x7c7c82)).on_click(cx.listener(|this, _, window, cx| this.show_context(None, window, cx)))))
                .child(row().gap(px(12.))
                    .child(self.agent_picker(cx))
                    .when(!running, |view| view.child(Button::new("send").primary().icon(icon(IconName::ArrowUp).text_color(rgb(0xffffff))).size(px(35.)).rounded(px(9.)).bg(rgb(0x353537)).disabled(is_empty || state.status.is_busy() || state.pending_config.is_some()).tooltip("Send message · ⌘Enter").accessibility_label("Send message").on_click(cx.listener(|this, _, window, cx| this.send_message(&SendMessage, window, cx)))))
                    .when(running, |view| view.child(Button::new("stop-response").primary().icon(icon(IconName::Square).text_color(rgb(0xffffff))).size(px(35.)).rounded(px(9.)).bg(rgb(0x353537)).disabled(state.status == ConnectionStatus::Cancelling).tooltip("Stop response").accessibility_label("Stop response").on_click(cx.listener(|this, _, _, cx| { this.agent_action(AgentCommand::Cancel, cx); }))))))
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
                            .max_w(px(800.))
                            .gap(px(27.))
                            .py(px(20.))
                            .children(state.messages.iter().map(|message| {
                                let user = message.role == MessageRole::User;
                                column()
                                    .w_full()
                                    .gap(px(9.))
                                    .when(user, |view| view.items_end())
                                    .child(
                                        muted(if user { "You" } else { "Codex" })
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
                                                "Working…".to_owned()
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
                                        |view| view.child(muted("Thinking…").text_size(px(13.))),
                                    )
                                    .when(message.status == MessageStatus::Interrupted, |view| {
                                        view.child(muted("Response interrupted").text_size(px(11.)))
                                    })
                            })),
                    ),
            )
            .child(
                column()
                    .w_full()
                    .max_w(px(800.))
                    .gap(px(10.))
                    .flex_shrink_0()
                    .child(self.permission_cards(cx))
                    .child(self.composer(cx))
                    .child(self.connection_notice(cx)),
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
            .h(px(42.))
            .px(px(18.))
            .rounded(px(23.))
            .bg(rgb(0xf6f6f7))
            .text_size(px(12.))
            .on_click(cx.listener(move |this, _, window, cx| this.use_prompt(prompt, window, cx)))
    }

    pub(super) fn welcome(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        column().flex_1().min_h_0().items_center().justify_center().px(px(if compact { 28. } else { 48. })).pb(px(57.))
            .child(column().w_full().max_w(px(728.)).items_center()
                .child(div().text_size(px(if compact { 29. } else { 36. })).font_weight(FontWeight::SEMIBOLD).child("Good afternoon, Alex."))
                .child(muted("What would you like to work on today?").text_size(px(if compact { 18. } else { 24. })).mt(px(7.)))
                .child(div().w_full().mt(px(35.)).child(self.composer(cx)))
                .child(div().w_full().mt(px(10.)).child(self.connection_notice(cx)))
                .child(column().items_center().gap(px(12.)).mt(px(24.))
                    .child(row().justify_center().gap(px(10.)).flex_wrap()
                        .child(self.quick_action(0, IconName::FileText, "Summarize this page", "Summarize this page and highlight the key takeaways.\n\n", cx))
                        .child(self.quick_action(1, IconName::ChartNoAxesColumn, "Analyze a document", "Analyze this document and help me understand its key findings.\n\n", cx))
                        .child(self.quick_action(2, IconName::Scale, "Compare options", "Help me compare these options and their trade-offs.\n\n", cx)))
                    .child(row().justify_center().gap(px(10.)).flex_wrap()
                        .child(self.quick_action(3, IconName::Sparkles, "Plan a project", "Help me turn this idea into a clear project plan.\n\n", cx))
                        .child(self.quick_action(4, IconName::Image, "Generate a design", "Explore a design direction for this idea.\n\n", cx))
                        .child(Button::new("more-actions").ghost().icon(icon(IconName::Ellipsis)).label("More").h(px(42.)).px(px(18.)).rounded(px(23.)).bg(rgb(0xf6f6f7)).text_size(px(12.)).on_click(cx.listener(|this, _, window, cx| {
                            this.use_prompt("Help me work through an idea.\n\n", window, cx);
                        }))))))
    }
}
