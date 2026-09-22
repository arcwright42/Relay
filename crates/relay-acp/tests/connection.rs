#![cfg(feature = "test-support")]
use relay_acp::{Command, ConnectionHandle, Event, LaunchSpec, SessionOptions};
use relay_core::agents::TurnOutcome;
use std::{
    collections::BTreeMap,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

fn fixture(saved: Option<String>) -> (ConnectionHandle, mpsc::Receiver<Event>) {
    fixture_with_options(SessionOptions {
        saved_session: saved,
        ..Default::default()
    })
}

fn fixture_with_options(options: SessionOptions) -> (ConnectionHandle, mpsc::Receiver<Event>) {
    fixture_config(options, false)
}

fn fixture_config(
    options: SessionOptions,
    requires_auth: bool,
) -> (ConnectionHandle, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let spec = LaunchSpec {
        command: env!("CARGO_BIN_EXE_relay-acp-fixture").into(),
        args: vec![],
        env: if requires_auth {
            BTreeMap::from([("RELAY_FIXTURE_AUTH".into(), "1".into())])
        } else {
            BTreeMap::new()
        },
    };
    let handle = relay_acp::connect(
        spec,
        std::env::temp_dir(),
        options,
        Arc::new(move |event| {
            let _ = tx.send(event);
        }),
    )
    .unwrap();
    (handle, rx)
}

#[test]
fn sign_in_opens_the_session_and_restores_the_model_preference() {
    let (connection, events) = fixture_config(
        SessionOptions {
            saved_session: None,
            preferences: BTreeMap::from([("deployment".into(), "private-b".into())]),
        },
        true,
    );
    let Event::AuthenticationRequired(methods) =
        wait(&events, |e| matches!(e, Event::AuthenticationRequired(_)))
    else {
        unreachable!()
    };
    assert_eq!(methods[0].id, "browser");
    connection
        .send(Command::Authenticate(methods[0].id.clone()))
        .unwrap();
    let Event::Ready { configs, .. } = wait(&events, |e| matches!(e, Event::Ready { .. })) else {
        unreachable!()
    };
    assert_eq!(configs[0].current, "private-b");
}

#[test]
fn saved_model_is_restored_before_the_session_becomes_ready() {
    let (_connection, events) = fixture_with_options(SessionOptions {
        saved_session: None,
        preferences: BTreeMap::from([
            ("deployment".into(), "private-b".into()),
            ("removed".into(), "obsolete".into()),
        ]),
    });
    let Event::Ready { configs, .. } = wait(&events, |e| matches!(e, Event::Ready { .. })) else {
        unreachable!()
    };
    assert_eq!(configs[0].current, "private-b");
}
fn wait(rx: &mpsc::Receiver<Event>, predicate: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("ACP event before deadline");
        if predicate(&event) {
            return event;
        }
    }
}

#[test]
fn authoritative_model_selection_and_rejected_change() {
    let (connection, events) = fixture(None);
    let Event::Ready { configs, .. } = wait(&events, |e| matches!(e, Event::Ready { .. })) else {
        unreachable!()
    };
    assert_eq!(configs[0].current, "private-a");
    connection
        .send(Command::SetConfig {
            id: "deployment".into(),
            value: "private-b".into(),
        })
        .unwrap();
    let Event::Configs { configs, .. } = wait(&events, |e| {
        matches!(
            e,
            Event::Configs {
                confirmed: Some(_),
                ..
            }
        )
    }) else {
        unreachable!()
    };
    assert_eq!(configs[0].current, "private-b");
    connection
        .send(Command::SetConfig {
            id: "deployment".into(),
            value: "rejected".into(),
        })
        .unwrap();
    assert!(matches!(
        wait(&events, |e| matches!(e, Event::Error { .. })),
        Event::Error { fatal: false, .. }
    ));
}

#[test]
fn permissions_wait_for_the_user_and_preserve_their_choice() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "permission".into(),
            context: None,
        })
        .unwrap();
    let Event::Permission(request) = wait(&events, |e| matches!(e, Event::Permission(_))) else {
        unreachable!()
    };
    assert!(
        events.recv_timeout(Duration::from_millis(100)).is_err(),
        "permission was answered without user input"
    );
    connection
        .send(Command::Permission {
            id: request.id,
            choice: Some("no".into()),
        })
        .unwrap();
    let Event::Text(result) = wait(&events, |e| matches!(e, Event::Text(_))) else {
        unreachable!()
    };
    assert!(result.contains("no"));
    wait(&events, |e| {
        matches!(
            e,
            Event::TurnEnded {
                outcome: TurnOutcome::Complete,
                ..
            }
        )
    });
}

#[test]
fn cancellation_is_processed_while_prompt_is_in_flight() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "wait".into(),
            context: None,
        })
        .unwrap();
    wait(&events, |e| matches!(e, Event::Text(_)));
    connection.send(Command::Cancel).unwrap();
    wait(&events, |e| {
        matches!(
            e,
            Event::TurnEnded {
                outcome: TurnOutcome::Cancelled,
                ..
            }
        )
    });
}

#[test]
fn native_restore_does_not_duplicate_relays_transcript() {
    let (_connection, events) = fixture(Some("session-1".into()));
    let first = events.recv_timeout(Duration::from_secs(8)).unwrap();
    assert!(matches!(first, Event::Ready { resumed: true, .. }));
}

#[test]
fn dropping_connection_terminates_the_agent_process() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "pid".into(),
            context: None,
        })
        .unwrap();
    let Event::Text(pid) = wait(&events, |e| matches!(e, Event::Text(_))) else {
        unreachable!()
    };
    assert!(pid.parse::<u32>().is_ok());
    drop(connection);
    let started = Instant::now();
    loop {
        let alive = std::process::Command::new("/bin/kill")
            .args(["-0", &pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "agent process survived connection close"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn unexpected_agent_exit_is_reported_as_a_fatal_connection_error() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "exit".into(),
            context: None,
        })
        .unwrap();
    wait(&events, |e| matches!(e, Event::Error { fatal: true, .. }));
}

#[test]
fn cancelling_a_turn_dismisses_a_pending_permission() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "permission".into(),
            context: None,
        })
        .unwrap();
    wait(&events, |e| matches!(e, Event::Permission(_)));
    connection.send(Command::Cancel).unwrap();
    wait(&events, |e| {
        matches!(
            e,
            Event::TurnEnded {
                outcome: TurnOutcome::Cancelled,
                ..
            }
        )
    });
    connection
        .send(Command::Prompt {
            text: "next".into(),
            context: None,
        })
        .unwrap();
    wait(&events, |e| {
        matches!(
            e,
            Event::TurnEnded {
                outcome: TurnOutcome::Complete,
                ..
            }
        )
    });
}

#[test]
fn project_context_precedes_new_message_and_cache_usage_is_preserved() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "echo-context".into(),
            context: Some("Stable project context\n资料".into()),
        })
        .unwrap();
    let Event::Text(text) = wait(&events, |e| matches!(e, Event::Text(_))) else {
        unreachable!()
    };
    let prompt: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(prompt[0]["text"], "Stable project context\n资料");
    assert_eq!(prompt[1]["text"], "echo-context");
    let Event::TurnEnded { outcome, usage } =
        wait(&events, |e| matches!(e, Event::TurnEnded { .. }))
    else {
        unreachable!()
    };
    assert_eq!(outcome, TurnOutcome::Complete);
    let usage = usage.unwrap();
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.cached_read_tokens, Some(900));
    assert_eq!(usage.cached_write_tokens, None);
    assert_eq!(usage.cache_read_ratio(), Some(0.9));
}

#[test]
fn refusal_is_distinct_and_malformed_optional_usage_does_not_break_a_turn() {
    let (connection, events) = fixture(None);
    wait(&events, |e| matches!(e, Event::Ready { .. }));
    connection
        .send(Command::Prompt {
            text: "refuse".into(),
            context: None,
        })
        .unwrap();
    assert!(matches!(
        wait(&events, |e| matches!(e, Event::TurnEnded { .. })),
        Event::TurnEnded {
            outcome: TurnOutcome::Refused,
            usage: None
        }
    ));
    connection
        .send(Command::Prompt {
            text: "bad-usage".into(),
            context: None,
        })
        .unwrap();
    assert!(
        matches!(wait(&events, |e| matches!(e, Event::Text(_))), Event::Text(text) if text == "Still usable")
    );
    assert!(matches!(
        wait(&events, |e| matches!(e, Event::TurnEnded { .. })),
        Event::TurnEnded {
            outcome: TurnOutcome::Complete,
            usage: None
        }
    ));
}
