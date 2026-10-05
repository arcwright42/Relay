use super::*;

fn entry(row: &Value) -> Result<MemoryEntry> {
    Ok(MemoryEntry {
        id: row["id"].as_u64().context("Observation ID missing")?,
        title: row["title"].as_str().unwrap_or("Observation").into(),
        preview: row["subtitle"]
            .as_str()
            .or_else(|| row["narrative"].as_str())
            .unwrap_or("")
            .chars()
            .take(240)
            .collect(),
        kind: row["type"].as_str().unwrap_or("observation").into(),
    })
}

fn strings(value: &Value) -> Vec<String> {
    let parsed = value
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    parsed
        .as_ref()
        .unwrap_or(value)
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl MemoryService for ClaudeMemProvider {
    fn provider(&self) -> MemoryProviderInfo {
        MemoryProviderInfo { id:"claude-mem".into(),name:"Claude-Mem".into(),
            capabilities:MemoryCapabilities {forget_memory:true,retry:true},
            description:"Claude-Mem manages observations, summaries and retrieval. Relay durably queues session events for delivery.".into() }
    }

    fn overview(&self, query: &MemoryQuery) -> Result<MemoryOverview, String> {
        (|| -> Result<_> {
            let mut overview = MemoryOverview::default();
            let status = self.status()?;
            let counts = &status["delivery"]["counts"];
            overview.progress = MemoryProgress {
                pending:counts["pending"].as_u64().unwrap_or(0),
                running:counts["sending"].as_u64().unwrap_or(0),
                accepted:counts["accepted"].as_u64().unwrap_or(0),
                failed:counts["failed"].as_u64().unwrap_or(0),
                uncertain:counts["uncertain"].as_u64().unwrap_or(0),
                skipped:counts["skipped"].as_u64().unwrap_or(0),
                provider_status:Some(format!("Worker: {} · queued {} · accepted {} · failed {} · uncertain {} · skipped {}. Accepted ≠ extracted/indexed. {}",
                    if status["reachable"] == true {"reachable"} else {"offline"}, counts["pending"].as_u64().unwrap_or(0), counts["accepted"].as_u64().unwrap_or(0), counts["failed"].as_u64().unwrap_or(0),counts["uncertain"].as_u64().unwrap_or(0),counts["skipped"].as_u64().unwrap_or(0),
                    status["error"].as_str().unwrap_or("Model and embedding settings are managed by Claude-Mem."))),
                error:status["error"].as_str().map(str::to_owned),
            };
            // Keep delivery health visible even while the Worker is unavailable.
            if status["reachable"] != true { return Ok(overview); }
            // External cursor is an offset, deliberately opaque to the UI.
            let offset = query.before_memory.unwrap_or(0);
            let (rows,more) = if query.text.trim().is_empty() { self.list(crate::resident::MAIN,"observations",25,offset)? }
                else { let rows = self.search(crate::resident::MAIN,&query.text,25,offset)?; let more=rows.len()==25;(rows,more) };
            overview.memories = rows.iter().map(entry).collect::<Result<Vec<_>>>()?;
            overview.next_memory = more.then_some(offset+25);
            Ok(overview)
        })().map_err(|e| e.to_string())
    }

    fn detail(&self, id: u64) -> Result<MemoryDetail, String> {
        (|| -> Result<_> {
            let row = self.observation(crate::resident::MAIN, id)?;
            let facts = strings(&row["facts"]);
            let body = format!(
                "{}\n\n{}",
                row["narrative"]
                    .as_str()
                    .or_else(|| row["text"].as_str())
                    .unwrap_or(""),
                facts.join("\n")
            );
            Ok(MemoryDetail {
                entry: entry(&row)?,
                body,
                concepts: strings(&row["concepts"]),
            })
        })()
        .map_err(|e| e.to_string())
    }
    fn apply(&self, command: MemoryCommand) -> Result<(), String> {
        (|| -> Result<_> {
            match command {
                MemoryCommand::RetryFailed => self.outbox.retry()?,
                MemoryCommand::ForgetMemory(id) => {
                    self.call_tool(crate::resident::MAIN, "memory_forget", &json!({"id":id}))?;
                }
            }
            Ok(())
        })()
        .map_err(|e| e.to_string())
    }
}
