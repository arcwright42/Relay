use crate::i18n::Text;
use relay_core::voice::{VoiceTurnError, VoiceTurnStage};

pub(super) fn stage_text(stage: VoiceTurnStage) -> Text {
    match stage {
        VoiceTurnStage::Transcribing => Text::VoiceTranscribing,
        VoiceTurnStage::Connecting => Text::VoiceConnecting,
        VoiceTurnStage::WaitingForAgent => Text::VoiceWaiting,
        VoiceTurnStage::NeedsAttention => Text::VoiceNeedsAttention,
        VoiceTurnStage::Speaking => Text::VoiceSpeaking,
        VoiceTurnStage::Complete => Text::VoiceSessionListening,
        VoiceTurnStage::Failed => Text::VoiceRequestFailed,
    }
}

pub(super) fn error_text(error: &VoiceTurnError) -> Text {
    match error {
        VoiceTurnError::AsrNotConfigured => Text::VoiceAsrNotConfigured,
        VoiceTurnError::Transcription(_) => Text::VoiceTranscriptionFailed,
        VoiceTurnError::ThreadUnavailable => Text::VoiceAgentFailed,
        VoiceTurnError::AgentBusy => Text::VoiceAgentBusy,
        VoiceTurnError::AgentAuthentication => Text::VoiceAgentAuthentication,
        VoiceTurnError::Agent(_) => Text::VoiceAgentFailed,
        VoiceTurnError::SpeechOutput(_) => Text::VoiceSpeechFailed,
        VoiceTurnError::InputTooLong => Text::VoiceInputTooLong,
        VoiceTurnError::QueueFull => Text::VoiceQueueFull,
        VoiceTurnError::Cancelled => Text::VoiceContinue,
    }
}
