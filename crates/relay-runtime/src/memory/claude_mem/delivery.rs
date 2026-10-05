use super::client::Client;
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Mutex, time::Duration};

pub(super) struct Event {
    pub key: String,
    pub session: String,
    pub path: &'static str,
    pub payload: Value,
}

/// Transport journal only. It does not extract, rank, embed or summarize memories.
pub(super) struct Outbox {
    db: Mutex<Connection>,
    directory: PathBuf,
}
impl Outbox {
    pub(super) fn open(directory: PathBuf) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let path = directory.join("delivery.sqlite3");
        let db = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS delivery (
                id INTEGER PRIMARY KEY, event_key TEXT NOT NULL UNIQUE, session TEXT NOT NULL,
                path TEXT NOT NULL, payload TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0, error TEXT, created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS delivery_pending ON delivery(state,id);
            CREATE INDEX IF NOT EXISTS delivery_session ON delivery(session,id);")?;
        Ok(Self {
            db: Mutex::new(db),
            directory,
        })
    }

    pub(super) fn enqueue(&self, events: &[Event]) -> Result<()> {
        let mut db = self.db.lock().expect("memory outbox");
        let tx = db.transaction()?;
        for event in events {
            let payload = serde_json::to_string(&event.payload)?;
            ensure!(
                payload.len() <= 2 * 1024 * 1024,
                "Memory event exceeds 2 MiB; capture was not acknowledged"
            );
            tx.execute(
                "INSERT INTO delivery(event_key,session,path,payload) VALUES (?,?,?,?) ON CONFLICT(event_key) DO UPDATE SET payload=excluded.payload WHERE delivery.state='pending'",
                params![event.key, event.session, event.path, payload],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn status(&self) -> Result<Value> {
        let db = self.db.lock().expect("memory outbox");
        let mut counts = serde_json::Map::new();
        let mut stmt = db.prepare("SELECT state,count(*) FROM delivery GROUP BY state")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
            let (state, count) = row?;
            counts.insert(state, json!(count));
        }
        let mut stmt = db.prepare("SELECT id,path,state,attempts,error FROM delivery WHERE state IN ('failed','uncertain') ORDER BY id LIMIT 20")?;
        let issues = stmt.query_map([], |r| Ok(json!({"id":r.get::<_,u64>(0)?,"path":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"attempts":r.get::<_,u64>(3)?,"error":r.get::<_,Option<String>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(
            json!({"counts":counts,"issues":issues,"resolution":"--memory-provider-resolve <id> accepted|retry|discard; retry may duplicate a Worker operation. Inspect the Worker first."}),
        )
    }

    pub(super) fn retry(&self) -> Result<()> {
        self.db.lock().expect("memory outbox").execute(
            "UPDATE delivery SET state='pending',error=NULL WHERE state='failed'",
            [],
        )?;
        Ok(())
    }

    pub(super) fn resolve(&self, id: u64, action: &str) -> Result<()> {
        let state = match action {
            "accepted" => "accepted",
            "retry" => "pending",
            "discard" => "discarded",
            _ => bail!("Resolution must be accepted, retry or discard"),
        };
        let changed = self.db.lock().expect("memory outbox").execute(
            "UPDATE delivery SET state=?1,error=NULL,payload=CASE WHEN ?1='pending' THEN payload ELSE '{}' END WHERE id=?2 AND state IN ('uncertain','failed')",
            params![state,id])?;
        ensure!(changed == 1, "No unresolved delivery with this ID");
        Ok(())
    }

    pub(super) fn drain(&self, client: &Client) -> Result<Value> {
        // A filesystem lock covers the network call too, across UI, CLI and MCP processes.
        // Opening a provider does not reset a live worker's jobs.
        let owner = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.directory.join("delivery.lock"))?;
        if let Err(error) = owner.try_lock() {
            return match error {
                std::fs::TryLockError::WouldBlock => Ok(json!({"busy":true})),
                other => Err(other.into()),
            };
        }
        self.db.lock().expect("memory outbox").execute("UPDATE delivery SET state='uncertain',error='Relay stopped during POST; inspect Worker before replay' WHERE state='sending'",[])?;
        let mut accepted = 0;
        for _ in 0..4 {
            let next: Option<(u64,String,String)> = self.db.lock().expect("memory outbox").query_row(
                "SELECT d.id,d.path,d.payload FROM delivery d WHERE d.state='pending' AND NOT EXISTS (SELECT 1 FROM delivery prior WHERE prior.session=d.session AND prior.id<d.id AND prior.state IN ('pending','sending','failed','uncertain')) ORDER BY d.id LIMIT 1",
                [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let Some((id, path, payload)) = next else {
                break;
            };
            // An unreachable Worker before POST is safely retryable. Avoid treating every
            // normal offline launch as an uncertain delivery or dropping captured events.
            if let Err(error) = client.health() {
                return Ok(json!({"accepted":accepted,"offline":true,"error":error.to_string()}));
            }
            let payload: Value =
                serde_json::from_str(&payload).context("Invalid durable memory event")?;
            self.db.lock().expect("memory outbox").execute(
                "UPDATE delivery SET state='sending',attempts=attempts+1 WHERE id=?",
                [id],
            )?;
            let (state, error) = match client.post(&path, &payload) {
                Ok((code, value)) if (200..300).contains(&code) => {
                    if value["status"] == "queued"
                        || value["status"] == "initialized"
                        || value["status"] == "accepted"
                        || (value["skipped"] == true && value["reason"] == "duplicate")
                    {
                        ("accepted", None)
                    } else if value["reason"] == "unknown_session"
                        || value["status"] == "unknown_session"
                        || value["stored"] == false
                    {
                        ("failed",Some("Worker did not store the event; session initialization may have failed".into()))
                    } else if value["status"] == "skipped" || value["skipped"] == true {
                        (
                            "skipped",
                            Some(format!(
                                "Worker skipped event ({})",
                                value["reason"]
                                    .as_str()
                                    .unwrap_or("unspecified")
                                    .chars()
                                    .take(160)
                                    .collect::<String>()
                            )),
                        )
                    } else {
                        ("uncertain",Some("Worker returned an unrecognized acknowledgement; inspect before replay".into()))
                    }
                }
                Ok((code, _)) if (400..500).contains(&code) && code != 408 => (
                    "failed",
                    Some(format!("Worker rejected delivery: HTTP {code}")),
                ),
                Ok((code, _)) => (
                    "uncertain",
                    Some(format!("Worker delivery outcome unknown: HTTP {code}")),
                ),
                Err(error) => ("uncertain", Some(error.to_string())),
            };
            self.db.lock().expect("memory outbox").execute("UPDATE delivery SET state=?1,error=?2,payload=CASE WHEN ?1 IN ('accepted','skipped') THEN '{}' ELSE payload END WHERE id=?3",params![state,error,id])?;
            if state == "accepted" {
                accepted += 1;
            }
        }
        Ok(json!({"accepted":accepted,"delivery":self.status()?}))
    }
}
