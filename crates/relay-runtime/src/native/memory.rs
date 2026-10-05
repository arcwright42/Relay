use super::*;
use sha2::{Digest, Sha256};

pub(super) fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn clip(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}
fn visible(caller: ThreadId, scope: Option<u64>) -> bool {
    caller == MAIN || caller == OBSERVER || scope.is_none() || scope == Some(caller.0)
}

impl NativeStore {
    /// Commit the immutable event revision and extraction job together. Replays are no-ops.
    pub(crate) fn ingest(
        &self,
        key: &str,
        thread: Option<ThreadId>,
        origin: &str,
        title: &str,
        body: &str,
    ) -> Result<u64> {
        ensure!(
            body.len() <= 256000,
            "Source exceeds the bounded event size"
        );
        let hash = digest(&format!("{title}\n{body}"));
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let old = tx
            .query_row(
                "SELECT id,hash,forgotten,retired FROM sources WHERE source_key=?",
                [key],
                |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, bool>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        if let Some((id, ref prior, forgotten, retired)) = old
            && ((prior == &hash && !retired) || forgotten)
        {
            return Ok(id);
        }
        if let Some((id, _, _, _)) = old {
            // A rewritten source invalidates every derived memory, including its search index.
            tx.execute("DELETE FROM memory_fts WHERE rowid IN (SELECT memory_id FROM evidence WHERE source_id=?)",[id])?;
            tx.execute("UPDATE memories SET status='stale' WHERE id IN (SELECT memory_id FROM evidence WHERE source_id=?) AND status IN ('candidate','confirmed')",[id])?;
            tx.execute("UPDATE jobs SET state='obsolete' WHERE source_id=? AND state IN ('pending','running')",[id])?;
            tx.execute("DELETE FROM source_fts WHERE rowid=?", [id])?;
            tx.execute("UPDATE sources SET title=?,body=?,hash=?,origin=?,retired=0,revision=revision+1,updated_at=unixepoch() WHERE id=?",params![title,body,hash,origin,id])?;
        } else {
            let session = thread
                .map(|t| format!("thread:{}", t.0))
                .unwrap_or_else(|| origin.to_owned());
            tx.execute("INSERT INTO sources(source_key,thread_id,origin,title,body,hash,session_key) VALUES(?,?,?,?,?,?,?)",params![key,thread.map(|t|t.0),origin,title,body,hash,session])?;
        }
        let (id, revision): (u64, u64) = tx.query_row(
            "SELECT id,revision FROM sources WHERE source_key=?",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        tx.execute(
            "INSERT INTO source_versions(source_id,revision,title,body,hash) VALUES(?,?,?,?,?)",
            params![id, revision, title, body, hash],
        )?;
        tx.execute(
            "INSERT INTO source_fts(rowid,title,body) VALUES(?,?,?)",
            params![id, title, body],
        )?;
        if !matches!(origin, "relay-user" | "relay-tool-pending") {
            tx.execute(
                "INSERT OR IGNORE INTO jobs(source_id,source_revision) VALUES(?,?)",
                params![id, revision],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn get_memory(&self, caller: ThreadId, ids: &[u64]) -> Result<Value> {
        self.get_memory_page(caller, ids, 0)
    }

    pub(super) fn get_memory_page(
        &self,
        caller: ThreadId,
        ids: &[u64],
        evidence_offset: u64,
    ) -> Result<Value> {
        ensure!(ids.len() <= 10, "Read at most 10 memories at once");
        ensure!(
            evidence_offset <= i64::MAX as u64,
            "Invalid evidence offset"
        );
        let db = self.db.lock().expect("native store");
        let mut out = vec![];
        let mut budget = 32000;
        for id in ids {
            let (title, body, kind, scope, status, attributes): (
                String,
                String,
                String,
                Option<u64>,
                String,
                String,
            ) = db.query_row(
                "SELECT title,body,kind,scope,status,attributes FROM memories WHERE id=?",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )?;
            ensure!(
                visible(caller, scope) && matches!(status.as_str(), "candidate" | "confirmed"),
                "Memory is unavailable in this scope"
            );
            let evidence_count: u64 = db.query_row(
                "SELECT count(*) FROM evidence WHERE memory_id=?",
                [id],
                |r| r.get(0),
            )?;
            let mut stmt=db.prepare("SELECT e.source_id,e.source_revision,s.source_key,s.title FROM evidence e JOIN sources s ON s.id=e.source_id WHERE e.memory_id=? AND s.forgotten=0 AND s.retired=0 AND s.revision=e.source_revision ORDER BY s.id DESC LIMIT 32 OFFSET ?")?;
            let evidence=stmt.query_map(params![id,evidence_offset],|r|Ok(json!({"source_id":r.get::<_,u64>(0)?,"revision":r.get::<_,u64>(1)?,"source_key":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let length = body.chars().count();
            let body = clip(&body, budget.min(8000));
            budget = budget.saturating_sub(body.chars().count());
            let summary: Option<String> = db
                .query_row(
                    "SELECT fields FROM session_summaries WHERE memory_id=?",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            out.push(json!({"id":id,"title":title,"body":body,"kind":kind,"scope":scope,"status":status,"evidence":evidence,"evidence_count":evidence_count,"next_evidence_offset":(evidence_offset+32<evidence_count).then_some(evidence_offset+32),"attributes":serde_json::from_str::<Value>(&attributes)?,"summary":summary.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,"truncated":body.chars().count()<length}));
            if budget == 0 {
                break;
            }
        }
        Ok(json!(out))
    }

    pub(crate) fn timeline(&self, caller: ThreadId, source: u64) -> Result<Value> {
        let db = self.db.lock().expect("native store");
        let (scope, session): (Option<u64>, String) = db.query_row(
            "SELECT thread_id,session_key FROM sources WHERE id=? AND forgotten=0 AND retired=0",
            [source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            caller == MAIN || caller == OBSERVER || scope == Some(caller.0),
            "Source is unavailable in this scope"
        );
        let mut stmt=db.prepare("SELECT id,title,body,source_key,revision FROM sources WHERE id IN (SELECT id FROM (SELECT id FROM sources WHERE forgotten=0 AND retired=0 AND thread_id IS ?1 AND session_key=?2 AND id<=?3 ORDER BY id DESC LIMIT 3) UNION SELECT id FROM (SELECT id FROM sources WHERE forgotten=0 AND retired=0 AND thread_id IS ?1 AND session_key=?2 AND id>?3 ORDER BY id LIMIT 2)) ORDER BY id")?;
        Ok(json!(stmt.query_map(params![scope,session,source],|r|Ok(json!({"source_id":r.get::<_,u64>(0)?,"title":r.get::<_,String>(1)?,"body":clip(&r.get::<_,String>(2)?,6000),"source_key":r.get::<_,String>(3)?,"revision":r.get::<_,u64>(4)?,"truncated":r.get::<_,String>(2)?.chars().count()>6000})))?.collect::<rusqlite::Result<Vec<_>>>()?))
    }

    pub(crate) fn write_memory(&self, caller: ThreadId, note: &Value) -> Result<u64> {
        ensure!(
            caller != OBSERVER,
            "Observer writes must acknowledge a leased job"
        );
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let id = insert_note(&tx, caller, note, None)?;
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn read_source(
        &self,
        caller: ThreadId,
        source: u64,
        offset: usize,
    ) -> Result<Value> {
        let db = self.db.lock().expect("native store");
        let (body, scope, revision, key): (String, Option<u64>, u64, String) = db.query_row(
            "SELECT body,thread_id,revision,source_key FROM sources WHERE id=? AND forgotten=0 AND retired=0",
            [source], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        ensure!(
            caller == MAIN || scope == Some(caller.0),
            "Source is unavailable in this scope"
        );
        let count = body.chars().count();
        ensure!(offset <= count, "Source offset exceeds length");
        Ok(
            json!({"source_id":source,"revision":revision,"source_key":key,"body":body.chars().skip(offset).take(12000).collect::<String>(),"next_offset":(offset+12000<count).then_some(offset+12000)}),
        )
    }

    /// A snapshot rewrite may remove tail episodes. Retain provenance, but retire its index/jobs.
    pub(super) fn retire_missing(
        &self,
        origin: &str,
        keys: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT id,source_key FROM sources WHERE origin=? AND retired=0 AND forgotten=0",
            )?;
            stmt.query_map([origin], |r| {
                Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, key) in rows {
            if keys.contains(&key) {
                continue;
            }
            tx.execute("UPDATE sources SET retired=1 WHERE id=?", [id])?;
            tx.execute("DELETE FROM source_fts WHERE rowid=?", [id])?;
            tx.execute("DELETE FROM memory_fts WHERE rowid IN (SELECT memory_id FROM evidence WHERE source_id=?)",[id])?;
            tx.execute("UPDATE memories SET status='stale' WHERE id IN (SELECT memory_id FROM evidence WHERE source_id=?) AND status IN ('candidate','confirmed')",[id])?;
            tx.execute("UPDATE jobs SET state='obsolete' WHERE source_id=? AND state IN ('pending','running')",[id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn forget(
        &self,
        caller: ThreadId,
        source: Option<u64>,
        memory: Option<u64>,
    ) -> Result<()> {
        ensure!(caller == MAIN, "Only the resident agent can forget memory");
        ensure!(
            source.is_some() ^ memory.is_some(),
            "Specify exactly one source_id or memory_id"
        );
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(id) = source {
            tx.execute(
                "UPDATE source_versions SET title='',body='' WHERE source_id=?",
                [id],
            )?;
            tx.execute("DELETE FROM memory_fts WHERE rowid IN (SELECT memory_id FROM evidence WHERE source_id=?)",[id])?;
            tx.execute("UPDATE memories SET status='deleted',body='' WHERE id IN (SELECT memory_id FROM evidence WHERE source_id=?)",[id])?;
            tx.execute("DELETE FROM source_fts WHERE rowid=?", [id])?;
            tx.execute(
                "UPDATE sources SET forgotten=1,title='',body='' WHERE id=?",
                [id],
            )?;
            tx.execute("UPDATE jobs SET state='forgotten' WHERE source_id=?", [id])?;
        }
        if let Some(id) = memory {
            tx.execute("DELETE FROM memory_fts WHERE rowid=?", [id])?;
            tx.execute(
                "UPDATE memories SET status='deleted',body='' WHERE id=?",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn insert_note(
    db: &Connection,
    caller: ThreadId,
    n: &Value,
    job: Option<(u64, u64)>,
) -> Result<u64> {
    let title = n["title"].as_str().context("title required")?;
    let body = n["body"].as_str().context("body required")?;
    ensure!(
        !title.trim().is_empty()
            && title.chars().count() <= 160
            && !body.trim().is_empty()
            && body.chars().count() <= 8000,
        "Invalid memory size"
    );
    let kind = n["kind"].as_str().unwrap_or("observation");
    ensure!(
        [
            "fact",
            "decision",
            "preference",
            "working",
            "observation",
            "summary"
        ]
        .contains(&kind),
        "Invalid memory kind"
    );
    ensure!(
        n["scope"].is_null() || n["scope"].as_u64().is_some(),
        "Invalid memory scope"
    );
    let scope = n["scope"].as_u64();
    ensure!(scope != Some(OBSERVER.0), "Invalid memory scope");
    ensure!(
        caller == MAIN || caller == OBSERVER || scope == Some(caller.0),
        "Cannot write outside this task"
    );
    let status = if caller == MAIN && n["status"].as_str() == Some("confirmed") {
        "confirmed"
    } else {
        "candidate"
    };
    let sources = n["sources"]
        .as_array()
        .context("sources with source_id and revision required")?;
    ensure!(
        !sources.is_empty() && sources.len() <= 4096,
        "At least one source is required"
    );
    let mut evidence = vec![];
    for reference in sources {
        let id = reference["source_id"]
            .as_u64()
            .context("Invalid source ID")?;
        let expected_revision = reference["revision"]
            .as_u64()
            .context("Source revision required")?;
        let (revision, thread): (u64, Option<u64>) = db.query_row(
            "SELECT revision,thread_id FROM sources WHERE id=? AND forgotten=0 AND retired=0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            revision == expected_revision,
            "Source changed; retrieve its current revision before writing memory"
        );
        ensure!(
            caller == MAIN || caller == OBSERVER || thread == Some(caller.0),
            "Source belongs to another task"
        );
        if let Some((expected, v)) = job {
            ensure!(id == expected && revision == v, "Source changed")
        }
        evidence.push((id, revision));
    }
    let prior = n["supersedes"].as_u64();
    let mut attributes = json!({});
    for field in ["facts", "concepts", "files_read", "files_modified"] {
        if let Some(values) = n.get(field) {
            let values = values
                .as_array()
                .context("Observation attributes must be string arrays")?;
            ensure!(
                values.len() <= 32
                    && values
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| s.chars().count() <= 500)),
                "Invalid observation attributes"
            );
            attributes[field] = json!(values);
        }
    }
    ensure!(
        attributes.to_string().chars().count() <= 8000,
        "Observation attributes exceed the context budget"
    );
    if let Some(id) = prior {
        let prior_scope: Option<u64> = db.query_row(
            "SELECT scope FROM memories WHERE id=? AND status IN ('candidate','confirmed')",
            [id],
            |r| r.get(0),
        )?;
        ensure!(
            caller == MAIN && scope == prior_scope,
            "Only the resident agent can supersede a memory in the same scope"
        );
        db.execute("UPDATE memories SET status='superseded' WHERE id=?", [id])?;
        db.execute("DELETE FROM memory_fts WHERE rowid=?", [id])?;
    }
    db.execute(
        "INSERT INTO memories(title,body,kind,scope,status,supersedes,attributes) VALUES(?,?,?,?,?,?,?)",
        params![title, body, kind, scope, status, prior, attributes.to_string()],
    )?;
    let id = db.last_insert_rowid() as u64;
    db.execute(
        "INSERT INTO memory_fts(rowid,title,body) VALUES(?,?,?)",
        params![id, title, format!("{body}\n{attributes}")],
    )?;
    for (source, rev) in evidence {
        db.execute(
            "INSERT OR IGNORE INTO evidence(memory_id,source_id,source_revision) VALUES(?,?,?)",
            params![id, source, rev],
        )?;
    }
    for topic in n["topics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(8)
    {
        let topic = clip(topic.trim(), 60).to_lowercase();
        if topic.is_empty() {
            continue;
        }
        db.execute("INSERT OR IGNORE INTO topics(name) VALUES(?)", [&topic])?;
        db.execute(
            "INSERT OR IGNORE INTO memory_topics SELECT ?,id FROM topics WHERE name=?",
            params![id, topic],
        )?;
    }
    Ok(id)
}
