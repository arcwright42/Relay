//! Hybrid recall returns a compact index; evidence remains behind get/timeline/source.
use super::embedding::{self, EmbeddingConfig};
use super::memory::clip;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct SearchFilter {
    pub kind: String,
    pub topic: String,
    pub session: String,
}

// Chinese sentences do not contain whitespace. Preserve identifiers (SQLite,
// paths, error codes) and use overlapping CJK trigrams for the lexical leg.
fn terms(query: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    let mut run = String::new();
    let mut ascii = None;
    let flush = |run: &mut String, ascii: Option<bool>, terms: &mut BTreeSet<String>| {
        let text = std::mem::take(run);
        if text.chars().count() < 3 {
            return;
        }
        if ascii == Some(true) {
            if ![
                "the", "and", "did", "what", "why", "how", "was", "were", "for", "with", "that",
                "this", "our", "have",
            ]
            .contains(&text.to_lowercase().as_str())
            {
                terms.insert(text);
            }
        } else {
            let chars: Vec<_> = text.chars().collect();
            if chars.len() <= 12 {
                terms.insert(text);
            }
            for part in chars.windows(3) {
                terms.insert(part.iter().collect());
            }
        }
    };
    for c in query.chars() {
        if !c.is_alphanumeric() && !matches!(c, '_' | '-' | '.' | '/') {
            flush(&mut run, ascii, &mut terms);
            ascii = None;
            continue;
        }
        if ascii.is_some_and(|a| a != c.is_ascii()) {
            flush(&mut run, ascii, &mut terms);
        }
        ascii = Some(c.is_ascii());
        run.push(c);
    }
    flush(&mut run, ascii, &mut terms);
    terms.into_iter().take(64).collect()
}

fn memory_guard() -> &'static str {
    "m.status IN ('candidate','confirmed') AND (?1 OR m.scope IS NULL OR m.scope=?2) AND (?4='' OR m.kind=?4) AND (?5='' OR EXISTS(SELECT 1 FROM memory_topics mt JOIN topics t ON t.id=mt.topic_id WHERE mt.memory_id=m.id AND t.name=?5)) AND (?6='' OR EXISTS(SELECT 1 FROM evidence e JOIN sources s ON s.id=e.source_id WHERE e.memory_id=m.id AND s.session_key=?6))"
}
fn source_guard() -> &'static str {
    "m.forgotten=0 AND m.retired=0 AND (?1 OR m.thread_id=?2) AND (?4='' OR m.origin=?4) AND (?5='' OR EXISTS(SELECT 1 FROM evidence e JOIN memory_topics mt ON mt.memory_id=e.memory_id JOIN topics t ON t.id=mt.topic_id WHERE e.source_id=m.id AND t.name=?5)) AND (?6='' OR m.session_key=?6)"
}
fn index_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,u64>(0)?,"title":r.get::<_,String>(1)?,"preview":clip(&r.get::<_,String>(2)?,240),"status_or_origin":r.get::<_,String>(3)?,"scope":r.get::<_,i64>(4)?,"source_revision":r.get::<_,u64>(5)?,"kind":r.get::<_,String>(6)?,"created_at":r.get::<_,u64>(7)?}),
    )
}

impl NativeStore {
    #[cfg(test)]
    pub(crate) fn search(&self, caller: ThreadId, query: &str, sources: bool) -> Result<Value> {
        Ok(self.search_report(caller, query, sources, &SearchFilter::default())?["results"].take())
    }

    pub(super) fn search_report(
        &self,
        caller: ThreadId,
        query: &str,
        sources: bool,
        filter: &SearchFilter,
    ) -> Result<Value> {
        let query = clip(query.trim(), 400);
        self.search_report_with(caller, &query, sources, filter, || {
            self.semantic_search(caller, &query, filter)
        })
    }

    pub(super) fn search_report_with(
        &self,
        caller: ThreadId,
        query: &str,
        sources: bool,
        filter: &SearchFilter,
        semantic: impl FnOnce() -> Result<Option<Vec<(Value, f64)>>>,
    ) -> Result<Value> {
        let lexical = self.lexical_search(caller, query, sources, filter)?;
        let mut ranked = BTreeMap::<u64, (Value, f64)>::new();
        for (position, row) in lexical.into_iter().enumerate() {
            ranked.insert(
                row["id"].as_u64().context("memory id")?,
                (row, 1.0 / (60.0 + position as f64 + 1.0)),
            );
        }
        let mut mode = "lexical";
        let mut warning = None;
        if !sources && !query.is_empty() {
            match semantic() {
                Ok(Some(rows)) => {
                    mode = "hybrid";
                    for (position, (mut row, similarity)) in rows.into_iter().enumerate() {
                        let id = row["id"].as_u64().context("memory id")?;
                        row["semantic_similarity"] = json!(similarity);
                        let contribution = 1.0 / (60.0 + position as f64 + 1.0);
                        if let Some((existing, score)) = ranked.get_mut(&id) {
                            existing["semantic_similarity"] = json!(similarity);
                            *score += contribution;
                        } else {
                            ranked.insert(id, (row, contribution));
                        }
                    }
                }
                Ok(None) => {
                    warning = Some("Semantic index is not ready; lexical results only".to_owned())
                }
                Err(e) => warning = Some(e.to_string()),
            }
        }
        // Network I/O can race with forget, revision or a scope change. Recheck
        // every lexical and semantic candidate after that I/O, before disclosure.
        let mut results = vec![];
        {
            let db = self.db.lock().expect("native store");
            let (table, guard) = if sources {
                ("sources", source_guard())
            } else {
                ("memories", memory_guard())
            };
            let mut stmt = db.prepare(&format!(
                "SELECT EXISTS(SELECT 1 FROM {table} m WHERE {guard} AND m.id=?3)"
            ))?;
            for (id, mut row) in ranked {
                if stmt.query_row(
                    params![
                        caller == MAIN || caller == OBSERVER,
                        caller.0,
                        id,
                        filter.kind,
                        filter.topic,
                        filter.session
                    ],
                    |r| r.get::<_, bool>(0),
                )? {
                    row.0["source_id"] = if sources {
                        json!(id)
                    } else {
                        let source:Option<u64>=db.query_row("SELECT coalesce((SELECT j.source_id FROM jobs j JOIN memories m ON m.observer_job_id=j.id WHERE m.id=?1),(SELECT max(source_id) FROM evidence WHERE memory_id=?1))",[id],|r|r.get(0))?;
                        json!(source)
                    };
                    results.push(row);
                }
            }
        }
        results.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| b.0["id"].as_u64().cmp(&a.0["id"].as_u64()))
        });
        let results: Vec<_> = results
            .into_iter()
            .take(12)
            .map(|(mut row, score)| {
                row["rank_score"] = json!(score);
                row
            })
            .collect();
        Ok(json!({"results":results,"retrieval":{"mode":mode,"warning":warning}}))
    }

    fn lexical_search(
        &self,
        caller: ThreadId,
        query: &str,
        sources: bool,
        filter: &SearchFilter,
    ) -> Result<Vec<Value>> {
        let fts = terms(query)
            .into_iter()
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let db = self.db.lock().expect("native store");
        let (table, index, guard, columns) = if sources {
            (
                "sources",
                "source_fts",
                source_guard(),
                "m.origin,coalesce(m.thread_id,-1),m.revision,m.origin,m.updated_at",
            )
        } else {
            (
                "memories",
                "memory_fts",
                memory_guard(),
                "m.status,coalesce(m.scope,-1),0,m.kind,m.created_at",
            )
        };
        let (join, condition, order) = if query.is_empty() {
            (String::new(), "?3=''".to_owned(), "m.id DESC".to_owned())
        } else if !fts.is_empty() {
            (
                format!("JOIN {index} f ON f.rowid=m.id"),
                format!("{index} MATCH ?3"),
                format!("bm25({index}),m.id DESC"),
            )
        } else {
            (
                String::new(),
                "(instr(lower(m.title),lower(?3))>0 OR instr(lower(m.body),lower(?3))>0)"
                    .to_owned(),
                "m.id DESC".to_owned(),
            )
        };
        let mut stmt=db.prepare(&format!("SELECT m.id,m.title,m.body,{columns} FROM {table} m {join} WHERE {guard} AND {condition} ORDER BY {order} LIMIT 48"))?;
        Ok(stmt
            .query_map(
                params![
                    caller == MAIN || caller == OBSERVER,
                    caller.0,
                    if fts.is_empty() { query } else { &fts },
                    filter.kind,
                    filter.topic,
                    filter.session
                ],
                index_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn semantic_search(
        &self,
        caller: ThreadId,
        query: &str,
        filter: &SearchFilter,
    ) -> Result<Option<Vec<(Value, f64)>>> {
        let Some(config) = EmbeddingConfig::load(&self.root)? else {
            return Ok(None);
        };
        let profile = self.embedding_profile(&config)?;
        let has_index: bool = self.db.lock().expect("native store").query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_embeddings WHERE profile=?)",
            [&profile],
            |r| r.get(0),
        )?;
        if !has_index {
            return Ok(None);
        }
        let mut vectors = embedding::embed(
            &self.root,
            &config,
            &[query.to_owned()],
            Duration::from_secs(5),
        )?;
        let query = vectors.pop().context("Missing query embedding")?;
        let db = self.db.lock().expect("native store");
        let current: String = db.query_row(
            "SELECT value FROM embedding_state WHERE key='profile'",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            current == profile,
            "Embedding configuration changed during recall; retry the query"
        );
        let mut stmt=db.prepare(&format!("SELECT e.memory_id,e.dimensions,e.vector FROM memory_embeddings e JOIN memories m ON m.id=e.memory_id WHERE {} AND e.profile=?3",memory_guard()))?;
        let mut rows = stmt.query(params![
            caller == MAIN || caller == OBSERVER,
            caller.0,
            profile,
            filter.kind,
            filter.topic,
            filter.session
        ])?;
        let mut scores = BTreeMap::<u64, f64>::new();
        while let Some(row) = rows.next()? {
            let id: u64 = row.get(0)?;
            let dimensions: usize = row.get(1)?;
            let bytes: Vec<u8> = row.get(2)?;
            ensure!(
                dimensions == query.len() && bytes.len() == dimensions * 4,
                "Stored embedding dimensions differ from the query; rebuild the index"
            );
            let score = bytes
                .chunks_exact(4)
                .zip(&query)
                .map(|(v, q)| {
                    f64::from(f32::from_le_bytes([v[0], v[1], v[2], v[3]])) * f64::from(*q)
                })
                .sum::<f64>();
            ensure!(score.is_finite(), "Stored embedding is invalid");
            scores
                .entry(id)
                .and_modify(|s| *s = s.max(score))
                .or_insert(score);
        }
        let mut scores: Vec<_> = scores.into_iter().collect();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut out = vec![];
        for (id, score) in scores.into_iter().take(48) {
            let row=db.query_row(&format!("SELECT m.id,m.title,m.body,m.status,coalesce(m.scope,-1),0,m.kind,m.created_at FROM memories m WHERE {} AND m.id=?3",memory_guard()),params![caller==MAIN || caller==OBSERVER,caller.0,id,filter.kind,filter.topic,filter.session],index_row).optional()?;
            if let Some(row) = row {
                out.push((row, score));
            }
        }
        Ok(Some(out))
    }

    pub(super) fn recovery_context(&self, caller: ThreadId) -> Result<Value> {
        let db = self.db.lock().expect("native store");
        let mut stmt=db.prepare("SELECT m.id,m.title,m.body,m.kind,m.status FROM memories m WHERE m.status IN ('candidate','confirmed') AND m.kind!='summary' AND (?1 OR m.scope IS NULL OR m.scope=?2) ORDER BY m.id DESC LIMIT 12")?;
        let recent=stmt.query_map(params![caller==MAIN,caller.0],|r|Ok(json!({"memory_id":r.get::<_,u64>(0)?,"title":r.get::<_,String>(1)?,"preview":clip(&r.get::<_,String>(2)?,160),"kind":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt=db.prepare("SELECT m.id,c.session_key,c.fields FROM session_summaries c JOIN memories m ON m.id=c.memory_id WHERE m.status IN ('candidate','confirmed') AND (?1 OR m.scope IS NULL OR m.scope=?2) AND NOT EXISTS(SELECT 1 FROM session_summaries n JOIN memories newer ON newer.id=n.memory_id WHERE n.session_key=c.session_key AND newer.status IN ('candidate','confirmed') AND (n.through_source_id>c.through_source_id OR (n.through_source_id=c.through_source_id AND n.memory_id>c.memory_id))) ORDER BY (m.scope=?2) DESC,c.through_source_id DESC LIMIT 3")?;
        let summaries = stmt
            .query_map(params![caller == MAIN, caller.0], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let summaries=summaries.into_iter().map(|(id,session,fields)| -> Result<Value> {
            let mut fields:Value=serde_json::from_str(&fields)?;
            for v in fields.as_object_mut().context("Invalid summary fields")?.values_mut() {
                if let Some(s)=v.as_str() { *v=json!(clip(s,360)); }
            }
            Ok(json!({"memory_id":id,"session":session,"summary":fields,"details":"memory_get"}))
        }).collect::<Result<Vec<_>>>()?;
        Ok(json!({"session_checkpoints":summaries,"recent_observations":recent}))
    }
}
