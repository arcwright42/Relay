use super::*;

#[test]
fn v1_migration_preserves_evidence_and_schedules_backfill_once() {
    let root = std::env::temp_dir().join(format!("relay-migration-v2-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    {
        let db = Connection::open(root.join("relay.sqlite3")).unwrap();
        db.execute_batch(include_str!("../schema.sql")).unwrap();
        db.execute_batch("INSERT INTO sources(id,source_key,origin,title,body,hash) VALUES(5,'old','archive:session','old decision','SQLite','hash'); INSERT INTO source_versions VALUES(5,1,'old decision','SQLite','hash'); INSERT INTO jobs(id,source_id,source_revision,state,attempts) VALUES(7,5,1,'done',2); INSERT INTO memories(id,title,body,kind,status) VALUES(9,'decision','SQLite','decision','confirmed'); INSERT INTO evidence VALUES(9,5,1); INSERT INTO memory_fts(rowid,title,body) VALUES(9,'decision','SQLite');").unwrap();
    }
    for _ in 0..2 {
        let store = NativeStore::open(root.clone()).unwrap();
        assert_eq!(
            store.get_memory(MAIN, &[9]).unwrap()[0]["evidence"][0]["source_id"],
            5
        );
        let db = store.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("SELECT attempts FROM jobs WHERE id=7", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM embedding_jobs", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
    {
        let db = Connection::open(root.join("relay.sqlite3")).unwrap();
        db.execute("UPDATE meta SET value=99 WHERE key='version'", [])
            .unwrap();
    }
    assert!(NativeStore::open(root.clone()).is_err());
    let db = Connection::open(root.join("relay.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT body FROM memories WHERE id=9", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "SQLite"
    );
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
