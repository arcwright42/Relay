use super::{delivery::Event, *};

impl ClaudeMemProvider {
    pub(super) fn events(&self, event: &MemoryEvent) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        match event {
            MemoryEvent::User {
                thread,
                message,
                text,
                ..
            } if *thread != crate::resident::RETIRED_OBSERVER => {
                self.user(
                    &mut events,
                    &self.session(*thread),
                    &Self::platform(*thread),
                    *message,
                    text,
                    "Relay conversation",
                );
            }
            MemoryEvent::Tool {
                thread,
                response,
                tool,
                ..
            } if *thread != crate::resident::RETIRED_OBSERVER => {
                self.tool(
                    &mut events,
                    &self.session(*thread),
                    &Self::platform(*thread),
                    *response,
                    tool,
                );
            }
            MemoryEvent::Turn {
                thread, messages, ..
            } if *thread != crate::resident::RETIRED_OBSERVER => {
                let session = self.session(*thread);
                let platform = Self::platform(*thread);
                for message in messages {
                    if message.role == "user" {
                        self.user(
                            &mut events,
                            &session,
                            &platform,
                            message.id,
                            &message.text,
                            "Relay conversation",
                        );
                    } else if message.role == "assistant" {
                        for tool in &message.tools {
                            self.tool(&mut events, &session, &platform, message.id, tool);
                        }
                        if message.complete {
                            self.assistant(
                                &mut events,
                                &session,
                                &platform,
                                message.id,
                                &message.text,
                            );
                        }
                    }
                }
            }
            MemoryEvent::Archive {
                client,
                session,
                title,
                messages,
                ..
            } if self.config.import_history => {
                let session = format!(
                    "{}-archive-{}",
                    self.project,
                    digest(&format!("{client}\n{session}"))
                );
                let mut initialized = false;
                for (id, role, text) in messages {
                    if role == "user" {
                        self.user(&mut events, &session, "relay-archive", *id, text, title);
                        initialized = true;
                    } else if role == "assistant" && initialized {
                        self.assistant(&mut events, &session, "relay-archive", *id, text);
                    }
                }
            }
            MemoryEvent::SessionEnd {
                thread,
                last_message,
            } if *thread != crate::resident::RETIRED_OBSERVER => {
                let session = self.session(*thread);
                events.push(Event {
                    key: format!("{session}:end:{last_message}"),
                    session: session.clone(),
                    path: "/api/sessions/session-end",
                    payload: json!({
                        "contentSessionId":session,"platformSource":Self::platform(*thread)
                    }),
                });
            }
            _ => {}
        }
        let directory = match event {
            MemoryEvent::User { directory, .. }
            | MemoryEvent::Tool { directory, .. }
            | MemoryEvent::Turn { directory, .. } => Some(directory),
            MemoryEvent::Archive { directory, .. } => directory.as_ref(),
            MemoryEvent::SessionEnd { .. } => None,
        };
        if let Some(directory) = directory {
            for event in &mut events {
                event.payload["cwd"] = json!(directory);
            }
        }
        Ok(events)
    }

    fn session(&self, thread: ThreadId) -> String {
        format!("{}-thread-{}", self.project, thread.0)
    }

    fn user(
        &self,
        events: &mut Vec<Event>,
        session: &str,
        platform: &str,
        id: u64,
        text: &str,
        title: &str,
    ) {
        events.push(Event {key:format!("{session}:user:{id}"),session:session.into(),path:"/api/sessions/init",payload:json!({
            "contentSessionId":session,"project":self.project,"prompt":text,"platformSource":platform,"customTitle":title
        })});
    }

    fn tool(
        &self,
        events: &mut Vec<Event>,
        session: &str,
        platform: &str,
        response: u64,
        tool: &MemoryToolEvent,
    ) {
        if !matches!(tool.status.as_str(), "completed" | "failed" | "cancelled") {
            return;
        }
        let key = format!("{session}:tool:{response}:{}", tool.id);
        let input =
            serde_json::from_str::<Value>(&tool.input).unwrap_or_else(|_| json!(tool.input));
        events.push(Event {key:key.clone(),session:session.into(),path:"/api/sessions/observations",payload:json!({
            "contentSessionId":session,"platformSource":platform,"tool_name":tool.title,"tool_input":input,
            "tool_response":{"status":tool.status,"output":tool.output},"tool_use_id":key
        })});
    }

    fn assistant(
        &self,
        events: &mut Vec<Event>,
        session: &str,
        platform: &str,
        id: u64,
        text: &str,
    ) {
        if !text.trim().is_empty() {
            let key = format!("{session}:assistant:{id}");
            events.push(Event {key:key.clone(),session:session.into(),path:"/api/sessions/observations",payload:json!({
                "contentSessionId":session,"platformSource":platform,"tool_name":"RelayAssistantResponse",
                "tool_input":{"message_id":id},"tool_response":text,"tool_use_id":key
            })});
        }
        events.push(Event {
            key: format!("{session}:summary:{id}"),
            session: session.into(),
            path: "/api/sessions/summarize",
            payload: json!({
                "contentSessionId":session,"platformSource":platform,"last_assistant_message":text
            }),
        });
    }
}
