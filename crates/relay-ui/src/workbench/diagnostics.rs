use super::*;

fn duration(value: Option<u64>, language: Language) -> String {
    value
        .map(|ms| format!("{:.3} s", ms as f64 / 1_000.))
        .unwrap_or_else(|| language.text(Text::NotReported).into())
}

impl Workbench {
    pub(super) fn show_diagnostics(&self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(metrics) = self.agent_states[self.selected_project]
            .messages
            .iter()
            .find(|message| message.id == id)
            .and_then(|message| message.metrics.clone())
        else {
            return;
        };
        let language = self.settings_snapshot.language;
        let missing = language.text(Text::NotReported);
        let usage = metrics.usage.as_ref();
        let count = |value: Option<u64>| {
            value
                .map(|v| v.to_string())
                .unwrap_or_else(|| missing.into())
        };
        let fields = vec![
            (
                Text::TurnOutcome,
                metrics
                    .outcome
                    .map(|outcome| {
                        language
                            .text(match outcome {
                                TurnOutcome::Complete => Text::ResponseComplete,
                                TurnOutcome::Cancelled => Text::ResponseCancelled,
                                TurnOutcome::Refused => Text::ResponseRefused,
                                TurnOutcome::Failed => Text::ResponseFailed,
                            })
                            .into()
                    })
                    .unwrap_or_else(|| language.text(Text::Working).into()),
            ),
            (Text::Model, metrics.model.unwrap_or_else(|| missing.into())),
            (
                Text::FirstTextLatency,
                duration(metrics.first_text_ms, language),
            ),
            (Text::TotalDuration, duration(metrics.total_ms, language)),
            (
                Text::ContextDelivery,
                language
                    .text(match metrics.context_kind {
                        ContextDeliveryKind::Snapshot => Text::ContextSnapshot,
                        ContextDeliveryKind::Delta => Text::ContextDelta,
                        ContextDeliveryKind::Unchanged => Text::ContextUnchanged,
                    })
                    .into(),
            ),
            (Text::ContextRevision, count(metrics.context_revision)),
            (Text::ContextBytes, metrics.context_bytes.to_string()),
            (
                Text::HistoryRestored,
                language
                    .text(if metrics.restored_history {
                        Text::Yes
                    } else {
                        Text::No
                    })
                    .into(),
            ),
            (Text::UncachedInput, count(usage.map(|u| u.input_tokens))),
            (
                Text::CachedRead,
                count(usage.and_then(|u| u.cached_read_tokens)),
            ),
            (
                Text::CachedWrite,
                count(usage.and_then(|u| u.cached_write_tokens)),
            ),
            (
                Text::CacheReadRatio,
                usage
                    .and_then(TokenUsage::cache_read_ratio)
                    .map(|ratio| format!("{:.1}%", ratio * 100.))
                    .unwrap_or_else(|| missing.into()),
            ),
            (Text::OutputTokens, count(usage.map(|u| u.output_tokens))),
            (
                Text::ReasoningTokens,
                count(usage.and_then(|u| u.thought_tokens)),
            ),
        ];
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(language.text(Text::ResponseDetails))
                .width(px(570.))
                .child(
                    column()
                        .gap(px(12.))
                        .child(
                            column()
                                .id("response-diagnostics")
                                .max_h(px(410.))
                                .overflow_y_scroll()
                                .gap(px(12.))
                                .children(fields.iter().map(|(label, value)| {
                                    row()
                                        .gap(px(18.))
                                        .justify_between()
                                        .child(muted(language.text(*label)).text_size(px(12.)))
                                        .child(
                                            div()
                                                .max_w(px(280.))
                                                .text_size(px(12.))
                                                .child(value.clone()),
                                        )
                                })),
                        )
                        .child(muted(language.text(Text::MetricsDetail)).text_size(px(12.)))
                        .child(muted(language.text(Text::UsageScopeDetail)).text_size(px(12.))),
                )
        });
    }
}
