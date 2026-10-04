use super::*;
use relay_core::voice::{VoiceErrorKind, VoiceService, VoiceStatus, WAKE_PHRASE};

pub struct OpenMicrophoneSettings;
impl EventEmitter<OpenMicrophoneSettings> for Workbench {}

impl Workbench {
    pub fn set_voice_service(&mut self, service: Arc<dyn VoiceService>, cx: &mut Context<Self>) {
        self.voice_snapshot = service.snapshot();
        self.voice_service = service;
        cx.notify();
    }

    pub(super) fn voice_settings(&self, cx: &mut Context<Self>) -> Div {
        let enabled = self.voice_snapshot.enabled;
        let status = match &self.voice_snapshot.status {
            VoiceStatus::Off => Text::VoiceWakeOff,
            VoiceStatus::Starting => Text::VoiceWakeStarting,
            VoiceStatus::RequestingMicrophone => Text::VoiceWakePermission,
            VoiceStatus::Listening => Text::VoiceWakeListening,
            VoiceStatus::Capturing => Text::VoiceSessionListening,
            VoiceStatus::Failed(error) => match error.kind {
                VoiceErrorKind::ResourcesUnavailable => Text::VoiceWakeResourcesError,
                VoiceErrorKind::MicrophoneDenied => Text::VoiceWakeDenied,
                VoiceErrorKind::MicrophoneUnavailable => Text::VoiceWakeMicrophoneError,
                VoiceErrorKind::DetectionFailed => Text::VoiceWakeDetectionError,
            },
        };
        let failed = matches!(self.voice_snapshot.status, VoiceStatus::Failed(_));
        let denied = matches!(&self.voice_snapshot.status, VoiceStatus::Failed(error) if error.kind == VoiceErrorKind::MicrophoneDenied);
        column()
            .gap(px(12.))
            .pb(px(20.))
            .border_b_1()
            .border_color(rgb(LINE))
            .child(
                row()
                    .justify_between()
                    .gap(px(20.))
                    .child(
                        column()
                            .flex_1()
                            .min_w(px(0.))
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.text(Text::VoiceWake)),
                            )
                            .child(
                                muted(self.text(Text::VoiceWakeDetail))
                                    .w_full()
                                    .text_size(px(12.))
                                    .line_height(px(20.)),
                            ),
                    )
                    .child(
                        Button::new("voice-wake-toggle")
                            .flex_shrink_0()
                            .outline()
                            .small()
                            .label(self.text(if enabled {
                                Text::DisableVoiceWake
                            } else {
                                Text::EnableVoiceWake
                            }))
                            .when(enabled, |button| button.primary().icon(IconName::Check))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.voice_service.set_enabled(!enabled);
                                this.voice_snapshot = this.voice_service.snapshot();
                                this.settings_snapshot = this.settings_service.snapshot();
                                cx.notify();
                            })),
                    ),
            )
            .child(muted(WAKE_PHRASE).text_size(px(12.)))
            .when_some(self.voice_snapshot.input_device.as_ref(), |view, device| {
                view.child(muted(device.clone()).text_size(px(11.)))
            })
            .when(
                self.voice_snapshot.status == VoiceStatus::Listening,
                |view| {
                    view.child(
                        Button::new("start-voice-session")
                            .ghost()
                            .small()
                            .label(self.text(Text::StartVoiceSession))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.voice_service.start_session();
                                this.voice_snapshot = this.voice_service.snapshot();
                                cx.notify();
                            })),
                    )
                },
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(if failed {
                        0x9a542a
                    } else if self.voice_snapshot.status == VoiceStatus::Listening {
                        0x39765a
                    } else {
                        MUTED
                    }))
                    .child(self.text(status)),
            )
            .when(failed && enabled, |view| {
                view.child(
                    row()
                        .gap(px(8.))
                        .child(
                            Button::new("retry-voice-wake")
                                .ghost()
                                .small()
                                .label(self.text(Text::Retry))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.voice_service.retry();
                                    this.voice_snapshot = this.voice_service.snapshot();
                                    cx.notify();
                                })),
                        )
                        .when(denied, |view| {
                            view.child(
                                Button::new("microphone-settings")
                                    .outline()
                                    .small()
                                    .label(self.text(Text::OpenMicrophoneSettings))
                                    .on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(OpenMicrophoneSettings)),
                                    ),
                            )
                        }),
                )
            })
    }
}
