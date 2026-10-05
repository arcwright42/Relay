use super::*;

impl MemoryView {
    fn status(&self, code: &str) -> &'static str {
        self.text(match code {
            "candidate" => Text::MemoryCandidate,
            "confirmed" => Text::MemoryConfirmed,
            "pending" => Text::MemoryPending,
            "running" => Text::MemoryExtracting,
            "done" => Text::MemoryProcessed,
            "failed" => Text::MemoryFailed,
            _ => Text::MemoryRecorded,
        })
    }
    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        if !self.loaded {
            return column().child(muted(self.text(if self.load_error.is_some() {
                Text::MemoryReadFailed
            } else {
                Text::Loading
            })));
        }
        let progress = &self.overview.progress;
        let busy = self.saving || self.editing.is_some();
        let status = if !progress.enabled {
            Text::MemoryPaused
        } else if progress.observer_error.is_some() {
            Text::MemoryObserverError
        } else if progress.running > 0 {
            Text::MemoryExtracting
        } else if progress.runs_this_hour >= progress.hourly_budget {
            Text::MemoryBudgetReached
        } else if !matches!(
            self.agent_status,
            ConnectionStatus::Ready | ConnectionStatus::Running
        ) {
            Text::MemoryAwaitingAgent
        } else {
            Text::MemoryReady
        };
        let status = if progress.running > 0 {
            format!(
                "{} · {} {}",
                self.text(status),
                progress.running,
                self.text(Text::MemoryInFlight)
            )
        } else {
            self.text(status).into()
        };
        column()
            .gap(px(10.))
            .child(
                row()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(div().flex_1().min_w(px(160.)).child(status))
                    .child(
                        Button::new("memory-connect")
                            .outline()
                            .small()
                            .label(self.text(Text::MemoryOpenAgent))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectCoordinator))),
                    )
                    .child(
                        Button::new("memory-pause")
                            .outline()
                            .small()
                            .disabled(busy)
                            .label(self.text(if progress.enabled {
                                Text::MemoryPause
                            } else {
                                Text::MemoryResume
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.command(
                                    MemoryCommand::SetEnabled(!this.overview.progress.enabled),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("memory-sync")
                            .outline()
                            .small()
                            .disabled(self.archive_state.syncing)
                            .label(self.text(if self.archive_state.syncing {
                                Text::ClientSessionsSyncing
                            } else {
                                Text::MemorySync
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.error =
                                    this.archives.dispatch(ClientSessionsCommand::Sync).err();
                                this.refresh_archives(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                muted(format!(
                    "{} {} · {} · {} / {} {}",
                    progress.sessions,
                    self.text(Text::MemorySessions),
                    self.text(Text::MemoryBudgetHint),
                    progress.runs_this_hour,
                    progress.hourly_budget,
                    self.text(Text::MemoryRuns)
                ))
                .text_size(px(12.)),
            )
            .child(
                muted(format!(
                    "{}: {} · {}",
                    self.text(Text::MemoryCheckpoints),
                    progress.session_summaries,
                    progress
                        .embedding_model
                        .as_ref()
                        .map(|model| format!(
                            "{} ({model}): {} / {}",
                            self.text(Text::MemorySemanticIndex),
                            progress.embedding_indexed,
                            progress.embedding_indexed
                                + progress.embedding_pending
                                + progress.embedding_failed
                        ))
                        .unwrap_or_else(|| self.text(Text::MemoryLexicalOnly).to_string())
                ))
                .text_size(px(12.)),
            )
            .children(
                progress
                    .embedding_error
                    .as_ref()
                    .map(|error| muted(error.clone()).text_size(px(12.))),
            )
            .child(
                row().gap(px(8.)).children(
                    [
                        (Text::MemorySources, progress.sources),
                        (Text::MemoryProcessed, progress.done),
                        (Text::MemoryPending, progress.pending),
                        (Text::MemoryFailed, progress.failed),
                        (Text::MemoryCandidate, progress.candidates),
                        (Text::MemoryConfirmed, progress.confirmed),
                    ]
                    .map(|(label, count)| {
                        column()
                            .flex_1()
                            .min_w_0()
                            .p(px(10.))
                            .rounded(px(9.))
                            .bg(rgb(SIDEBAR))
                            .child(div().text_size(px(21.)).child(count.to_string()))
                            .child(muted(self.text(label)).text_size(px(12.)))
                    }),
                ),
            )
    }
    fn detail_view(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = column()
            .id("memory-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(LINE));
        if let Some(id) = &self.archive {
            return panel.child(self.archive_view(id, cx));
        }
        if let Some((id, title, body)) = &self.editing {
            let id = *id;
            return panel
                .child(div().child(self.text(Text::MemoryEditHint)))
                .child(
                    Input::new(title)
                        .id("memory-edit-title")
                        .disabled(self.saving),
                )
                .child(
                    div()
                        .id("memory-edit-body")
                        .child(Textarea::new(body).disabled(self.saving)),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            Button::new("memory-save")
                                .label(self.text(Text::Save))
                                .disabled(self.saving)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some((_, title, body)) = &this.editing {
                                        let command = MemoryCommand::Revise {
                                            id,
                                            title: title.read(cx).value().to_string(),
                                            body: body.read(cx).value().to_string(),
                                        };
                                        this.command(command, window, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new("memory-cancel-edit")
                                .outline()
                                .label(self.text(Text::Cancel))
                                .disabled(self.saving)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.editing = None;
                                    cx.notify();
                                })),
                        ),
                );
        }
        match &self.detail {
            None => panel.child(muted(self.text(if self.loading {
                Text::Loading
            } else {
                Text::MemoryChoose
            }))),
            Some(Detail::Memory(detail)) => {
                let id = detail.entry.id;
                panel
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(detail.entry.title.clone()),
                    )
                    .child(muted(self.status(&detail.entry.status)).text_size(px(12.)))
                    .child(
                        div()
                            .text_size(px(14.))
                            .line_height(px(23.))
                            .child(detail.body.clone()),
                    )
                    .child(
                        row()
                            .flex_wrap()
                            .gap(px(6.))
                            .children(detail.topics.iter().map(|t| {
                                muted(t.clone())
                                    .px(px(8.))
                                    .py(px(4.))
                                    .bg(rgb(SIDEBAR))
                                    .rounded(px(6.))
                            })),
                    )
                    .child(
                        row()
                            .flex_wrap()
                            .gap(px(8.))
                            .when(detail.entry.status == "candidate", |r| {
                                r.child(
                                    Button::new("memory-confirm")
                                        .small()
                                        .disabled(self.saving)
                                        .label(self.text(Text::MemoryConfirm))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.command(MemoryCommand::Confirm(id), window, cx)
                                        })),
                                )
                            })
                            .child(
                                Button::new("memory-edit")
                                    .outline()
                                    .small()
                                    .disabled(self.saving)
                                    .label(self.text(Text::Edit))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.edit(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("memory-forget")
                                    .ghost()
                                    .small()
                                    .disabled(self.saving)
                                    .label(self.text(Text::MemoryForget))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.forget_dialog(false, id, window, cx)
                                    })),
                            ),
                    )
                    .child(div().mt(px(8.)).child(self.text(Text::MemoryEvidence)))
                    .children(detail.evidence.iter().map(|source| {
                        let id = source.id;
                        Button::new(("memory-evidence", id))
                            .ghost()
                            .h_auto()
                            .w_full()
                            .justify_start()
                            .child(
                                div()
                                    .w_full()
                                    .whitespace_normal()
                                    .text_left()
                                    .child(format!("{} · v{}", source.title, source.revision)),
                            )
                            .disabled(self.saving)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select(Selection::Source(id, 0), window, cx)
                            }))
                    }))
            }
            Some(Detail::Source(page)) => {
                let id = page.source.id;
                let next = page.next_offset;
                let offset = page.offset;
                panel
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(page.source.title.clone()),
                    )
                    .child(
                        muted(format!(
                            "{} · v{} · {}",
                            self.status(&page.source.state),
                            page.revision,
                            page.source.origin
                        ))
                        .text_size(px(12.)),
                    )
                    .when_some(page.source.error.clone(), |v, error| {
                        v.child(div().text_color(rgb(0xb04435)).child(error))
                    })
                    .child(
                        row()
                            .flex_wrap()
                            .gap(px(8.))
                            .when_some(page.archive.clone(), |r, archive| {
                                r.child(
                                    Button::new("memory-open-archive")
                                        .outline()
                                        .small()
                                        .label(self.text(Text::MemoryOpenArchive))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.error = this
                                                .archives
                                                .dispatch(ClientSessionsCommand::Open(
                                                    archive.clone(),
                                                ))
                                                .err();
                                            this.archive = Some(archive.clone());
                                            this.archive_page = 0;
                                            this.refresh_archives(cx);
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("memory-forget-source")
                                    .ghost()
                                    .small()
                                    .disabled(self.saving)
                                    .label(self.text(Text::MemoryForgetSource))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.forget_dialog(true, id, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(22.))
                            .child(page.body.clone()),
                    )
                    .child(
                        row()
                            .justify_between()
                            .child(
                                Button::new("memory-source-previous")
                                    .outline()
                                    .small()
                                    .disabled(offset == 0)
                                    .label(self.text(Text::MemoryPrevious))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select(
                                            Selection::Source(id, offset.saturating_sub(12000)),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new("memory-source-next")
                                    .outline()
                                    .small()
                                    .disabled(next.is_none())
                                    .label(self.text(Text::MemoryNext))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(next) = next {
                                            this.select(Selection::Source(id, next), window, cx);
                                        }
                                    })),
                            ),
                    )
            }
        }
    }
    fn archive_view(&self, id: &ClientSessionId, cx: &mut Context<Self>) -> Div {
        let view = column().gap(px(12.)).child(
            Button::new("memory-close-archive")
                .outline()
                .small()
                .label(self.text(Text::MemoryBackSource))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.archive = None;
                    cx.notify();
                })),
        );
        let Some(detail) = self
            .archive_state
            .detail
            .as_ref()
            .filter(|d| &d.session.id == id)
        else {
            return view.child(muted(
                self.archive_state
                    .error
                    .clone()
                    .unwrap_or_else(|| self.text(Text::ClientSessionsLoading).into()),
            ));
        };
        let length = detail.messages.len();
        let start = self.archive_page.min(length.saturating_sub(1) / 40) * 40;
        view.child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(detail.session.title.clone()),
        )
        .child(muted(self.text(Text::MemoryArchiveHint)).text_size(px(12.)))
        .when(!detail.session.available, |v| {
            v.child(muted(self.text(Text::MemoryArchiveMissing)).text_size(px(12.)))
        })
        .child(
            row()
                .gap(px(8.))
                .child(
                    Button::new("archive-previous")
                        .outline()
                        .small()
                        .disabled(start == 0)
                        .label(self.text(Text::MemoryPrevious))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.archive_page = this.archive_page.saturating_sub(1);
                            cx.notify();
                        })),
                )
                .child(
                    muted(format!(
                        "{}–{} / {}",
                        (start + 1).min(length),
                        (start + 40).min(length),
                        length
                    ))
                    .text_size(px(12.)),
                )
                .child(
                    Button::new("archive-next")
                        .outline()
                        .small()
                        .disabled(start + 40 >= length)
                        .label(self.text(Text::MemoryNext))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.archive_page += 1;
                            cx.notify();
                        })),
                ),
        )
        .children(detail.messages.iter().skip(start).take(40).map(|message| {
            let text = message.text.clone();
            column()
                .gap(px(8.))
                .p(px(12.))
                .bg(rgb(SIDEBAR))
                .rounded(px(8.))
                .child(
                    row()
                        .justify_between()
                        .child(
                            muted(if message.role == MessageRole::User {
                                "User"
                            } else {
                                "Assistant"
                            })
                            .text_size(px(12.)),
                        )
                        .child(
                            Button::new(("copy-memory-archive", message.id))
                                .ghost()
                                .small()
                                .label(self.text(Text::Copy))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                                }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(22.))
                        .child(message.text.chars().take(12000).collect::<String>()),
                )
                .when(message.text.chars().count() > 12000, |v| {
                    v.child(muted(self.text(Text::ClientSessionsTruncated)).text_size(px(12.)))
                })
        }))
    }
}

impl Render for MemoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.saving || self.editing.is_some();
        let mut list = column()
            .id("memory-list")
            .w(px(240.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .gap(px(6.));
        if self.mode == Mode::Memories {
            list = list
                .when(self.overview.memories.is_empty(), |v| {
                    v.child(
                        muted(self.text(if self.loading {
                            Text::Loading
                        } else {
                            Text::MemoryEmpty
                        }))
                        .p(px(12.))
                        .text_size(px(13.)),
                    )
                })
                .children(self.overview.memories.iter().map(|memory| {
                    let id = memory.id;
                    Button::new(("memory-entry", id))
                        .ghost()
                        .h_auto()
                        .w_full()
                        .justify_start()
                        .p(px(10.))
                        .disabled(busy)
                        .when(self.selection == Some(Selection::Memory(id)), |b| {
                            b.bg(rgb(0xededf0))
                        })
                        .child(
                            column()
                                .w_full()
                                .whitespace_normal()
                                .text_left()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .line_clamp(2)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(memory.title.clone()),
                                )
                                .child(muted(self.status(&memory.status)).text_size(px(12.)))
                                .child(
                                    muted(memory.preview.chars().take(100).collect::<String>())
                                        .line_clamp(3)
                                        .text_size(px(12.)),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select(Selection::Memory(id), window, cx)
                        }))
                }));
        } else {
            list = list
                .when(self.overview.sources.is_empty(), |v| {
                    v.child(
                        muted(self.text(if self.loading {
                            Text::Loading
                        } else {
                            Text::MemoryNoSources
                        }))
                        .p(px(12.))
                        .text_size(px(13.)),
                    )
                })
                .children(self.overview.sources.iter().map(|source| {
                    let id = source.id;
                    Button::new(("memory-source", id))
                        .ghost()
                        .h_auto()
                        .w_full()
                        .justify_start()
                        .p(px(10.))
                        .disabled(busy)
                        .when(matches!(self.selection, Some(Selection::Source(selected, _)) if selected == id), |b| b.bg(rgb(0xededf0)))
                        .child(
                            column()
                                .w_full()
                                .whitespace_normal()
                                .text_left()
                                .gap(px(6.))
                                .child(div().line_clamp(2).font_weight(FontWeight::MEDIUM).child(source.title.clone()))
                                .child(muted(self.status(&source.state)).text_size(px(12.)))
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select(Selection::Source(id, 0), window, cx)
                        }))
                }));
        }
        let next = if self.mode == Mode::Memories {
            self.overview.next_memory
        } else {
            self.overview.next_source
        };
        let previous = if self.mode == Mode::Memories {
            self.query.before_memory
        } else {
            self.query.before_source
        };
        list = list.child(
            row()
                .gap(px(6.))
                .mt(px(8.))
                .child(
                    Button::new("memory-first-page")
                        .outline()
                        .small()
                        .disabled(previous.is_none() || busy)
                        .label(self.text(Text::MemoryFirst))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.query.before_memory = None;
                            this.query.before_source = None;
                            this.reload(window, cx);
                        })),
                )
                .child(
                    Button::new("memory-next-page")
                        .outline()
                        .small()
                        .disabled(next.is_none() || busy)
                        .label(self.text(Text::MemoryNext))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.mode == Mode::Memories {
                                this.query.before_memory = next;
                            } else {
                                this.query.before_source = next;
                            }
                            this.reload(window, cx);
                        })),
                ),
        );
        let content = if self.mode == Mode::Topics {
            column()
                .id("memory-topics-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .child(muted(self.text(Text::MemoryTopicsHint)).text_size(px(13.)))
                .when(self.overview.topics.is_empty(), |v| {
                    v.child(muted(self.text(Text::MemoryNoTopics)).p(px(20.)))
                })
                .children(self.overview.topics.iter().map(|topic| {
                    let name = topic.name.clone();
                    Button::new(ElementId::Name(format!("memory-topic-{name}").into()))
                        .outline()
                        .h_auto()
                        .w_full()
                        .justify_start()
                        .p(px(14.))
                        .child(
                            row()
                                .w_full()
                                .gap(px(14.))
                                .child(div().flex_1().child(name.clone()))
                                .child(
                                    muted(format!(
                                        "{} {} · {} {}",
                                        topic.memories,
                                        self.text(Text::MemoryItems),
                                        topic.sessions,
                                        self.text(Text::MemorySessions)
                                    ))
                                    .text_size(px(12.)),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.query.topic = Some(name.clone());
                            this.mode = Mode::Memories;
                            this.query.before_memory = None;
                            this.query.before_source = None;
                            this.selection = None;
                            this.detail = None;
                            this.archive = None;
                            this.reload(window, cx);
                        }))
                }))
        } else {
            row()
                .id("memory-browser")
                .flex_1()
                .min_h_0()
                .items_start()
                .gap(px(12.))
                .child(list)
                .child(self.detail_view(cx))
        };
        column()
            .flex_1()
            .min_h_0()
            .h_full()
            .px(px(24.))
            .pb(px(18.))
            .gap(px(12.))
            .child(self.toolbar(cx))
            .when_some(
                self.error
                    .clone()
                    .or(self.load_error.clone())
                    .or(self.archive_state.error.clone())
                    .or(self.overview.progress.observer_error.clone()),
                |v, error| {
                    v.child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0xb04435))
                            .child(error),
                    )
                },
            )
            .when(!self.overview.failures.is_empty(), |v| {
                v.child(
                    row()
                        .gap(px(8.))
                        .child(
                            Button::new("memory-failure-detail")
                                .ghost()
                                .small()
                                .label(self.text(Text::MemoryShowFailure))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(source) = this.overview.failures.first() {
                                        this.select(Selection::Source(source.id, 0), window, cx);
                                    }
                                })),
                        )
                        .child(
                            muted(self.overview.failures[0].error.clone().unwrap_or_default())
                                .flex_1()
                                .truncate()
                                .text_size(px(12.)),
                        )
                        .child(
                            Button::new("memory-retry")
                                .outline()
                                .small()
                                .disabled(
                                    busy || (self.overview.progress.failed == 0
                                        && self.overview.progress.embedding_error.is_none()),
                                )
                                .label(self.text(Text::MemoryRetry))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.command(MemoryCommand::RetryFailed, window, cx)
                                })),
                        ),
                )
            })
            .child(
                row()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(
                        [
                            ("memory-memories-tab", Mode::Memories, Text::MemoryItems),
                            ("memory-topics-tab", Mode::Topics, Text::MemoryTopics),
                            ("memory-sources-tab", Mode::Sources, Text::MemorySources),
                        ]
                        .map(|(id, mode, label)| {
                            Button::new(id)
                                .ghost()
                                .selected(self.mode == mode)
                                .disabled(busy)
                                .label(self.text(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.mode = mode;
                                    cx.notify();
                                }))
                        }),
                    )
                    .when_some(self.query.topic.clone(), |r, topic| {
                        r.child(
                            Button::new("memory-clear-topic")
                                .outline()
                                .small()
                                .disabled(busy)
                                .label(format!("{topic} ×"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.query.topic = None;
                                    this.query.before_memory = None;
                                    this.query.before_source = None;
                                    this.reload(window, cx);
                                })),
                        )
                    })
                    .when(self.mode != Mode::Topics, |r| {
                        r.child(
                            Input::new(&self.search)
                                .id("memory-search")
                                .disabled(busy)
                                .flex_1()
                                .min_w(px(160.)),
                        )
                    }),
            )
            .child(content)
    }
}
