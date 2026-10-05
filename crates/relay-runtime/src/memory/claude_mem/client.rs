use anyhow::{Result, ensure};
use serde_json::Value;
use std::time::Duration;

pub(super) struct Client {
    base: String,
    agent: ureq::Agent,
}
impl Client {
    pub(super) fn new(base: &str) -> Self {
        Self {
            base: base.into(),
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(8)))
                .proxy(None)
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }
    fn url(&self, path: &str, params: &[(String, String)]) -> String {
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!(
            "{}{path}{}{query}",
            self.base,
            if query.is_empty() { "" } else { "?" }
        )
    }
    pub(super) fn text(&self, path: &str, params: &[(String, String)]) -> Result<String> {
        let mut response =
            self.agent.get(self.url(path, params)).call().map_err(|_| {
                anyhow::anyhow!("Claude-Mem Worker unavailable or request timed out")
            })?;
        ensure!(
            response.status().is_success(),
            "Claude-Mem {path} returned HTTP {}",
            response.status().as_u16()
        );
        response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|_| anyhow::anyhow!("Invalid or oversized Claude-Mem response"))
    }
    pub(super) fn get(&self, path: &str, params: &[(String, String)]) -> Result<Value> {
        Ok(serde_json::from_str(&self.text(path, params)?)?)
    }
    pub(super) fn health(&self) -> Result<Value> {
        let health = self.get("/api/health", &[])?;
        ensure!(
            health["status"] == "ok"
                && health["initialized"] == true
                && health["version"].is_string(),
            "Claude-Mem Worker is not initialized or its health response is incompatible"
        );
        Ok(health)
    }
    /// Once a POST is attempted, a transport/5xx failure has an unknown outcome.
    pub(super) fn post(&self, path: &str, payload: &Value) -> Result<(u16, Value)> {
        let mut response = self
            .agent
            .post(format!("{}{path}", self.base))
            .send_json(payload)
            .map_err(|_| {
                anyhow::anyhow!(
                    "Worker POST outcome unknown (connection or timeout); inspect before replaying"
                )
            })?;
        let status = response.status().as_u16();
        let value = response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_json()
            .map_err(|_| {
                anyhow::anyhow!(
                    "Worker POST outcome unknown (invalid response); inspect before replaying"
                )
            })?;
        Ok((status, value))
    }
    pub(super) fn delete(&self, path: &str) -> Result<Value> {
        let mut response = self
            .agent
            .delete(format!("{}{path}", self.base))
            .call()
            .map_err(|_| {
                anyhow::anyhow!("Worker deletion outcome unknown; refresh before retrying")
            })?;
        ensure!(
            response.status().is_success(),
            "Worker deletion returned HTTP {}",
            response.status().as_u16()
        );
        Ok(response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_json()?)
    }
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
