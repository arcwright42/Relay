use super::*;

impl NativeStore {
    pub(crate) fn reserve_message_ids(&self, thread: ThreadId, minimum: u64) -> Result<u64> {
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved: Option<u64> = tx
            .query_row(
                "SELECT next_id FROM message_sequences WHERE thread_id=?",
                [thread.0],
                |r| r.get(0),
            )
            .optional()?;
        let next = if let Some(next) = saved {
            next
        } else {
            // Seed upgraded databases from stable source identities, including
            // tombstones, so losing the chat file cannot overwrite old evidence.
            let mut stmt = tx.prepare(
                "SELECT source_key FROM sources WHERE thread_id=? AND source_key LIKE 'relay:%'",
            )?;
            let keys = stmt
                .query_map([thread.0], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            keys.iter()
                .filter_map(|key| key.split(':').nth(3)?.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .context("Message ID exhausted")?
        };
        let id = next.max(minimum);
        let next = id
            .checked_add(2)
            .filter(|n| *n <= i64::MAX as u64)
            .context("Message ID exhausted")?;
        tx.execute("INSERT INTO message_sequences VALUES(?,?) ON CONFLICT(thread_id) DO UPDATE SET next_id=excluded.next_id",params![thread.0,next])?;
        tx.commit()?;
        Ok(id)
    }

    /// Each ACP tool update is a durable event, independent of the assistant's
    /// final answer. Only terminal results are offered to the observer.
    pub(crate) fn capture_tool(
        &self,
        thread: ThreadId,
        response: u64,
        request: &str,
        tool: &crate::store::SavedTool,
    ) -> Result<()> {
        let origin = if matches!(tool.status.as_str(), "completed" | "failed" | "cancelled") {
            "relay-tool"
        } else {
            "relay-tool-pending"
        };
        let fragments = self.ingest_fragments(
            &format!("relay:{}:tool:{response}:{}", thread.0, tool.id),
            Some(thread),
            origin,
            &tool.title,
            &json!({"response_id":response,"user_request":request,"tool":tool}).to_string(),
        )?;
        if origin == "relay-tool" {
            for (_, source) in fragments {
                self.checkpoint_if_needed(source)?;
            }
        }
        Ok(())
    }
    /// Bounded source fragments preserve the full payload; extraction never silently truncates it.
    pub(super) fn ingest_fragments(
        &self,
        key: &str,
        thread: Option<ThreadId>,
        origin: &str,
        title: &str,
        body: &str,
    ) -> Result<Vec<(String, u64)>> {
        let chars: Vec<_> = body.chars().collect();
        let count = chars.len().div_ceil(24000);
        let fragments = chars
            .chunks(24000)
            .enumerate()
            .map(|(i, part)| {
                let key = format!("{key}:part:{i}");
                let title = if count > 1 {
                    format!("{title} ({}/{count})", i + 1)
                } else {
                    title.to_owned()
                };
                let id = self.ingest(
                    &key,
                    thread,
                    origin,
                    &title,
                    &part.iter().collect::<String>(),
                )?;
                Ok((key, id))
            })
            .collect::<Result<Vec<_>>>()?;
        // A shorter replacement must retire old tail fragments as well.
        let prefix = format!("{key}:part:");
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut stmt = tx.prepare("SELECT id,source_key FROM sources WHERE substr(source_key,1,length(?))=? AND retired=0 AND forgotten=0")?;
        let old = stmt
            .query_map(params![prefix, prefix], |r| {
                Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (id, key) in old {
            if fragments.iter().any(|(current, _)| current == &key) {
                continue;
            }
            tx.execute("UPDATE sources SET retired=1 WHERE id=?", [id])?;
            tx.execute("DELETE FROM source_fts WHERE rowid=?", [id])?;
            tx.execute("UPDATE jobs SET state='obsolete' WHERE source_id=?", [id])?;
            tx.execute("UPDATE memories SET status='stale' WHERE status IN ('candidate','confirmed') AND id IN (SELECT memory_id FROM evidence WHERE source_id=?)",[id])?;
        }
        tx.commit()?;
        Ok(fragments)
    }

    pub(crate) fn capture_user(
        &self,
        thread: ThreadId,
        message: u64,
        text: &str,
    ) -> Result<Vec<Value>> {
        self.ingest_fragments(
            &format!("relay:{}:message:{message}", thread.0),
            Some(thread),
            "relay-user",
            "User message",
            &json!({"role":"user","text":text}).to_string(),
        )?
        .into_iter()
        .map(|(_, id)| {
            let revision: u64 = self.db.lock().expect("native store").query_row(
                "SELECT revision FROM sources WHERE id=?",
                [id],
                |r| r.get(0),
            )?;
            Ok(json!({"source_id":id,"revision":revision}))
        })
        .collect::<Result<Vec<_>>>()
    }

    pub(crate) fn capture_turn(
        &self,
        thread: ThreadId,
        messages: &[crate::store::SavedMessage],
    ) -> Result<()> {
        let Some(last) = messages.last().filter(|m| m.role == "assistant") else {
            return Ok(());
        };
        for tool in &last.tools {
            self.capture_tool(
                thread,
                last.id,
                messages
                    .first()
                    .filter(|m| m.role == "user")
                    .map_or("", |m| m.text.as_str()),
                tool,
            )?;
        }
        let mut events = messages.to_vec();
        // Tool results are already durable independent events. Keep their identity
        // and outcome in the turn checkpoint without re-observing the full output.
        for message in &mut events {
            for tool in &mut message.tools {
                tool.input.clear();
                tool.output.clear();
            }
        }
        let fragments = self.ingest_fragments(
            &format!("relay:{}:turn:{}", thread.0, last.id),
            Some(thread),
            "relay",
            &format!("Thread {} / response {}", thread.0, last.id),
            &json!({"events":events}).to_string(),
        )?;
        for (_, source) in fragments {
            self.queue_summary(source)?;
        }
        Ok(())
    }

    /// Reconcile durable transcripts after a crash between transcript replacement and event capture.
    pub(super) fn replay_transcript(&self, thread: ThreadId) -> Result<()> {
        let saved = crate::store::load(&self.root, thread)?;
        for (index, message) in saved.messages.iter().enumerate() {
            if message.role == "user" {
                self.capture_user(thread, message.id, &message.text)?;
            } else {
                self.capture_turn(thread, &saved.messages[index.saturating_sub(1)..=index])?;
            }
        }
        Ok(())
    }
}
