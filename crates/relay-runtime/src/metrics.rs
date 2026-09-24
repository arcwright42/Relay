use relay_core::agents::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct SavedMetrics {
    model: Option<String>,
    first_text_ms: Option<u64>,
    total_ms: Option<u64>,
    context_kind: String,
    context_revision: Option<u64>,
    context_bytes: usize,
    restored_history: bool,
    usage: Option<SavedUsage>,
    outcome: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SavedUsage {
    input_tokens: u64,
    output_tokens: u64,
    cached_read_tokens: Option<u64>,
    cached_write_tokens: Option<u64>,
    thought_tokens: Option<u64>,
}

impl From<&TurnMetrics> for SavedMetrics {
    fn from(value: &TurnMetrics) -> Self {
        Self {
            model: value.model.clone(),
            first_text_ms: value.first_text_ms,
            total_ms: value.total_ms,
            context_kind: match value.context_kind {
                ContextDeliveryKind::Unchanged => "unchanged",
                ContextDeliveryKind::Snapshot => "snapshot",
                ContextDeliveryKind::Delta => "delta",
            }
            .into(),
            context_revision: value.context_revision,
            context_bytes: value.context_bytes,
            restored_history: value.restored_history,
            outcome: value.outcome.map(|outcome| {
                match outcome {
                    TurnOutcome::Complete => "complete",
                    TurnOutcome::Cancelled => "cancelled",
                    TurnOutcome::Refused => "refused",
                    TurnOutcome::Failed => "failed",
                }
                .into()
            }),
            usage: value.usage.as_ref().map(|usage| SavedUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_read_tokens: usage.cached_read_tokens,
                cached_write_tokens: usage.cached_write_tokens,
                thought_tokens: usage.thought_tokens,
            }),
        }
    }
}

impl From<SavedMetrics> for TurnMetrics {
    fn from(value: SavedMetrics) -> Self {
        Self {
            model: value.model,
            first_text_ms: value.first_text_ms,
            total_ms: value.total_ms,
            context_kind: match value.context_kind.as_str() {
                "snapshot" => ContextDeliveryKind::Snapshot,
                "delta" => ContextDeliveryKind::Delta,
                _ => ContextDeliveryKind::Unchanged,
            },
            context_revision: value.context_revision,
            context_bytes: value.context_bytes,
            restored_history: value.restored_history,
            outcome: value.outcome.as_deref().and_then(|value| match value {
                "complete" => Some(TurnOutcome::Complete),
                "cancelled" => Some(TurnOutcome::Cancelled),
                "refused" => Some(TurnOutcome::Refused),
                "failed" => Some(TurnOutcome::Failed),
                _ => None,
            }),
            usage: value.usage.map(|usage| TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_read_tokens: usage.cached_read_tokens,
                cached_write_tokens: usage.cached_write_tokens,
                thought_tokens: usage.thought_tokens,
            }),
        }
    }
}
