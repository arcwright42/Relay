use super::*;
use gpui_kit::component::popover::Popover;
use relay_core::{ThreadId, sessions::*};

impl Workbench {
    pub fn set_client_session_service(
        &mut self,
        service: Arc<dyn ClientSessionsService>,
        cx: &mut Context<Self>,
    ) {
        self.client_session_snapshot = service.snapshot();
        self.client_session_revision = service.revision();
        self.client_session_service = service;
        cx.notify();
    }

    pub(super) fn refresh_client_sessions(&mut self, cx: &mut Context<Self>) {
        let revision = self.client_session_service.revision();
        if self.client_session_revision != revision {
            self.client_session_revision = revision;
            self.client_session_snapshot = self.client_session_service.snapshot();
            cx.notify();
        }
    }

    fn client_session_action(&mut self, command: ClientSessionsCommand, cx: &mut Context<Self>) {
        self.client_session_error = self.client_session_service.dispatch(command).err();
        self.refresh_client_sessions(cx);
        cx.notify();
    }

    fn client_thread_picker(&self, assigning: bool, cx: &mut Context<Self>) -> Popover {
        let selected = self
            .client_session_snapshot
            .detail
            .as_ref()
            .map(|d| d.session.clone());
        let current = if assigning {
            selected.as_ref().and_then(|s| s.thread)
        } else {
            self.client_session_filter
        };
        let label = current
            .and_then(|id| self.threads.iter().find(|p| p.id == id))
            .map(|p| p.name.as_str())
            .unwrap_or_else(|| {
                self.text(if assigning {
                    Text::ClientSessionsAssign
                } else {
                    Text::ClientSessionsAll
                })
            })
            .to_owned();
        let weak = cx.entity().downgrade();
        Popover::new(if assigning {
            "client-session-binding"
        } else {
            "client-session-filter"
        })
        .open(if assigning {
            self.client_session_binding_open
        } else {
            self.client_session_filter_open
        })
        .on_open_change(cx.listener(move |this, open, _, cx| {
            if assigning {
                this.client_session_binding_open = *open;
            } else {
                this.client_session_filter_open = *open;
            }
            cx.notify();
        }))
        .trigger(
            Button::new(if assigning {
                "assign-client-session"
            } else {
                "filter-client-sessions"
            })
            .outline()
            .label(label)
            .disabled(assigning && (selected.is_none() || self.client_session_snapshot.saving)),
        )
        .content(move |_, _, cx| {
            weak.update(cx, |this, cx| this.client_thread_menu(assigning, cx))
                .unwrap_or_else(|_| column())
        })
    }

    fn client_thread_menu(&self, assigning: bool, cx: &mut Context<Self>) -> Div {
        column().child(
            column()
                .id("client-thread-picker-list")
                .w(px(250.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .child(
                    Button::new("client-session-thread-none")
                        .ghost()
                        .w_full()
                        .justify_start()
                        .label(self.text(if assigning {
                            Text::ClientSessionsUnassigned
                        } else {
                            Text::ClientSessionsAll
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_client_thread(assigning, None, cx)
                        })),
                )
                .children(self.threads.iter().map(|thread| {
                    let id = thread.id;
                    Button::new(("client-session-thread", id.0))
                        .ghost()
                        .w_full()
                        .justify_start()
                        .label(thread.name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_client_thread(assigning, Some(id), cx)
                        }))
                })),
        )
    }

    fn choose_client_thread(
        &mut self,
        assigning: bool,
        thread: Option<ThreadId>,
        cx: &mut Context<Self>,
    ) {
        if assigning {
            self.client_session_binding_open = false;
            if let Some(id) = self
                .client_session_snapshot
                .detail
                .as_ref()
                .map(|d| d.session.id.clone())
            {
                self.client_session_action(
                    ClientSessionsCommand::Assign {
                        session: id,
                        thread,
                    },
                    cx,
                );
            }
        } else {
            self.client_session_filter = thread;
            self.client_session_filter_open = false;
            cx.notify();
        }
    }

    pub(super) fn client_sessions(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.client_session_snapshot;
        let query = self
            .client_session_search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let sessions: Vec<_> = state
            .sessions
            .iter()
            .filter(|s| {
                self.client_session_filter
                    .is_none_or(|p| s.thread == Some(p))
            })
            .filter(|s| {
                query.is_empty()
                    || s.title.to_lowercase().contains(&query)
                    || s.working_directory
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
                    || s.native_id.to_lowercase().contains(&query)
            })
            .collect();
        let stats = match (self.settings_snapshot.language, state.syncing) {
            (Language::SimplifiedChinese, true) => format!(
                "已扫描 {} 个文件 · 本次更新 {} 个 · 跳过 {} 个文件",
                state.scanned_files, state.updated_sessions, state.failed_files
            ),
            (Language::English, true) => format!(
                "{} files scanned · {} updated · {} files skipped",
                state.scanned_files, state.updated_sessions, state.failed_files
            ),
            (Language::SimplifiedChinese, false) => format!(
                "{} 个会话 · 本次更新 {} 个 · 跳过 {} 个文件",
                state.sessions.len(),
                state.updated_sessions,
                state.failed_files
            ),
            (Language::English, false) => format!(
                "{} sessions · {} updated · {} files skipped",
                state.sessions.len(),
                state.updated_sessions,
                state.failed_files
            ),
        };
        column()
            .flex_1()
            .min_h_0()
            .px(px(24.))
            .pb(px(24.))
            .gap(px(14.))
            .child(
                row()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(5.))
                            .child(muted(self.text(Text::ClientSessionsHint)).text_size(px(13.)))
                            .child(muted(stats).text_size(px(13.))),
                    )
                    .child(self.client_thread_picker(false, cx))
                    .child(
                        Button::new("sync-client-sessions")
                            .outline()
                            .label(self.text(if state.syncing {
                                Text::ClientSessionsSyncing
                            } else {
                                Text::ClientSessionsSync
                            }))
                            .disabled(state.syncing)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.client_session_action(ClientSessionsCommand::Sync, cx)
                            })),
                    ),
            )
            .child(
                Input::new(&self.client_session_search)
                    .id("search-client-sessions")
                    .aria_label(self.text(Text::ClientSessionsSearch))
                    .context_menu(crate::locale::input_menu),
            )
            .when(sessions.len() > 200, |v| {
                v.child(muted(self.text(Text::ClientSessionsListLimit)).text_size(px(13.)))
            })
            .when_some(
                self.client_session_error.as_ref().or(state.error.as_ref()),
                |view, error| {
                    view.child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0x9a542a))
                            .child(error.clone()),
                    )
                },
            )
            .child(
                row()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .gap(px(18.))
                    .child(
                        column()
                            .id("client-session-list")
                            .w(px(300.))
                            .h_full()
                            .overflow_y_scroll()
                            .gap(px(5.))
                            .when(sessions.is_empty(), |view| {
                                view.child(
                                    muted(self.text(Text::ClientSessionsEmpty))
                                        .py(px(30.))
                                        .text_size(px(13.)),
                                )
                            })
                            .children(sessions.iter().take(200).map(|session| {
                                let id = session.id.clone();
                                let thread = session
                                    .thread
                                    .and_then(|id| self.threads.iter().find(|p| p.id == id))
                                    .map(|p| p.name.as_str())
                                    .unwrap_or(self.text(Text::ClientSessionsUnassigned));
                                Button::new(ElementId::Name(
                                    format!("client-session-{}", id.0).into(),
                                ))
                                .ghost()
                                .w_full()
                                .h_auto()
                                .p(px(12.))
                                .justify_start()
                                .when(state.selected.as_ref() == Some(&id), |b| {
                                    b.bg(rgb(0xededf0))
                                })
                                .child(
                                    column()
                                        .w_full()
                                        .gap(px(6.))
                                        .child(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_size(px(13.))
                                                .child(session.title.clone()),
                                        )
                                        .child(
                                            muted(format!(
                                                "{} · {thread} · {}",
                                                session.client, session.message_count
                                            ))
                                            .text_size(px(13.)),
                                        )
                                        .child(
                                            muted(session.working_directory.display().to_string())
                                                .w_full()
                                                .truncate()
                                                .text_size(px(13.)),
                                        ),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.client_session_binding_open = false;
                                        this.client_session_action(
                                            ClientSessionsCommand::Open(id.clone()),
                                            cx,
                                        );
                                    },
                                ))
                            })),
                    )
                    .child(self.client_session_detail(cx)),
            )
    }

    fn client_session_detail(&self, cx: &mut Context<Self>) -> Div {
        let Some(detail) = &self.client_session_snapshot.detail else {
            return column()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .child(muted(self.text(
                    if self.client_session_snapshot.selected.is_some() {
                        Text::ClientSessionsLoading
                    } else {
                        Text::ClientSessionsChoose
                    },
                )));
        };
        let session = &detail.session;
        column()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap(px(12.))
            .child(
                row()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(17.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(session.title.clone()),
                    )
                    .child(self.client_thread_picker(true, cx)),
            )
            .child(muted(session.source.display().to_string()).text_size(px(13.)))
            .when(!session.available, |v| {
                v.child(muted(self.text(Text::ClientSessionsMissing)).text_size(px(13.)))
            })
            .when(detail.messages.len() > 80, |v| {
                v.child(muted(self.text(Text::ClientSessionsRecent)).text_size(px(13.)))
            })
            .child(
                column()
                    .id("client-session-transcript")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(px(16.))
                    .children(
                        detail
                            .messages
                            .iter()
                            .skip(detail.messages.len().saturating_sub(80))
                            .map(|message| {
                                let copy = message.text.clone();
                                let preview: String = message.text.chars().take(12_000).collect();
                                column()
                                    .w_full()
                                    .p(px(16.))
                                    .gap(px(10.))
                                    .rounded(px(12.))
                                    .bg(rgb(if message.role == MessageRole::User {
                                        0xf7f7f8
                                    } else {
                                        SURFACE
                                    }))
                                    .child(
                                        row()
                                            .gap(px(10.))
                                            .child(
                                                muted(if message.role == MessageRole::User {
                                                    "User"
                                                } else {
                                                    "Codex"
                                                })
                                                .flex_1()
                                                .text_size(px(13.)),
                                            )
                                            .child(
                                                Button::new(("copy-client-message", message.id))
                                                    .ghost()
                                                    .small()
                                                    .label(self.text(Text::Copy))
                                                    .on_click(move |_, _, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(copy.clone()),
                                                        )
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .line_height(px(21.))
                                            .child(preview.clone()),
                                    )
                                    .when(preview.len() < message.text.len(), |v| {
                                        v.child(
                                            muted(self.text(Text::ClientSessionsTruncated))
                                                .text_size(px(13.)),
                                        )
                                    })
                            }),
                    ),
            )
    }
}
