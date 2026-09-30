use super::*;
use relay_core::{ProjectId, projects::ProjectDraft, routing::*};

pub(super) struct RoutingUi {
    provider: RoutingProvider,
    pub draft: Entity<TextareaState>,
    pub key: Entity<InputState>,
    pub busy: bool,
    pub creating: bool,
    pub saving_key: bool,
    pub navigation_revision: u64,
    generation: u64,
    decision: Option<RouteDecision>,
    error: Option<RoutingError>,
    key_error: Option<RoutingError>,
    local_error: Option<String>,
    review: bool,
    pub pending_send: Option<(ProjectId, String)>,
}

impl RoutingUi {
    pub fn new(
        language: Language,
        provider: RoutingProvider,
        window: &mut Window,
        cx: &mut Context<Workbench>,
    ) -> Self {
        Self {
            provider,
            draft: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(language.text(Text::AskRelay))
                    .auto_grow(2, 6)
            }),
            key: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("API key")
                    .masked(true)
            }),
            busy: false,
            creating: false,
            saving_key: false,
            navigation_revision: 0,
            generation: 0,
            decision: None,
            error: None,
            key_error: None,
            local_error: None,
            review: false,
            pending_send: None,
        }
    }

    pub fn reset_decision(&mut self) {
        self.generation += 1;
        self.busy = false;
        self.decision = None;
        self.error = None;
        self.local_error = None;
        self.review = false;
    }
}

impl Workbench {
    fn clear_routing_key_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.routing.key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API key")
                .masked(true)
        });
        self._subscriptions.push(cx.subscribe_in(
            &self.routing.key,
            window,
            |_, _, _: &InputEvent, _, cx| cx.notify(),
        ));
    }

    pub(super) fn route_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.routing.draft.read(cx).value().to_string();
        if prompt.trim().is_empty()
            || self.routing.busy
            || self.routing.creating
            || self.routing.saving_key
        {
            return;
        }
        self.routing.reset_decision();
        self.routing.busy = true;
        let generation = self.routing.generation;
        let navigation_revision = self.routing.navigation_revision;
        let service = self.routing_service.clone();
        let request = cx
            .background_executor()
            .spawn(async move { service.decide(&prompt) });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.routing.generation != generation {
                    return;
                }
                this.routing.busy = false;
                this.routing.review = true;
                match result {
                    Ok(decision) => {
                        let target = decision.automatic;
                        let revision = decision.catalog_revision;
                        this.routing.decision = Some(decision);
                        if this.page == Page::Home
                            && this.routing.navigation_revision == navigation_revision
                            && let Some(target) = target
                        {
                            this.apply_route(target, revision, window, cx);
                        }
                    }
                    Err(error) => this.routing.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_route(&mut self, target: RouteTarget, window: &mut Window, cx: &mut Context<Self>) {
        if self.routing.busy || self.routing.creating {
            return;
        }
        // An explicit choice is evaluated against the current catalog, never an old API result.
        self.apply_route(target, self.project_service.revision(), window, cx);
    }

    fn apply_route(
        &mut self,
        target: RouteTarget,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project_service.revision() != revision {
            self.routing.error = Some(RoutingError::CatalogChanged);
            self.routing.review = true;
            return;
        }
        let prompt = self.routing.draft.read(cx).value().to_string();
        if prompt.trim().is_empty() {
            return;
        }
        match target {
            RouteTarget::Existing(id) => self.deliver_routed_prompt(id, prompt, window, cx),
            RouteTarget::NewProject => {
                self.routing.creating = true;
                self.routing.local_error = None;
                let command = ProjectCommand::CreateAtRevision {
                    expected_catalog_revision: revision,
                    draft: ProjectDraft {
                        name: prompt
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(60)
                            .collect(),
                        description: prompt.chars().take(600).collect(),
                        instructions: String::new(),
                    },
                };
                let service = self.project_service.clone();
                let navigation_revision = self.routing.navigation_revision;
                let request = cx
                    .background_executor()
                    .spawn(async move { service.apply(command) });
                cx.spawn_in(window, async move |this, cx| {
                    let result = request.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.routing.creating = false;
                        match result {
                            Ok(id) => {
                                this.refresh_projects(window, cx);
                                if this.page == Page::Home
                                    && this.routing.navigation_revision == navigation_revision
                                {
                                    this.deliver_routed_prompt(id, prompt, window, cx);
                                } else {
                                    // Do not navigate or execute after the user leaves Home.
                                    this.routing.decision = Some(RouteDecision {
                                        catalog_revision: this.project_revision,
                                        automatic: None,
                                        options: vec![RouteOption {
                                            target: RouteTarget::Existing(id),
                                            probability: 1.0,
                                        }],
                                        confidence: 0.0,
                                        elapsed_ms: 0,
                                        model: String::new(),
                                    });
                                    this.routing.review = true;
                                }
                            }
                            Err(error) => {
                                this.routing.local_error = Some(error);
                                this.routing.review = true;
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    fn deliver_routed_prompt(
        &mut self,
        id: ProjectId,
        prompt: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_projects(window, cx);
        let Some(index) = self.projects.iter().position(|p| p.id == id) else {
            self.routing.error = Some(RoutingError::CatalogChanged);
            return;
        };
        if !self.drafts[index].read(cx).value().trim().is_empty() {
            self.routing.local_error = Some(self.text(Text::RoutingDraftConflict).into());
            self.routing.review = true;
            return;
        }
        self.drafts[index].update(cx, |draft, cx| draft.set_value(prompt.clone(), window, cx));
        self.routing
            .draft
            .update(cx, |draft, cx| draft.set_value("", window, cx));
        self.routing.reset_decision();
        self.navigate(Page::Project(index), window, cx);
        self.refresh_agents(cx);
        let state = self.agent_service.snapshot(id);
        if matches!(
            state.status,
            ConnectionStatus::Running | ConnectionStatus::Cancelling
        ) {
            // Keep a draft without unexpectedly sending it when an unrelated turn ends.
            return;
        }
        self.routing.pending_send = Some((id, prompt));
        if matches!(
            state.status,
            ConnectionStatus::Disconnected | ConnectionStatus::Failed
        ) && !self.agent_action(AgentCommand::Connect(state.source), cx)
        {
            self.routing.pending_send = None;
        }
        self.advance_routed_send(window, cx);
    }

    pub(super) fn advance_routed_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, prompt)) = self.routing.pending_send.as_ref() else {
            return;
        };
        if !matches!(self.page, Page::Project(_))
            || self.projects[self.selected_project].id != *id
            || self.drafts[self.selected_project].read(cx).value().as_ref() != prompt
        {
            self.routing.pending_send = None;
            return;
        }
        let state = &self.agent_states[self.selected_project];
        if matches!(
            state.status,
            ConnectionStatus::Failed
                | ConnectionStatus::Disconnected
                | ConnectionStatus::Running
                | ConnectionStatus::Cancelling
        ) {
            self.routing.pending_send = None;
        } else if state.status == ConnectionStatus::Ready
            && state.pending_config.is_none()
            && (window.root::<Root>().flatten().is_none() || !window.has_active_dialog(cx))
        {
            self.routing.pending_send = None; // consume before dispatch, including failure
            self.send_message(&SendMessage, window, cx);
        }
    }

    pub(super) fn routing_composer(&self, cx: &mut Context<Self>) -> Div {
        let unavailable = self.routing.draft.read(cx).value().trim().is_empty();
        let busy = self.routing.busy || self.routing.creating || self.routing.saving_key;
        let status = if self.routing.creating {
            Text::RoutingCreating
        } else if self.routing.busy {
            Text::RoutingWorking
        } else if self.routing_service.snapshot().configured {
            Text::RoutingHint
        } else {
            Text::RoutingManualHint
        };
        column()
            .gap(px(12.))
            .child(
                composer_surface()
                    .gap(px(10.))
                    .child(
                        column().id("home-prompt").test_support().child(
                            Textarea::new(&self.routing.draft)
                                .appearance(false)
                                .bordered(false)
                                .text_size(px(15.))
                                .h(px(68.))
                                .disabled(busy)
                                .aria_label(self.text(Text::AskRelay))
                                .context_menu(crate::locale::input_menu),
                        ),
                    )
                    .child(
                        row()
                            .justify_between()
                            .gap(px(12.))
                            .child(
                                row()
                                    .gap(px(7.))
                                    .pl(px(5.))
                                    .child(
                                        icon(IconName::FolderClosed)
                                            .size(px(15.))
                                            .text_color(rgb(MUTED)),
                                    )
                                    .child(muted(self.text(status)).text_size(px(12.))),
                            )
                            .child(
                                row()
                                    .gap(px(8.))
                                    .when(self.routing.busy, |view| {
                                        view.child(
                                            Button::new("cancel-routing")
                                                .ghost()
                                                .label(self.text(Text::Cancel))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.routing.reset_decision();
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    .child(
                                        Button::new("route-prompt")
                                            .primary()
                                            .icon(
                                                icon(IconName::ArrowUp)
                                                    .size(px(18.))
                                                    .text_color(rgb(0xffffff)),
                                            )
                                            .size(px(32.))
                                            .rounded_full()
                                            .tooltip(self.text(Text::SendTooltip))
                                            .accessibility_label(self.text(Text::SendMessage))
                                            .disabled(
                                                busy || unavailable || self.project_error.is_some(),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.route_prompt(window, cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .when_some(self.routing.error, |view, error| {
                view.child(muted(self.text(routing_error_text(error))).text_size(px(13.)))
            })
            .when_some(self.routing.local_error.as_ref(), |view, error| {
                view.child(muted(error.clone()).text_size(px(13.)))
            })
            .when(self.routing.review && !busy && !unavailable, |view| {
                let mut options: Vec<RouteTarget> = self
                    .routing
                    .decision
                    .as_ref()
                    .map(|d| d.options.iter().map(|o| o.target).collect())
                    .unwrap_or_default();
                for project in &self.projects {
                    let target = RouteTarget::Existing(project.id);
                    if !options.contains(&target) {
                        options.push(target);
                    }
                }
                if !options.contains(&RouteTarget::NewProject) {
                    options.push(RouteTarget::NewProject);
                }
                view.child(
                    column()
                        .gap(px(8.))
                        .child(muted(self.text(Text::RoutingChoose)).text_size(px(13.)))
                        .children(
                            options
                                .into_iter()
                                .enumerate()
                                .filter_map(|(index, target)| {
                                    let name = match target {
                                        RouteTarget::NewProject => {
                                            self.text(Text::NewProject).to_owned()
                                        }
                                        RouteTarget::Existing(id) => {
                                            self.projects.iter().find(|p| p.id == id)?.name.clone()
                                        }
                                    };
                                    Some(
                                        Button::new(("route-choice", index))
                                            .outline()
                                            .label(name)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.choose_route(target, window, cx)
                                            })),
                                    )
                                }),
                        ),
                )
            })
    }

    fn save_routing_key(&mut self, remove: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.routing.saving_key {
            return;
        }
        self.routing.saving_key = true;
        self.routing.key_error = None;
        self.routing.reset_decision();
        let key = self.routing.key.read(cx).value().to_string();
        let provider = self.routing.provider;
        let service = self.routing_service.clone();
        let task = cx.background_executor().spawn(async move {
            if remove {
                service.remove_key()
            } else {
                service.save_key(provider, key)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.routing.saving_key = false;
                this.routing.key_error = result.err();
                if result.is_ok() {
                    // Replacing the input also discards undo history containing the secret.
                    this.clear_routing_key_input(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn routing_settings(&self, cx: &mut Context<Self>) -> Div {
        let snapshot = self.routing_service.snapshot();
        let configured = snapshot.configured && snapshot.provider == self.routing.provider;
        let external = matches!(
            snapshot.credential_source,
            Some(RoutingCredentialSource::Environment | RoutingCredentialSource::EnvFile)
        );
        let provider = self.routing.provider;
        column()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(LINE))
            .child(
                row()
                    .justify_between()
                    .child(
                        row().gap(px(6.))
                            .child(div().font_weight(FontWeight::MEDIUM).child(self.text(Text::RoutingSettings)))
                            .child(icon_button("routing-info", IconName::Info, self.text(Text::RoutingHelp))
                                .size(px(24.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    explain(
                                        this.text(Text::RoutingHelp),
                                        format!("{}\n\n{}\n\n{}", this.text(Text::RoutingSettingsDetail), this.text(Text::RoutingProviderHint), this.text(Text::RoutingDataDetail)),
                                        window, cx,
                                    );
                                }))),
                    )
                    .child(
                        muted(self.text(if configured {
                            match snapshot.credential_source {
                                Some(RoutingCredentialSource::Environment) => Text::RoutingFromEnvironment,
                                Some(RoutingCredentialSource::EnvFile) => Text::RoutingFromEnvFile,
                                _ => Text::RoutingConfigured,
                            }
                        } else {
                            Text::RoutingUnconfigured
                        }))
                        .text_size(px(12.)),
                    ),
            )
            .child(row().gap(px(8.)).children([RoutingProvider::OpenRouter, RoutingProvider::Vercel, RoutingProvider::TypeSafe].into_iter().map(|provider| {
                Button::new(provider.code()).outline().small().label(provider.name())
                    .disabled(self.routing.saving_key || external)
                    .when(provider == self.routing.provider, |button| button.primary().icon(IconName::Check))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.routing.provider = provider;
                        this.clear_routing_key_input(window, cx);
                        this.routing.key_error = None;
                        cx.notify();
                    }))
            })))
            .when(external, |view| view.child(muted(self.text(Text::RoutingExternalCredential)).text_size(px(12.))))
            .child(
                Input::new(&self.routing.key)
                    .id("jev-api-key")
                    .disabled(self.routing.saving_key || external)
                    .aria_label("Jev API key")
                    .context_menu(crate::locale::input_menu),
            )
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        Button::new("save-jev-key")
                            .primary()
                            .small()
                            .label(self.text(Text::Save))
                            .disabled(
                                self.routing.saving_key
                                    || external
                                    || self.routing.key.read(cx).value().trim().is_empty(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_routing_key(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("remove-jev-key")
                            .outline()
                            .small()
                            .label(self.text(Text::Remove))
                        .disabled(self.routing.saving_key || external || !configured)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_routing_key(true, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(Button::new("get-jev-key").ghost().small().label(self.text(Text::RoutingGetKey))
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(match provider {
                            RoutingProvider::OpenRouter => "https://openrouter.ai/settings/keys",
                            RoutingProvider::TypeSafe => "https://console.typesafe.ai/keys",
                            RoutingProvider::Vercel => "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys",
                        })))),
            )
            .when_some(self.routing.key_error.or(snapshot.error), |view, error| {
                view.child(muted(self.text(routing_error_text(error))).text_size(px(12.)))
            })
    }
}

fn routing_error_text(error: RoutingError) -> Text {
    match error {
        RoutingError::NotConfigured => Text::RoutingMissingKey,
        RoutingError::InvalidKey => Text::RoutingInvalidKey,
        RoutingError::ConfigurationFile => Text::RoutingConfigurationFileError,
        RoutingError::ExternallyConfigured => Text::RoutingExternalCredential,
        RoutingError::Keychain => Text::RoutingKeychainError,
        RoutingError::Unauthorized => Text::RoutingUnauthorized,
        RoutingError::RateLimited => Text::RoutingRateLimited,
        RoutingError::QuotaExceeded => Text::RoutingQuotaExceeded,
        RoutingError::Unavailable => Text::RoutingUnavailable,
        RoutingError::InvalidResponse => Text::RoutingInvalidResponse,
        RoutingError::InputTooLarge => Text::RoutingInputTooLarge,
        RoutingError::CatalogUnavailable => Text::RoutingCatalogUnavailable,
        RoutingError::CatalogChanged => Text::RoutingCatalogChanged,
    }
}
