use super::*;

impl NativeStore {
    pub(super) fn migrate_legacy(&self) -> Result<()> {
        let done: bool = self.db.lock().expect("native store").query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='legacy_imported')",
            [],
            |r| r.get(0),
        )?;
        if done {
            return Ok(());
        }
        let path = self.root.join("projects.json");
        let catalog: Value = if path.try_exists()? {
            serde_json::from_slice(&std::fs::read(&path)?)?
        } else {
            json!({"version":1,"projects":[]})
        };
        ensure!(
            matches!(catalog["version"].as_u64(), Some(1..=3)),
            "Unknown legacy catalog version; files preserved"
        );
        let mut projects = catalog["projects"]
            .as_array()
            .context("Invalid legacy project catalog")?
            .clone();
        let directory = self.root.join("projects");
        if directory.try_exists()? {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let Some(id) = entry
                    .file_name()
                    .to_str()
                    .and_then(|s| s.parse::<u64>().ok())
                else {
                    continue;
                };
                if entry.path().join("conversation.json").is_file()
                    && !projects.iter().any(|p| p["id"].as_u64() == Some(id))
                {
                    projects.push(json!({"id":id,"name":format!("Imported conversation {id}")}));
                }
            }
        }
        // Copy transcripts before publishing the import marker. Repeating after a crash is safe.
        for p in &projects {
            let id = p["id"].as_u64().context("Invalid legacy identifier")?;
            ensure!(
                id > 0 && id < OBSERVER.0,
                "Legacy ID conflicts with reserved thread"
            );
            let old = self.root.join(format!("projects/{id}/conversation.json"));
            let new = self.root.join(format!("threads/{id}/conversation.json"));
            if old.try_exists()? && !new.try_exists()? {
                let mut saved: crate::store::SavedThread =
                    serde_json::from_slice(&std::fs::read(old)?)?;
                ensure!(
                    matches!(saved.version, 1..=3),
                    "Unsupported legacy conversation; files preserved"
                );
                // Start a fresh execution context: old project prompts are historical evidence.
                saved.session_id = None;
                saved.session_key = None;
                saved.context_checkpoint = Default::default();
                saved.restore_history = true;
                crate::store::write_json(&new, &saved)?;
            }
        }
        {
            let mut db = self.db.lock().expect("native store");
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            for p in &projects {
                let id = p["id"].as_u64().context("Invalid ID")?;
                tx.execute("INSERT OR IGNORE INTO threads(id,kind,name,description,instructions,context,state) VALUES(?,'archive',?,?,?,?, 'archived')",params![id,p["name"].as_str().unwrap_or("Imported conversation"),p["description"].as_str().unwrap_or(""),p["instructions"].as_str().unwrap_or(""),p["context"].as_array().map(|c|json!(c)).unwrap_or(json!([])).to_string()])?;
            }
            Self::bump(&tx)?;
            tx.commit()?;
        }
        for p in &projects {
            let id = p["id"].as_u64().context("Invalid ID")?;
            let title = p["name"].as_str().unwrap_or("Imported conversation");
            for (i, m) in p["memory"].as_array().into_iter().flatten().enumerate() {
                self.ingest_fragments(
                    &format!("legacy:{id}:memory:{i}"),
                    Some(ThreadId(id)),
                    "legacy-memory",
                    title,
                    &m.to_string(),
                )?;
            }
            self.replay_transcript(ThreadId(id))?;
        }
        self.db
            .lock()
            .expect("native store")
            .execute("INSERT OR IGNORE INTO meta VALUES('legacy_imported',1)", [])?;
        Ok(())
    }

    /// Called on the archive worker after a durable snapshot, never on the UI thread.
    pub(crate) fn ingest_archive(
        &self,
        client: &str,
        native_id: &str,
        title: &str,
        messages: &[(u64, String, String)],
    ) -> Result<()> {
        // Fixed message groups form episodes; appending only revises the last partial group.
        // A complete session is never injected wholesale or treated as a single confirmed fact.
        let origin = format!("client:{client}:{native_id}");
        let mut keys = std::collections::BTreeSet::new();
        for group in messages.chunks(16) {
            let first = group.first().context("episode")?.0;
            let text = json!(
                group
                    .iter()
                    .map(|(id, role, text)| json!({"message_id":id,"role":role,"text":text}))
                    .collect::<Vec<_>>()
            )
            .to_string();
            let fragments = self.ingest_fragments(
                &format!("{origin}:episode:{first}"),
                None,
                &origin,
                title,
                &text,
            )?;
            for (key, source) in fragments {
                keys.insert(key);
                self.queue_summary(source)?;
            }
        }
        self.retire_missing(&origin, &keys)
    }
}
