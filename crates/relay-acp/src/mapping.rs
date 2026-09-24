use crate::Event;
use agent_client_protocol::schema::v1 as acp;
use relay_core::agents::{
    ConfigChoice, PermissionChoice, PermissionRequest, SessionConfig, TokenUsage, ToolActivity,
};
use serde_json::Value;

pub fn usage(value: acp::Usage) -> TokenUsage {
    TokenUsage {
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        cached_read_tokens: value.cached_read_tokens,
        cached_write_tokens: value.cached_write_tokens,
        thought_tokens: value.thought_tokens,
    }
}

pub fn configs(options: &[acp::SessionConfigOption]) -> Vec<SessionConfig> {
    options
        .iter()
        .filter_map(|option| config_value(&serde_json::to_value(option).ok()?))
        .collect()
}

fn config_value(value: &Value) -> Option<SessionConfig> {
    // Only advertise support for select options; boolean config requires separate negotiation.
    if value["type"] != "select" {
        return None;
    }
    let mut choices = Vec::new();
    for item in value["options"].as_array()? {
        if let Some(options) = item["options"].as_array() {
            for option in options {
                if let Some(choice) = choice_value(option, item["name"].as_str()) {
                    choices.push(choice);
                }
            }
        } else if let Some(choice) = choice_value(item, None) {
            choices.push(choice);
        }
    }
    Some(SessionConfig {
        id: value["id"].as_str()?.into(),
        name: value["name"].as_str()?.into(),
        category: value["category"].as_str().map(str::to_owned),
        current: value["currentValue"].as_str()?.into(),
        choices,
    })
}

fn choice_value(value: &Value, group: Option<&str>) -> Option<ConfigChoice> {
    Some(ConfigChoice {
        id: value["value"].as_str()?.into(),
        name: value["name"].as_str()?.into(),
        description: value["description"].as_str().map(str::to_owned),
        group: group.map(str::to_owned),
    })
}

pub fn notification(update: acp::SessionUpdate) -> Option<Event> {
    match update {
        acp::SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
            acp::ContentBlock::Text(text) => Some(Event::Text(text.text)),
            _ => None,
        },
        acp::SessionUpdate::ConfigOptionUpdate(update) => Some(Event::Configs {
            configs: configs(&update.config_options),
            confirmed: None,
        }),
        acp::SessionUpdate::ToolCall(call) => tool(serde_json::to_value(call).ok()?),
        acp::SessionUpdate::ToolCallUpdate(call) => tool(serde_json::to_value(call).ok()?),
        _ => None,
    }
}

fn tool(value: Value) -> Option<Event> {
    Some(Event::Tool(ToolActivity {
        id: value["toolCallId"].as_str()?.into(),
        title: value["title"].as_str().unwrap_or("").into(),
        status: value["status"].as_str().unwrap_or("").into(),
    }))
}

pub fn permission(id: u64, request: &acp::RequestPermissionRequest) -> PermissionRequest {
    let tool = serde_json::to_value(&request.tool_call).unwrap_or_default();
    let detail = tool["rawInput"]
        .as_object()
        .map(|_| serde_json::to_string_pretty(&tool["rawInput"]).unwrap_or_default())
        .unwrap_or_else(|| {
            tool["locations"]
                .as_array()
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(|p| p["path"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        });
    PermissionRequest {
        id,
        title: tool["title"]
            .as_str()
            .unwrap_or("Codex requests permission")
            .into(),
        detail,
        choices: request
            .options
            .iter()
            .map(|option| {
                let value = serde_json::to_value(option).unwrap_or_default();
                PermissionChoice {
                    id: option.option_id.to_string(),
                    name: option.name.clone(),
                    allows: value["kind"]
                        .as_str()
                        .is_some_and(|kind| kind.starts_with("allow_")),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_model_ids_and_current_value_are_preserved() {
        let option = serde_json::json!({"id":"provider_model", "name":"Model", "category":"model", "type":"select", "currentValue":"tenant/model-x", "options":[{"group":"tenant", "name":"My gateway", "options":[{"value":"tenant/model-x", "name":"Private model"}]}]});
        let model = config_value(&option).unwrap();
        assert_eq!(model.id, "provider_model");
        assert_eq!(model.current_name(), "Private model");
        assert_eq!(model.choices[0].group.as_deref(), Some("My gateway"));
        assert!(config_value(&serde_json::json!({"type":"boolean"})).is_none());
    }
}
