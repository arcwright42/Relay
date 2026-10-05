PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
INSERT OR IGNORE INTO meta VALUES ('version',3),('revision',1);
CREATE TABLE IF NOT EXISTS threads(
 id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
 instructions TEXT NOT NULL DEFAULT '', context TEXT NOT NULL DEFAULT '[]', revision INTEGER NOT NULL DEFAULT 1,
 state TEXT NOT NULL DEFAULT 'idle', summary TEXT NOT NULL DEFAULT '', parent_id INTEGER REFERENCES threads(id)
);
INSERT OR IGNORE INTO threads(id,kind,name) VALUES(0,'coordinator','Relay');
CREATE TABLE IF NOT EXISTS runtime_sessions(
 thread_id INTEGER PRIMARY KEY REFERENCES threads(id), session_id TEXT NOT NULL, generation INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS owned_sessions(session_id TEXT PRIMARY KEY, thread_id INTEGER NOT NULL REFERENCES threads(id));
CREATE TABLE IF NOT EXISTS requests(
 id INTEGER PRIMARY KEY, request_key TEXT NOT NULL UNIQUE, thread_id INTEGER NOT NULL REFERENCES threads(id),
 prompt TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'queued', response_id INTEGER, error TEXT,
 created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS message_sequences(
 thread_id INTEGER PRIMARY KEY REFERENCES threads(id), next_id INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS requests_pending ON requests(state,id);
