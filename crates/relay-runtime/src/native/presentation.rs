use super::*;
use relay_core::memory::*;

impl MemoryService for NativeStore {
    fn overview(&self, query: &MemoryQuery) -> Result<MemoryOverview, String> {
        self.memory_overview(query).map_err(|e| format!("{e:#}"))
    }
    fn detail(&self, id: u64) -> Result<MemoryDetail, String> {
        self.memory_detail(id).map_err(|e| format!("{e:#}"))
    }
    fn source(&self, id: u64, offset: usize) -> Result<MemorySourcePage, String> {
        self.memory_source_page(id, offset)
            .map_err(|e| format!("{e:#}"))
    }
    fn apply(&self, command: MemoryCommand) -> Result<Option<u64>, String> {
        self.memory_action(command).map_err(|e| format!("{e:#}"))
    }
}

fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    Ok(MemoryEntry {
        id: row.get(0)?,
        title: row.get(1)?,
        preview: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
    })
}

fn source_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemorySourceEntry> {
    Ok(MemorySourceEntry {
        id: row.get(0)?,
        title: row.get(1)?,
        origin: row.get(2)?,
        state: row.get(3)?,
        error: row.get(4)?,
    })
}

impl NativeStore {
    fn memory_overview(&self, query: &MemoryQuery) -> Result<MemoryOverview> {
        ensure!(
            query.text.chars().count() <= 400,
            "Search is limited to 400 characters"
        );
        let embedding = self.embedding_status()?;
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction()?;
        let count = |sql: &str| -> Result<u64> { Ok(tx.query_row(sql, [], |r| r.get(0))?) };
        let mut progress = MemoryProgress {
            observer_error: self.observer_error.lock().expect("observer health").clone(),
            session_summaries: count(
                "SELECT count(*) FROM session_summaries c JOIN memories m ON m.id=c.memory_id WHERE m.status IN ('candidate','confirmed')",
            )?,
            embedding_model: embedding["model"].as_str().map(str::to_owned),
            embedding_indexed: embedding["jobs"]["done"].as_u64().unwrap_or(0),
            embedding_failed: embedding["jobs"]["failed"].as_u64().unwrap_or(0),
            embedding_pending: embedding["jobs"]["pending"].as_u64().unwrap_or(0)
                + embedding["jobs"]["running"].as_u64().unwrap_or(0),
            embedding_error: embedding["error"].as_str().map(str::to_owned),
            sources: count("SELECT count(*) FROM sources WHERE forgotten=0 AND retired=0")?,
            sessions: count(
                "SELECT count(DISTINCT CASE WHEN thread_id IS NULL THEN origin ELSE 'thread:'||thread_id END) FROM sources WHERE forgotten=0 AND retired=0",
            )?,
            candidates: count("SELECT count(*) FROM memories WHERE status='candidate'")?,
            confirmed: count("SELECT count(*) FROM memories WHERE status='confirmed'")?,
            enabled: count("SELECT value FROM meta WHERE key='observer_enabled'")? != 0,
            hourly_budget: count("SELECT value FROM meta WHERE key='observer_budget'")?,
            runs_this_hour: count(
                "SELECT CASE WHEN (SELECT value FROM meta WHERE key='observer_window')>unixepoch()-3600 THEN value ELSE 0 END FROM meta WHERE key='observer_runs'",
            )?,
            ..Default::default()
        };
        let mut stmt = tx.prepare("SELECT j.state,count(*) FROM jobs j JOIN sources s ON s.id=j.source_id AND s.revision=j.source_revision WHERE s.forgotten=0 AND s.retired=0 GROUP BY j.state")?;
        for result in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
            let (state, count) = result?;
            match state.as_str() {
                "pending" => progress.pending = count,
                "running" => progress.running = count,
                "done" => progress.done = count,
                "failed" => progress.failed = count,
                _ => {}
            }
        }
        // Literal substring search intentionally does not imply semantic retrieval.
        let text = query.text.trim();
        let mut stmt = tx.prepare("SELECT m.id,m.title,substr(m.body,1,240),m.kind,m.status FROM memories m WHERE m.status IN ('candidate','confirmed') AND (?1='' OR instr(lower(m.title),lower(?1))>0 OR instr(lower(m.body),lower(?1))>0) AND (?2 IS NULL OR EXISTS(SELECT 1 FROM memory_topics mt JOIN topics t ON t.id=mt.topic_id WHERE mt.memory_id=m.id AND t.name=?2)) AND (?3 IS NULL OR m.id<?3) ORDER BY m.id DESC LIMIT 26")?;
        let mut memories = stmt
            .query_map(params![text, query.topic, query.before_memory], entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next_memory = (memories.len() > 25).then(|| memories[24].id);
        memories.truncate(25);
        let mut stmt = tx.prepare("SELECT t.name,count(DISTINCT m.id),count(DISTINCT CASE WHEN s.thread_id IS NULL THEN s.origin ELSE 'thread:'||s.thread_id END) FROM topics t JOIN memory_topics mt ON mt.topic_id=t.id JOIN memories m ON m.id=mt.memory_id JOIN evidence e ON e.memory_id=m.id JOIN sources s ON s.id=e.source_id AND s.revision=e.source_revision WHERE m.status IN ('candidate','confirmed') AND s.forgotten=0 AND s.retired=0 GROUP BY t.id ORDER BY count(DISTINCT m.id) DESC,t.name LIMIT 80")?;
        let topics = stmt
            .query_map([], |r| {
                Ok(MemoryTopic {
                    name: r.get(0)?,
                    memories: r.get(1)?,
                    sessions: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = tx.prepare("SELECT s.id,s.title,s.origin,coalesce(j.state,'recorded'),j.error FROM sources s LEFT JOIN jobs j ON j.source_id=s.id AND j.source_revision=s.revision AND j.kind='observation' WHERE s.forgotten=0 AND s.retired=0 AND (?1='' OR instr(lower(s.title),lower(?1))>0 OR instr(lower(s.body),lower(?1))>0) AND (?2 IS NULL OR EXISTS(SELECT 1 FROM evidence e JOIN memories m ON m.id=e.memory_id JOIN memory_topics mt ON mt.memory_id=m.id JOIN topics t ON t.id=mt.topic_id WHERE e.source_id=s.id AND e.source_revision=s.revision AND m.status IN ('candidate','confirmed') AND t.name=?2)) AND (?3 IS NULL OR s.id<?3) ORDER BY s.id DESC LIMIT 26")?;
        let mut sources = stmt
            .query_map(
                params![text, query.topic, query.before_source],
                source_entry,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next_source = (sources.len() > 25).then(|| sources[24].id);
        sources.truncate(25);
        let mut stmt = tx.prepare("SELECT s.id,s.title,s.origin,j.state,j.error FROM jobs j JOIN sources s ON s.id=j.source_id AND s.revision=j.source_revision WHERE s.forgotten=0 AND s.retired=0 AND j.error IS NOT NULL AND j.state IN ('failed','pending') ORDER BY j.id DESC LIMIT 8")?;
        let failures = stmt
            .query_map([], source_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(MemoryOverview {
            progress,
            memories,
            topics,
            sources,
            failures,
            next_memory,
            next_source,
        })
    }

    fn memory_detail(&self, id: u64) -> Result<MemoryDetail> {
        let db = self.db.lock().expect("native store");
        let entry = db.query_row("SELECT id,title,substr(body,1,240),kind,status FROM memories WHERE id=? AND status IN ('candidate','confirmed')",[id],entry).context("Memory was revised, invalidated or forgotten; refresh the list")?;
        let body = db.query_row("SELECT body FROM memories WHERE id=?", [id], |r| r.get(0))?;
        let mut stmt = db.prepare("SELECT t.name FROM topics t JOIN memory_topics mt ON mt.topic_id=t.id WHERE mt.memory_id=? ORDER BY t.name")?;
        let topics = stmt
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = db.prepare("SELECT s.id,s.revision,s.title,s.origin FROM sources s JOIN evidence e ON e.source_id=s.id AND e.source_revision=s.revision WHERE e.memory_id=? AND s.forgotten=0 AND s.retired=0 ORDER BY s.id")?;
        let evidence = stmt
            .query_map([id], |r| {
                Ok(MemoryEvidence {
                    id: r.get(0)?,
                    revision: r.get(1)?,
                    title: r.get(2)?,
                    origin: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(!evidence.is_empty(), "Memory has no current source");
        Ok(MemoryDetail {
            entry,
            body,
            topics,
            evidence,
        })
    }

    fn memory_source_page(&self, id: u64, offset: usize) -> Result<MemorySourcePage> {
        let db = self.db.lock().expect("native store");
        let source = db.query_row("SELECT s.id,s.title,s.origin,coalesce(j.state,'recorded'),j.error FROM sources s LEFT JOIN jobs j ON j.source_id=s.id AND j.source_revision=s.revision AND j.kind='observation' WHERE s.id=? AND s.forgotten=0 AND s.retired=0",[id],source_entry).context("Source was withdrawn or forgotten")?;
        let (body, revision): (String, u64) =
            db.query_row("SELECT body,revision FROM sources WHERE id=?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let body = readable_source(&body);
        let length = body.chars().count();
        ensure!(
            offset <= length,
            "Source changed; reopen it from the beginning"
        );
        let archive = source
            .origin
            .strip_prefix("client:")
            .map(|s| relay_core::sessions::ClientSessionId(s.into()));
        Ok(MemorySourcePage {
            source,
            revision,
            body: body.chars().skip(offset).take(12000).collect(),
            offset,
            next_offset: (offset + 12000 < length).then_some(offset + 12000),
            archive,
        })
    }

    fn memory_action(&self, command: MemoryCommand) -> Result<Option<u64>> {
        match command {
            MemoryCommand::ForgetMemory(id) => self.forget(MAIN, None, Some(id))?,
            MemoryCommand::ForgetSource(id) => self.forget(MAIN, Some(id), None)?,
            MemoryCommand::SetEnabled(enabled) => {
                self.db.lock().expect("native store").execute(
                    "UPDATE meta SET value=? WHERE key='observer_enabled'",
                    [enabled as u64],
                )?;
            }
            MemoryCommand::RetryFailed => {
                self.retry_embeddings()?;
                self.db.lock().expect("native store").execute("UPDATE jobs SET state='pending',retry_attempts=0,error=NULL,lease_until=NULL WHERE state='failed' AND EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id AND s.revision=source_revision AND s.forgotten=0 AND s.retired=0)",[])?;
            }
            MemoryCommand::Confirm(id) | MemoryCommand::Revise { id, .. } => {
                let mut db = self.db.lock().expect("native store");
                let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let mut note = tx.query_row("SELECT title,body,kind,scope FROM memories WHERE id=? AND status IN ('candidate','confirmed')",[id],|r| Ok(json!({"title":r.get::<_,String>(0)?,"body":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"scope":r.get::<_,Option<u64>>(3)?,"status":"confirmed","supersedes":id}))).context("Memory changed; reopen its current version")?;
                let attributes: String =
                    tx.query_row("SELECT attributes FROM memories WHERE id=?", [id], |r| {
                        r.get(0)
                    })?;
                let attributes: Value = serde_json::from_str(&attributes)?;
                for (key, value) in attributes
                    .as_object()
                    .context("Invalid observation metadata")?
                {
                    note[key] = value.clone();
                }
                let revised = matches!(&command, MemoryCommand::Revise { .. });
                if let MemoryCommand::Revise { title, body, .. } = command {
                    note["title"] = json!(title);
                    note["body"] = json!(body);
                    // A free-form human correction becomes an observation; do not
                    // present the old structured checkpoint as its new content.
                    if note["kind"] == "summary" {
                        note["kind"] = json!("observation");
                    }
                }
                let mut stmt =
                    tx.prepare("SELECT source_id,source_revision FROM evidence WHERE memory_id=?")?;
                note["sources"] = json!(
                    stmt.query_map([id], |r| Ok(
                        json!({"source_id":r.get::<_,u64>(0)?,"revision":r.get::<_,u64>(1)?})
                    ))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                );
                drop(stmt);
                let mut stmt = tx.prepare("SELECT t.name FROM topics t JOIN memory_topics mt ON mt.topic_id=t.id WHERE mt.memory_id=?")?;
                note["topics"] = json!(
                    stmt.query_map([id], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                );
                drop(stmt);
                let new_id = super::memory::insert_note(&tx, MAIN, &note, None)?;
                tx.execute("INSERT OR IGNORE INTO memory_dependencies SELECT ?1,depends_on FROM memory_dependencies WHERE memory_id=?2",params![new_id,id])?;
                if !revised {
                    tx.execute("INSERT INTO session_summaries SELECT ?1,session_key,through_source_id,fields FROM session_summaries WHERE memory_id=?2",params![new_id,id])?;
                }
                tx.commit()?;
                return Ok(Some(new_id));
            }
        }
        Ok(None)
    }
}

fn readable_source(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return body.into();
    };
    let events = value.as_array().or_else(|| value["events"].as_array());
    let Some(events) = events else {
        return body.into();
    };
    events
        .iter()
        .map(|event| {
            let role = event["role"].as_str().unwrap_or("event");
            let status = event["complete"]
                .as_bool()
                .map(|complete| {
                    if complete {
                        " · complete"
                    } else {
                        " · interrupted"
                    }
                })
                .unwrap_or_default();
            let text = event["text"].as_str().unwrap_or("");
            let tools = event["tools"]
                .as_array()
                .filter(|t| !t.is_empty())
                .map(|t| format!("\n\nTools:\n{}", json!(t)))
                .unwrap_or_default();
            format!("{role}{status}\n{text}{tools}")
        })
        .collect::<Vec<_>>()
        .join("\n\n────────\n\n")
}
