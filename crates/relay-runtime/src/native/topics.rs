use super::*;

impl NativeStore {
    pub(super) fn topics(&self, caller: ThreadId, topic: Option<&str>) -> Result<Value> {
        let db = self.db.lock().expect("native store");
        if let Some(topic) = topic {
            let topic = topic.trim().to_lowercase();
            let mut stmt=db.prepare("SELECT DISTINCT m.id,m.title,m.status FROM memories m JOIN memory_topics mt ON mt.memory_id=m.id JOIN topics t ON t.id=mt.topic_id WHERE t.name=?1 AND m.status IN ('candidate','confirmed') AND (?2 OR m.scope IS NULL OR m.scope=?3) ORDER BY m.id DESC LIMIT 30")?;
            let memories=stmt.query_map(params![topic,caller==MAIN,caller.0],|r|Ok(json!({"id":r.get::<_,u64>(0)?,"title":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut stmt=db.prepare("SELECT s.origin,s.thread_id,min(s.title),min(s.id),count(DISTINCT s.id) FROM sources s JOIN evidence e ON e.source_id=s.id JOIN memories m ON m.id=e.memory_id JOIN memory_topics mt ON mt.memory_id=m.id JOIN topics t ON t.id=mt.topic_id WHERE t.name=?1 AND m.status IN ('candidate','confirmed') AND s.forgotten=0 AND s.retired=0 AND s.revision=e.source_revision AND (?2 OR m.scope IS NULL OR m.scope=?3) GROUP BY s.origin,s.thread_id ORDER BY max(s.id) DESC LIMIT 80")?;
            let sessions=stmt.query_map(params![topic,caller==MAIN,caller.0],|r|Ok(json!({"origin":r.get::<_,String>(0)?,"thread_id":r.get::<_,Option<u64>>(1)?,"title":r.get::<_,String>(2)?,"source_id":r.get::<_,u64>(3)?,"source_count":r.get::<_,u64>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(json!({"topic":topic,"memories":memories,"sessions":sessions}));
        }
        let mut stmt=db.prepare("SELECT t.name,count(DISTINCT m.id),count(DISTINCT coalesce(cast(s.thread_id AS TEXT),s.origin)) FROM topics t JOIN memory_topics mt ON mt.topic_id=t.id JOIN memories m ON m.id=mt.memory_id JOIN evidence e ON e.memory_id=m.id JOIN sources s ON s.id=e.source_id WHERE m.status IN ('candidate','confirmed') AND s.forgotten=0 AND s.retired=0 AND s.revision=e.source_revision AND (? OR m.scope IS NULL OR m.scope=?) GROUP BY t.id ORDER BY count(DISTINCT m.id) DESC LIMIT 40")?;
        Ok(json!(stmt.query_map(params![caller==MAIN,caller.0],|r|Ok(json!({"topic":r.get::<_,String>(0)?,"memory_count":r.get::<_,u64>(1)?,"session_count":r.get::<_,u64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?))
    }

    pub(super) fn merge_topics(&self, from: &str, into: &str) -> Result<()> {
        let from = from.trim().to_lowercase();
        let into = into.trim().to_lowercase();
        ensure!(
            !into.is_empty() && into.chars().count() <= 60 && !from.is_empty(),
            "Invalid topic label"
        );
        if from == into {
            return Ok(());
        }
        let mut db = self.db.lock().expect("native store");
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let source: u64 =
            tx.query_row("SELECT id FROM topics WHERE name=?", [from], |r| r.get(0))?;
        tx.execute("INSERT OR IGNORE INTO topics(name) VALUES(?)", [&into])?;
        let target: u64 =
            tx.query_row("SELECT id FROM topics WHERE name=?", [into], |r| r.get(0))?;
        tx.execute("INSERT OR IGNORE INTO memory_topics(memory_id,topic_id) SELECT memory_id,? FROM memory_topics WHERE topic_id=?",params![target,source])?;
        tx.execute("DELETE FROM memory_topics WHERE topic_id=?", [source])?;
        tx.execute("DELETE FROM topics WHERE id=?", [source])?;
        tx.commit()?;
        Ok(())
    }
}
