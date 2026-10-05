use super::*;

impl NativeStore {
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
        chars
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
            .collect()
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
        self.ingest_fragments(
            &format!("relay:{}:turn:{}", thread.0, last.id),
            Some(thread),
            "relay",
            &format!("Thread {} / response {}", thread.0, last.id),
            &json!({"events":messages}).to_string(),
        )?;
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
