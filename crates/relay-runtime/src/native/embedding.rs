//! OpenAI-compatible embedding transport. Credentials never enter SQLite or MCP.
use super::memory::digest;
use super::*;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub dimensions: Option<usize>,
    #[serde(default = "default_chunk_chars")]
    pub chunk_chars: usize,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}
fn default_chunk_chars() -> usize {
    2048
}
fn default_batch_size() -> usize {
    16
}

impl EmbeddingConfig {
    pub(super) fn validate(&self) -> Result<()> {
        let uri: ureq::http::Uri = self
            .base_url
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid embedding Base URL"))?;
        let local = matches!(uri.host(), Some("127.0.0.1" | "localhost" | "[::1]"));
        ensure!(
            uri.scheme_str() == Some("https") || (local && uri.scheme_str() == Some("http")),
            "Embedding Base URL requires HTTPS (HTTP is allowed for localhost)"
        );
        ensure!(
            uri.host().is_some() && uri.query().is_none() && !self.base_url.contains(['@', '#']),
            "Embedding Base URL must not contain credentials, query parameters or fragments"
        );
        ensure!(
            !self.model.trim().is_empty()
                && self.model.len() <= 200
                && !self.model.contains(['\n', '\r']),
            "Invalid embedding model"
        );
        ensure!(
            (256..=8192).contains(&self.chunk_chars) && (1..=20).contains(&self.batch_size),
            "Embedding chunk_chars must be 256–8192 and batch_size 1–20"
        );
        ensure!(
            self.dimensions.is_none_or(|d| (1..=8192).contains(&d)),
            "Invalid embedding dimensions"
        );
        Ok(())
    }
    pub(super) fn profile(&self) -> String {
        digest(&format!(
            "relay-embedding-v1\n{}\n{}\n{:?}\n{}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.dimensions,
            self.chunk_chars
        ))
    }
    pub(super) fn load(root: &Path) -> Result<Option<Self>> {
        let saved = match std::fs::read(root.join("embedding.json")) {
            Ok(bytes) => Some(
                serde_json::from_slice::<Self>(&bytes)
                    .context("Could not parse embedding configuration")?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => bail!("Could not read embedding configuration"),
        };
        let config = match (
            std::env::var("RELAY_EMBEDDING_BASE_URL").ok(),
            std::env::var("RELAY_EMBEDDING_MODEL").ok(),
        ) {
            (Some(base_url), Some(model)) => Some(Self {
                base_url,
                model,
                dimensions: saved.as_ref().and_then(|s| s.dimensions),
                chunk_chars: default_chunk_chars(),
                batch_size: default_batch_size(),
            }),
            (None, None) => saved,
            _ => bail!("Set both RELAY_EMBEDDING_BASE_URL and RELAY_EMBEDDING_MODEL"),
        };
        if let Some(c) = &config {
            c.validate()?;
        }
        Ok(config)
    }
}

/// Explicit user configuration. Call off the UI thread; pass a key via stdin, not argv.
pub fn configure_embeddings(
    root: &Path,
    config: &EmbeddingConfig,
    key: Option<&str>,
) -> Result<()> {
    config.validate()?;
    if let Some(key) = key {
        save_key(root, config, validate_key(key)?)?;
    }
    std::fs::create_dir_all(root)?;
    let temporary = root.join(format!(".embedding-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(config)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, root.join("embedding.json"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

const SERVICE: &str = "com.arcwright42.relay.memory.embedding";
fn account(root: &Path, config: &EmbeddingConfig) -> String {
    digest(&format!(
        "{}\n{}",
        root.display(),
        config.base_url.trim_end_matches('/')
    ))
}
fn validate_key(key: &str) -> Result<&str> {
    let key = key.trim();
    ensure!(
        !key.is_empty() && key.len() <= 4096 && key.bytes().all(|b| b.is_ascii_graphic()),
        "Invalid embedding API key"
    );
    Ok(key)
}
#[cfg(target_os = "macos")]
fn save_key(root: &Path, config: &EmbeddingConfig, key: &str) -> Result<()> {
    security_framework::passwords::set_generic_password(
        SERVICE,
        &account(root, config),
        key.as_bytes(),
    )
    .map_err(|_| anyhow::anyhow!("Could not save embedding API key to macOS Keychain"))
}
#[cfg(not(target_os = "macos"))]
fn save_key(_: &Path, _: &EmbeddingConfig, _: &str) -> Result<()> {
    bail!("Use RELAY_EMBEDDING_API_KEY on this platform")
}
fn read_key(root: &Path, config: &EmbeddingConfig) -> Result<String> {
    if let Ok(key) = std::env::var("RELAY_EMBEDDING_API_KEY") {
        return Ok(validate_key(&key)?.to_owned());
    }
    #[cfg(target_os = "macos")]
    match security_framework::passwords::get_generic_password(SERVICE, &account(root, config)) {
        Ok(bytes) => {
            let key = String::from_utf8(bytes).context("Invalid embedding Keychain entry")?;
            return Ok(validate_key(&key)?.to_owned());
        }
        Err(e) if e.code() == -25300 => {}
        Err(_) => bail!("Could not read embedding API key from macOS Keychain"),
    }
    bail!("Embedding API key is not configured")
}

pub(super) fn embed(
    root: &Path,
    config: &EmbeddingConfig,
    inputs: &[String],
    timeout: Duration,
) -> Result<Vec<Vec<f32>>> {
    let key = read_key(root, config)?;
    request_embeddings(config, &key, inputs, timeout)
}

pub(super) fn request_embeddings(
    config: &EmbeddingConfig,
    key: &str,
    inputs: &[String],
    timeout: Duration,
) -> Result<Vec<Vec<f32>>> {
    config.validate()?;
    validate_key(key)?;
    ensure!(
        !inputs.is_empty()
            && inputs.len() <= config.batch_size
            && inputs.iter().all(|s| !s.trim().is_empty()),
        "Invalid embedding batch"
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let mut payload = json!({"model":config.model,"input":inputs,"encoding_format":"float"});
    if let Some(d) = config.dimensions {
        payload["dimensions"] = json!(d);
    }
    let mut response = agent
        .post(format!(
            "{}/embeddings",
            config.base_url.trim_end_matches('/')
        ))
        .header("Authorization", format!("Bearer {key}"))
        .send_json(payload)
        .map_err(|_| anyhow::anyhow!("Embedding request failed or timed out"))?;
    ensure!(
        response.status().is_success(),
        "Embedding service returned HTTP {}",
        response.status().as_u16()
    );
    let value: Value = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_json()
        .map_err(|_| anyhow::anyhow!("Invalid embedding response JSON"))?;
    decode_embeddings(&value, inputs.len(), config.dimensions)
}

fn decode_embeddings(
    value: &Value,
    count: usize,
    dimensions: Option<usize>,
) -> Result<Vec<Vec<f32>>> {
    let rows = value["data"]
        .as_array()
        .context("Embedding response has no data array")?;
    ensure!(rows.len() == count, "Embedding response omitted inputs");
    let mut result = vec![None; count];
    let mut expected = dimensions;
    for row in rows {
        let i: usize = row["index"]
            .as_u64()
            .context("Embedding index missing")?
            .try_into()?;
        ensure!(
            i < count && result[i].is_none(),
            "Duplicate or invalid embedding index"
        );
        let raw = row["embedding"]
            .as_array()
            .context("Expected float embedding data")?;
        ensure!(
            !raw.is_empty() && raw.len() <= 8192 && expected.is_none_or(|n| n == raw.len()),
            "Embedding dimension mismatch"
        );
        expected = Some(raw.len());
        let mut vector = raw
            .iter()
            .map(|v| {
                v.as_f64()
                    .filter(|v| v.is_finite())
                    .map(|v| v as f32)
                    .context("Invalid embedding coordinate")
            })
            .collect::<Result<Vec<_>>>()?;
        let norm = vector
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        ensure!(norm.is_finite() && norm > 0.0, "Invalid embedding norm");
        for v in &mut vector {
            *v = (f64::from(*v) / norm) as f32;
        }
        result[i] = Some(vector);
    }
    result
        .into_iter()
        .map(|v| v.context("Missing embedding input"))
        .collect()
}

pub(super) fn chunks(title: &str, body: &str, maximum: usize) -> Vec<String> {
    let text: Vec<char> = format!("{title}\n{body}").chars().collect();
    let mut out = vec![];
    let mut start = 0;
    while start < text.len() {
        let end = (start + maximum).min(text.len());
        out.push(text[start..end].iter().collect());
        if end == text.len() {
            break;
        }
        start = end - 128.min(maximum / 4);
    }
    out
}

impl NativeStore {
    pub(super) fn embedding_profile(&self, config: &EmbeddingConfig) -> Result<String> {
        let profile = config.profile();
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT value FROM embedding_state WHERE key='profile'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if old.as_deref() != Some(&profile) {
            tx.execute("DELETE FROM memory_embeddings", [])?;
            tx.execute("DELETE FROM embedding_jobs", [])?;
            tx.execute("INSERT INTO embedding_jobs(memory_id) SELECT id FROM memories WHERE status IN ('candidate','confirmed')", [])?;
            tx.execute(
                "INSERT OR REPLACE INTO embedding_state VALUES('profile',?)",
                [&profile],
            )?;
            tx.execute(
                "DELETE FROM embedding_state WHERE key IN ('error','retry_at')",
                [],
            )?;
        }
        tx.commit()?;
        Ok(profile)
    }

    /// Runs on its own worker so API latency cannot stall task dispatch or capture.
    pub fn index_embedding_batch(&self) -> Result<usize> {
        self.index_embedding_batch_with(|config, inputs| {
            embed(&self.root, config, inputs, Duration::from_secs(20))
        })
    }

    fn index_embedding_batch_with(
        &self,
        mut request: impl FnMut(&EmbeddingConfig, &[String]) -> Result<Vec<Vec<f32>>>,
    ) -> Result<usize> {
        let Some(config) = EmbeddingConfig::load(&self.root)? else {
            return Ok(0);
        };
        let profile = self.embedding_profile(&config)?;
        let job = {
            let mut db = self.db.lock().expect("native store");
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("UPDATE embedding_jobs SET state=CASE WHEN attempts>=5 THEN 'failed' ELSE 'pending' END WHERE state='running' AND lease_until<unixepoch()", [])?;
            let backoff: bool = tx.query_row("SELECT coalesce((SELECT CAST(value AS INTEGER) FROM embedding_state WHERE key='retry_at'),0)>unixepoch()",[],|r|r.get(0))?;
            if backoff {
                return Ok(0);
            }
            let row = tx.query_row("SELECT j.memory_id,m.title,m.body||char(10)||m.attributes,j.attempts+1 FROM embedding_jobs j JOIN memories m ON m.id=j.memory_id WHERE j.state='pending' AND j.retry_at<=unixepoch() AND m.status IN ('candidate','confirmed') ORDER BY j.memory_id LIMIT 1", [], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?,uuid::Uuid::new_v4().to_string()))).optional()?;
            if let Some((id, _, _, _, token)) = &row {
                tx.execute("UPDATE embedding_jobs SET state='running',attempts=attempts+1,lease_until=unixepoch()+180,lease_token=? WHERE memory_id=?", params![token,id])?;
            }
            tx.commit()?;
            row
        };
        let Some((id, title, body, attempt, token)) = job else {
            return Ok(0);
        };
        let result = (|| -> Result<Vec<Vec<f32>>> {
            let mut vectors = vec![];
            let started = std::time::Instant::now();
            for batch in chunks(&title, &body, config.chunk_chars).chunks(config.batch_size) {
                ensure!(
                    started.elapsed() < Duration::from_secs(120),
                    "Embedding job exceeded its time budget"
                );
                vectors.extend(request(&config, batch)?);
            }
            ensure!(
                !vectors.is_empty() && vectors.iter().all(|v| v.len() == vectors[0].len()),
                "Embedding dimensions changed between batches"
            );
            Ok(vectors)
        })();
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM embedding_jobs j JOIN memories m ON m.id=j.memory_id WHERE j.memory_id=?1 AND j.state='running' AND j.attempts=?2 AND j.lease_token=?4 AND j.lease_until>=unixepoch() AND m.status IN ('candidate','confirmed')) AND (SELECT value FROM embedding_state WHERE key='profile')=?3", params![id,attempt,profile,token], |r| r.get(0))?;
        if !current {
            return Ok(0);
        }
        match result {
            Ok(vectors) => {
                tx.execute("DELETE FROM memory_embeddings WHERE memory_id=?", [id])?;
                for (part, vector) in vectors.iter().enumerate() {
                    let bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
                    tx.execute(
                        "INSERT INTO memory_embeddings VALUES(?,?,?,?,?)",
                        params![id, part, profile, vector.len(), bytes],
                    )?;
                }
                tx.execute("UPDATE embedding_jobs SET state='done',lease_until=NULL,error=NULL WHERE memory_id=?", [id])?;
                tx.execute(
                    "DELETE FROM embedding_state WHERE key IN ('error','retry_at')",
                    [],
                )?;
                tx.commit()?;
                Ok(1)
            }
            Err(error) => {
                let delay = 5_u64 * (1_u64 << attempt.min(6));
                tx.execute("UPDATE embedding_jobs SET state=CASE WHEN attempts>=5 THEN 'failed' ELSE 'pending' END,lease_until=NULL,retry_at=unixepoch()+?,error=? WHERE memory_id=?", params![delay,error.to_string(),id])?;
                tx.execute(
                    "INSERT OR REPLACE INTO embedding_state VALUES('error',?)",
                    [error.to_string()],
                )?;
                tx.execute(
                    "INSERT OR REPLACE INTO embedding_state VALUES('retry_at',unixepoch()+?)",
                    [delay],
                )?;
                tx.commit()?;
                Err(error)
            }
        }
    }

    pub fn embedding_status(&self) -> Result<Value> {
        let config = EmbeddingConfig::load(&self.root);
        let db = self.db.lock().expect("native store");
        let mut stmt = db.prepare("SELECT state,count(*) FROM embedding_jobs GROUP BY state")?;
        let counts = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
            .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?;
        let error: Option<String> = db
            .query_row(
                "SELECT value FROM embedding_state WHERE key='error'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match config {
            Ok(c) => Ok(
                json!({"configured":c.is_some(),"model":c.map(|c| c.model),"jobs":counts,"error":error}),
            ),
            Err(e) => Ok(json!({"configured":false,"jobs":counts,"error":e.to_string()})),
        }
    }

    pub fn retry_embeddings(&self) -> Result<()> {
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE embedding_jobs SET state='pending',attempts=0,retry_at=0,error=NULL,lease_token=NULL WHERE state='failed' OR (state='pending' AND error IS NOT NULL)",[])?;
        tx.execute(
            "DELETE FROM embedding_state WHERE key IN ('error','retry_at')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn rebuild_embeddings(&self) -> Result<()> {
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM memory_embeddings", [])?;
        tx.execute("DELETE FROM embedding_jobs", [])?;
        tx.execute("INSERT INTO embedding_jobs(memory_id) SELECT id FROM memories WHERE status IN ('candidate','confirmed')",[])?;
        tx.execute(
            "DELETE FROM embedding_state WHERE key IN ('error','retry_at')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Fixed diagnostic strings; configuration checks never open or migrate user data.
pub(super) fn check(root: &Path) -> Result<Value> {
    let config = EmbeddingConfig::load(root)?.context("Embedding service is not configured")?;
    let vectors = embed(
        root,
        &config,
        &[
            "本地记忆与会话恢复".into(),
            "Local memory and session recovery".into(),
        ],
        Duration::from_secs(30),
    )?;
    Ok(
        json!({"model":config.model,"dimensions":vectors[0].len(),"inputs":vectors.len(),"cross_language_similarity":vectors[0].iter().zip(&vectors[1]).map(|(a,b)|f64::from(*a)*f64::from(*b)).sum::<f64>()}),
    )
}

#[cfg(test)]
mod live;
#[cfg(test)]
mod tests;
