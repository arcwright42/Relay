use super::*;

impl ResidentStore {
    /// One-time upgrade of stable IDs, without opening or rebuilding a memory engine.
    pub(super) fn migrate_message_sequences(db: &Connection) -> Result<()> {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sources')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        let mut stmt = db.prepare("SELECT thread_id,source_key FROM sources WHERE thread_id IS NOT NULL AND thread_id<9223372036854775807 AND source_key LIKE 'relay:%' AND thread_id NOT IN (SELECT thread_id FROM message_sequences)")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        let mut maxima = std::collections::BTreeMap::<u64, u64>::new();
        for (thread, key) in rows {
            if let Some(id) = key.split(':').nth(3).and_then(|id| id.parse::<u64>().ok()) {
                maxima
                    .entry(thread)
                    .and_modify(|max| *max = (*max).max(id))
                    .or_insert(id);
            }
        }
        for (thread, max) in maxima {
            let next = max
                .checked_add(1)
                .filter(|n| *n <= i64::MAX as u64)
                .context("Message ID exhausted")?;
            db.execute(
                "INSERT OR IGNORE INTO message_sequences VALUES(?,?)",
                params![thread, next],
            )?;
        }
        Ok(())
    }

    pub(crate) fn reserve_message_ids(&self, thread: ThreadId, minimum: u64) -> Result<u64> {
        let mut db = self.db.lock().expect("resident store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved: Option<u64> = tx
            .query_row(
                "SELECT next_id FROM message_sequences WHERE thread_id=?",
                [thread.0],
                |r| r.get(0),
            )
            .optional()?;
        let next = saved.unwrap_or(1);
        let id = next.max(minimum);
        let next = id
            .checked_add(2)
            .filter(|n| *n <= i64::MAX as u64)
            .context("Message ID exhausted")?;
        tx.execute("INSERT INTO message_sequences VALUES(?,?) ON CONFLICT(thread_id) DO UPDATE SET next_id=excluded.next_id",params![thread.0,next])?;
        tx.commit()?;
        Ok(id)
    }
}
