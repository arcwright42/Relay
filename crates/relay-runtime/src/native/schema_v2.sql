-- Applied once, in the same IMMEDIATE transaction as the schema version update.
ALTER TABLE sources ADD COLUMN session_key TEXT NOT NULL DEFAULT '';
UPDATE sources SET session_key=CASE WHEN thread_id IS NULL THEN origin ELSE 'thread:'||thread_id END;
CREATE INDEX source_session ON sources(session_key,id);
-- Message identity must outlive any particular chat/context buffer.
CREATE TABLE message_sequences(
 thread_id INTEGER PRIMARY KEY REFERENCES threads(id), next_id INTEGER NOT NULL
);

ALTER TABLE jobs RENAME TO jobs_v1;
CREATE TABLE jobs(
 id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES sources(id), source_revision INTEGER NOT NULL,
 kind TEXT NOT NULL DEFAULT 'observation' CHECK(kind IN ('observation','summary')),
 state TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0, retry_attempts INTEGER NOT NULL DEFAULT 0, lease_until INTEGER,
 error TEXT, UNIQUE(source_id,source_revision,kind)
);
INSERT INTO jobs(id,source_id,source_revision,state,attempts,retry_attempts,lease_until,error)
 SELECT id,source_id,source_revision,state,attempts,attempts,lease_until,error FROM jobs_v1;
DROP TABLE jobs_v1;
CREATE INDEX jobs_pending ON jobs(state,id);
CREATE TABLE job_sources(
 job_id INTEGER NOT NULL REFERENCES jobs(id), source_id INTEGER NOT NULL REFERENCES sources(id),
 source_revision INTEGER NOT NULL, PRIMARY KEY(job_id,source_id)
);
CREATE TABLE job_memories(
 job_id INTEGER NOT NULL REFERENCES jobs(id), memory_id INTEGER NOT NULL REFERENCES memories(id),
 PRIMARY KEY(job_id,memory_id)
);
CREATE TABLE memory_dependencies(
 memory_id INTEGER NOT NULL REFERENCES memories(id), depends_on INTEGER NOT NULL REFERENCES memories(id),
 PRIMARY KEY(memory_id,depends_on)
);
CREATE INDEX memory_dependents ON memory_dependencies(depends_on);
ALTER TABLE memories ADD COLUMN attributes TEXT NOT NULL DEFAULT '{}';
ALTER TABLE memories ADD COLUMN observer_job_id INTEGER REFERENCES jobs(id);
CREATE TABLE session_summaries(
 memory_id INTEGER PRIMARY KEY REFERENCES memories(id), session_key TEXT NOT NULL,
 through_source_id INTEGER NOT NULL REFERENCES sources(id), fields TEXT NOT NULL
);
CREATE INDEX summaries_session ON session_summaries(session_key,through_source_id);

CREATE TABLE embedding_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE embedding_jobs(
 memory_id INTEGER PRIMARY KEY REFERENCES memories(id), state TEXT NOT NULL DEFAULT 'pending',
 attempts INTEGER NOT NULL DEFAULT 0, lease_until INTEGER, retry_at INTEGER NOT NULL DEFAULT 0,
 error TEXT, lease_token TEXT
);
CREATE TABLE memory_embeddings(
 memory_id INTEGER NOT NULL REFERENCES memories(id), part INTEGER NOT NULL,
 profile TEXT NOT NULL, dimensions INTEGER NOT NULL, vector BLOB NOT NULL,
 PRIMARY KEY(memory_id,part)
);
CREATE TRIGGER memory_embedding_insert AFTER INSERT ON memories
 WHEN NEW.status IN ('candidate','confirmed') BEGIN
 INSERT OR IGNORE INTO embedding_jobs(memory_id) VALUES(NEW.id);
END;
CREATE TRIGGER memory_withdraw AFTER UPDATE OF status ON memories
 WHEN NEW.status NOT IN ('candidate','confirmed') AND
 (OLD.status IN ('candidate','confirmed') OR (NEW.status='deleted' AND OLD.status!='deleted')) BEGIN
 DELETE FROM memory_fts WHERE rowid IN (
  WITH RECURSIVE affected(id) AS (SELECT NEW.id UNION SELECT d.memory_id FROM memory_dependencies d JOIN affected a ON d.depends_on=a.id) SELECT id FROM affected
 );
 DELETE FROM memory_embeddings WHERE memory_id IN (
  WITH RECURSIVE affected(id) AS (SELECT NEW.id UNION SELECT d.memory_id FROM memory_dependencies d JOIN affected a ON d.depends_on=a.id) SELECT id FROM affected
 );
 DELETE FROM embedding_jobs WHERE memory_id IN (
  WITH RECURSIVE affected(id) AS (SELECT NEW.id UNION SELECT d.memory_id FROM memory_dependencies d JOIN affected a ON d.depends_on=a.id) SELECT id FROM affected
 );
 UPDATE session_summaries SET fields='{}' WHERE NEW.status='deleted' AND memory_id IN (
  WITH RECURSIVE affected(id) AS (SELECT NEW.id UNION SELECT d.memory_id FROM memory_dependencies d JOIN affected a ON d.depends_on=a.id) SELECT id FROM affected
 );
 UPDATE memories SET title='',body='',attributes='{}' WHERE id=NEW.id AND NEW.status='deleted';
 UPDATE jobs SET state='pending',retry_attempts=0,lease_until=NULL,error='Prior memory changed; rebuilding context'
 WHERE state='done' AND NEW.status!='deleted' AND id IN (
  SELECT observer_job_id FROM memories WHERE status IN ('candidate','confirmed') AND id IN (
   WITH RECURSIVE descendants(id) AS (SELECT memory_id FROM memory_dependencies WHERE depends_on=NEW.id UNION SELECT d.memory_id FROM memory_dependencies d JOIN descendants p ON d.depends_on=p.id) SELECT id FROM descendants
  )
 );
 UPDATE memories SET status=CASE WHEN NEW.status='deleted' THEN 'deleted' ELSE 'stale' END,
  title=CASE WHEN NEW.status='deleted' THEN '' ELSE title END,
  body=CASE WHEN NEW.status='deleted' THEN '' ELSE body END,
  attributes=CASE WHEN NEW.status='deleted' THEN '{}' ELSE attributes END
 WHERE (status IN ('candidate','confirmed') OR NEW.status='deleted') AND id IN (
  WITH RECURSIVE descendants(id) AS (
   SELECT memory_id FROM memory_dependencies WHERE depends_on=NEW.id
   UNION SELECT d.memory_id FROM memory_dependencies d JOIN descendants p ON d.depends_on=p.id
  ) SELECT id FROM descendants
 );
END;
INSERT OR IGNORE INTO embedding_jobs(memory_id) SELECT id FROM memories WHERE status IN ('candidate','confirmed');
-- Existing installations receive a summary checkpoint without replaying user work.
INSERT OR IGNORE INTO jobs(source_id,source_revision,kind)
 SELECT s.id,s.revision,'summary' FROM sources s
 WHERE s.forgotten=0 AND s.retired=0 AND s.origin!='relay-user'
 AND s.id=(SELECT max(p.id) FROM sources p WHERE p.session_key=s.session_key AND p.origin!='relay-user' AND p.forgotten=0 AND p.retired=0);
UPDATE meta SET value=2 WHERE key='version';
