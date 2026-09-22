//! A deterministic, dependency-free ACP peer for subprocess integration tests.
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

fn emit(value: Value) {
    println!("{value}");
    io::stdout().flush().unwrap();
}
fn config(current: &str) -> Value {
    json!([{"id":"deployment", "name":"Model", "category":"model", "type":"select", "currentValue":current,
        "options":[{"value":"private-a", "name":"Private A"},{"value":"private-b", "name":"Private B"},{"value":"rejected", "name":"Rejected by server"}]}])
}
fn chunk(text: &str) {
    emit(
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}}}),
    );
}
fn reply(id: Value, result: Value) {
    emit(json!({"jsonrpc":"2.0","id":id,"result":result}));
}

fn main() {
    let mut current = "private-a".to_owned();
    let mut pending = None;
    let mut authenticated = std::env::var_os("RELAY_FIXTURE_AUTH").is_none();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let request: Value = serde_json::from_str(&line).unwrap();
        let id = request["id"].clone();
        match request["method"].as_str() {
            Some("initialize") => reply(
                id,
                json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true},"authMethods":[{"id":"browser", "name":"Browser sign-in"}],"agentInfo":{"name":"fixture","version":"1"}}),
            ),
            Some("authenticate") => {
                authenticated = true;
                reply(id, json!({}));
            }
            Some("session/new" | "session/load") if !authenticated => {
                emit(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"Sign in first"}}),
                );
            }
            Some("session/new") => reply(
                id,
                json!({"sessionId":"session-1","configOptions":config(&current)}),
            ),
            Some("session/load") => {
                chunk("DO NOT DUPLICATE RESTORED HISTORY");
                reply(id, json!({"configOptions":config(&current)}));
            }
            Some("session/set_config_option") => {
                if request["params"]["value"] == "rejected" {
                    emit(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Model unavailable"}}),
                    );
                } else {
                    current = request["params"]["value"].as_str().unwrap().into();
                    reply(id, json!({"configOptions":config(&current)}));
                }
            }
            Some("session/prompt") => {
                let text = request["params"]["prompt"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()["text"]
                    .as_str()
                    .unwrap();
                match text {
                    "echo-context" => {
                        chunk(&request["params"]["prompt"].to_string());
                        reply(
                            id,
                            json!({"stopReason":"end_turn","usage":{"totalTokens":1020,"inputTokens":100,"outputTokens":20,"cachedReadTokens":900,"thoughtTokens":10}}),
                        );
                    }
                    "refuse" => reply(id, json!({"stopReason":"refusal"})),
                    "bad-usage" => {
                        chunk("Still usable");
                        reply(
                            id,
                            json!({"stopReason":"end_turn","usage":{"inputTokens":"invalid"}}),
                        );
                    }
                    "exit" => std::process::exit(0),
                    "permission" => {
                        pending = Some(id);
                        emit(
                            json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission","params":{"sessionId":"session-1","toolCall":{"toolCallId":"tool-1","title":"Write example.txt"},"options":[{"optionId":"yes","name":"Allow once","kind":"allow_once"},{"optionId":"no","name":"Reject","kind":"reject_once"}]}}),
                        );
                    }
                    "wait" => {
                        pending = Some(id);
                        chunk("Partial response");
                    }
                    "pid" => {
                        chunk(&std::process::id().to_string());
                        reply(id, json!({"stopReason":"end_turn"}));
                    }
                    _ => {
                        chunk("Hello ");
                        chunk("Relay");
                        reply(id, json!({"stopReason":"end_turn"}));
                    }
                }
            }
            Some("session/cancel") => {
                if let Some(prompt) = pending.take() {
                    reply(prompt, json!({"stopReason":"cancelled"}));
                }
            }
            None if request["id"] == "permission-1" => {
                if let Some(prompt) = pending.take() {
                    chunk(&request["result"]["outcome"].to_string());
                    reply(prompt, json!({"stopReason":"end_turn"}));
                }
            }
            _ => {}
        }
    }
}
