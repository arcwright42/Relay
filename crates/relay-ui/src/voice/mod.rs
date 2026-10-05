//! Dedicated voice conversation surface over domain snapshots.
mod presentation;
#[cfg(test)]
mod tests;

use crate::{
    OpenThread,
    i18n::{Text, Translate},
};
use gpui_kit::{
    assets::IconName,
    component::{
        Icon, Sizable,
        button::{Button, ButtonVariants},
    },
    prelude::FluentBuilder,
    *,
};
use relay_core::{
    settings::SettingsService,
    voice::{TranscriptionStatus, VoiceService, VoiceSnapshot, VoiceTurnStage},
};
use std::{sync::Arc, time::Duration};

actions!(voice, [EndVoiceSession]);

pub struct VoicePanel {
    service: Arc<dyn VoiceService>,
    settings: Arc<dyn SettingsService>,
    snapshot: VoiceSnapshot,
    session_id: u64,
    focus: FocusHandle,
    _updates: Task<()>,
}
impl VoicePanel {
    pub fn new(
        service: Arc<dyn VoiceService>,
        settings: Arc<dyn SettingsService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = service.snapshot();
        let session_id = snapshot.session.as_ref().map_or(0, |session| session.id);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let executor = cx.background_executor().clone();
        let updates = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(50)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let snapshot = this.service.snapshot();
                        if snapshot
                            .session
                            .as_ref()
                            .is_none_or(|session| session.id != this.session_id)
                        {
                            window.remove_window();
                            return;
                        }
                        if this.snapshot != snapshot {
                            this.snapshot = snapshot;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            service,
            settings,
            snapshot,
            session_id,
            focus,
            _updates: updates,
        }
    }
    fn end(&mut self, window: &mut Window) {
        self.service.end_session(self.session_id);
        window.remove_window();
    }
}
impl EventEmitter<OpenThread> for VoicePanel {}

impl Render for VoicePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.snapshot().language;
        let text = |key| language.text(key);
        let level = self.snapshot.input_level as f32 / 100.;
        let session = self.snapshot.session.as_ref();
        let turn = session.and_then(|session| session.turn.as_ref());
        let busy = turn.is_some_and(|turn| {
            !matches!(
                turn.stage,
                VoiceTurnStage::Complete | VoiceTurnStage::Failed
            )
        });
        let title = turn.map_or(Text::VoiceSessionListening, |turn| {
            presentation::stage_text(turn.stage)
        });
        let detail = if let Some(error) = turn.and_then(|turn| turn.error.as_ref()) {
            presentation::error_text(error)
        } else if session
            .is_some_and(|session| session.transcription == TranscriptionStatus::NotConfigured)
        {
            Text::VoiceAsrNotConfigured
        } else if busy {
            Text::VoiceProcessingDetail
        } else {
            Text::VoiceContinue
        };
        div()
            .key_context("VoiceSession")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &EndVoiceSession, window, _| this.end(window)))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .p(px(24.))
            .rounded(px(22.))
            .bg(rgb(0xfafcff))
            .border_1()
            .border_color(rgb(0xe1e9ef))
            .shadow_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(52.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(rgb(0xe7f3ef))
                    .text_color(rgb(0x39765a))
                    .child(Icon::new(IconName::Mic).size(px(26.))),
            )
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(text(title)),
            )
            .child(
                div()
                    .id("voice-input-meter")
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .h(px(26.))
                    .children(
                        [0.3, 0.5, 0.7, 1., 0.8, 0.5, 0.3]
                            .into_iter()
                            .map(|factor| {
                                div()
                                    .w(px(4.))
                                    .h(px(4. + 22. * level * factor))
                                    .rounded_full()
                                    .bg(rgb(if level > 0.05 { 0x39765a } else { 0xd7e0e5 }))
                            }),
                    ),
            )
            .when_some(self.snapshot.input_device.clone(), |view, device| {
                view.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(0x7c8790))
                        .child(device),
                )
            })
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(12.))
                    .text_color(rgb(0x65727e))
                    .child(text(detail)),
            )
            .child(
                div()
                    .id("voice-turn-content")
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .when_some(turn.filter(|turn| !turn.prompt.is_empty()), |view, turn| {
                        view.child(
                            div()
                                .id("voice-transcript")
                                .flex_shrink_0()
                                .text_size(px(14.))
                                .line_height(px(22.))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(0x7c8790))
                                        .child(text(Text::VoiceYourRequest)),
                                )
                                .child(turn.prompt.clone()),
                        )
                    })
                    .when_some(
                        turn.filter(|turn| !turn.response.is_empty()),
                        |view, turn| {
                            view.child(
                                div()
                                    .id("voice-reply")
                                    .flex_shrink_0()
                                    .text_size(px(14.))
                                    .line_height(px(22.))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(0x7c8790))
                                            .child(text(Text::VoiceReply)),
                                    )
                                    .child(turn.response.clone()),
                            )
                        },
                    ),
            )
            .when_some(
                turn.and_then(|turn| turn.thread.as_ref()),
                |view, thread| {
                    let thread_id = thread.id;
                    view.child(
                        Button::new("voice-open-thread")
                            .ghost()
                            .small()
                            .label(format!("{} · {}", text(Text::VoiceOpenThread), thread.name))
                            .on_click(
                                cx.listener(move |_, _, _, cx| {
                                    cx.emit(OpenThread(Some(thread_id)))
                                }),
                            ),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap(px(10.))
                    .when(
                        turn.is_some_and(|turn| turn.stage != VoiceTurnStage::Complete),
                        |view| {
                            view.child(
                                Button::new("resume-voice-listening")
                                    .outline()
                                    .small()
                                    .label(text(if busy {
                                        Text::VoiceResume
                                    } else {
                                        Text::VoiceListenAgain
                                    }))
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.service.resume_listening(this.session_id)
                                    })),
                            )
                        },
                    )
                    .child(
                        Button::new("end-voice-session")
                            .outline()
                            .small()
                            .label(text(Text::EndVoiceSession))
                            .on_click(cx.listener(|this, _, window, _| this.end(window))),
                    ),
            )
    }
}
