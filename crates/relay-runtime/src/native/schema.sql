PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
INSERT OR IGNORE INTO meta VALUES ('version',1),('revision',1);
INSERT OR IGNORE INTO meta VALUES ('observer_enabled',1),('observer_budget',60),('observer_window',0),('observer_runs',0);
CREATE TABLE IF NOT EXISTS threads(
 id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
 instructions TEXT NOT NULL DEFAULT '', context TEXT NOT NULL DEFAULT '[]', revision INTEGER NOT NULL DEFAULT 1,
 state TEXT NOT NULL DEFAULT 'idle', summary TEXT NOT NULL DEFAULT '', parent_id INTEGER REFERENCES threads(id)
);
INSERT OR IGNORE INTO threads(id,kind,name) VALUES(0,'coordinator','Relay');
INSERT OR IGNORE INTO threads(id,kind,name) VALUES(9223372036854775807,'observer','Memory observer');
CREATE TABLE IF NOT EXISTS runtime_sessions(
 thread_id INTEGER PRIMARY KEY REFERENCES threads(id), session_id TEXT NOT NULL, generation INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS owned_sessions(session_id TEXT PRIMARY KEY, thread_id INTEGER NOT NULL REFERENCES threads(id));
CREATE TABLE IF NOT EXISTS requests(
 id INTEGER PRIMARY KEY, request_key TEXT NOT NULL UNIQUE, thread_id INTEGER NOT NULL REFERENCES threads(id),
 prompt TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'queued', response_id INTEGER, error TEXT,
 created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS sources(
 id INTEGER PRIMARY KEY, source_key TEXT NOT NULL UNIQUE, thread_id INTEGER REFERENCES threads(id),
 origin TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL, hash TEXT NOT NULL,
 revision INTEGER NOT NULL DEFAULT 1, forgotten INTEGER NOT NULL DEFAULT 0, retired INTEGER NOT NULL DEFAULT 0,
 updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE VIRTUAL TABLE IF NOT EXISTS source_fts USING fts5(title,body,tokenize='trigram');
CREATE TABLE IF NOT EXISTS source_versions(source_id INTEGER REFERENCES sources(id),revision INTEGER,title TEXT NOT NULL,body TEXT NOT NULL,hash TEXT NOT NULL,PRIMARY KEY(source_id,revision));
CREATE TABLE IF NOT EXISTS jobs(
 id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES sources(id), source_revision INTEGER NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0, lease_until INTEGER,
 error TEXT, UNIQUE(source_id,source_revision)
);
CREATE TABLE IF NOT EXISTS memories(
 id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL, kind TEXT NOT NULL,
 scope INTEGER REFERENCES threads(id), status TEXT NOT NULL DEFAULT 'candidate',
 supersedes INTEGER REFERENCES memories(id), created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS evidence(
 memory_id INTEGER NOT NULL REFERENCES memories(id), source_id INTEGER NOT NULL REFERENCES sources(id),
 source_revision INTEGER NOT NULL, PRIMARY KEY(memory_id,source_id)
);
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(title,body,tokenize='trigram');
CREATE TABLE IF NOT EXISTS topics(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS memory_topics(memory_id INTEGER REFERENCES memories(id), topic_id INTEGER REFERENCES topics(id), PRIMARY KEY(memory_id,topic_id));
CREATE INDEX IF NOT EXISTS jobs_pending ON jobs(state,id);
CREATE INDEX IF NOT EXISTS requests_pending ON requests(state,id);
CREATE INDEX IF NOT EXISTS evidence_source ON evidence(source_id);
CREATE INDEX IF NOT EXISTS source_timeline ON sources(origin,thread_id,id);
