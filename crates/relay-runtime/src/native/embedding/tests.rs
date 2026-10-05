use super::super::tests::{Sandbox, note};
use super::*;
use std::io::{BufRead, BufReader};

fn config() -> EmbeddingConfig {
    EmbeddingConfig {
        base_url: "https://example.invalid/v1".into(),
        model: "test-embedding".into(),
        dimensions: Some(3),
        chunk_chars: 256,
        batch_size: 2,
    }
}

#[test]
fn transport_obeys_openai_ordering_and_never_exposes_error_bodies() {
    for status in [200, 401] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut cfg = config();
        cfg.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "POST /v1/embeddings HTTP/1.1");
            let mut length = 0;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            std::io::Read::read_exact(&mut reader, &mut body).unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["model"], "test-embedding");
            assert_eq!(payload["input"], json!(["中文", "English"]));
            assert_eq!(payload["dimensions"], 3);
            let body = if status == 200 {
                json!({"data":[{"index":1,"embedding":[0,4,0]},{"index":0,"embedding":[3,0,0]}]})
                    .to_string()
            } else {
                "provider-secret-must-not-leak".into()
            };
            write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        let result = request_embeddings(
            &cfg,
            "unit-test-key",
            &["中文".into(), "English".into()],
            Duration::from_secs(3),
        );
        server.join().unwrap();
        if status == 200 {
            assert_eq!(result.unwrap(), vec![vec![1., 0., 0.], vec![0., 1., 0.]]);
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                "Embedding service returned HTTP 401"
            );
        }
    }
}

#[test]
fn malformed_vectors_and_unsafe_endpoints_are_rejected() {
    for data in [
        json!([{"index":0,"embedding":[1,0]},{"index":0,"embedding":[1,0]}]),
        json!([{"index":0,"embedding":[1,0]},{"index":1,"embedding":[1]}]),
        json!([{"index":0,"embedding":[0,0]},{"index":1,"embedding":[1,0]}]),
        json!([{"index":0,"embedding":[1,0]}]),
    ] {
        assert!(decode_embeddings(&json!({"data":data}), 2, None).is_err());
    }
    for base in [
        "http://example.com/v1",
        "https://key@example.com/v1",
        "https://example.com/v1?key=secret",
    ] {
        let mut cfg = config();
        cfg.base_url = base.into();
        assert!(cfg.validate().is_err());
    }
    let parts = chunks("title", &format!("{}tail-marker", "中文".repeat(2000)), 256);
    assert!(parts.len() > 10);
    assert!(parts.last().unwrap().ends_with("tail-marker"));
    assert!(parts.iter().all(|s| s.chars().count() <= 256));
}

#[test]
fn indexing_is_atomic_and_fenced_against_forget_and_profile_changes() {
    let s = Sandbox::new();
    let mut cfg = config();
    configure_embeddings(&s.root, &cfg, None).unwrap();
    let source = s
        .store
        .ingest(
            "embedding",
            Some(MAIN),
            "relay",
            "durable source",
            "evidence",
        )
        .unwrap();
    let id = s
        .store
        .write_memory(MAIN, &note(source, &"long memory ".repeat(120)))
        .unwrap();
    let mut batches = 0;
    let error = s.store.index_embedding_batch_with(|_, inputs| {
        batches += 1;
        if batches == 2 {
            bail!("transient failure");
        }
        Ok(vec![vec![1., 0., 0.]; inputs.len()])
    });
    assert!(error.is_err());
    assert_eq!(
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM memory_embeddings", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    s.store.retry_embeddings().unwrap();
    assert_eq!(
        s.store
            .index_embedding_batch_with(|_, inputs| Ok(vec![vec![1., 0., 0.]; inputs.len()]))
            .unwrap(),
        1
    );
    assert!(
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM memory_embeddings", [], |r| r
                .get::<_, u64>(0))
            .unwrap()
            > 1
    );
    cfg.model = "replacement-model".into();
    configure_embeddings(&s.root, &cfg, None).unwrap();
    s.store.embedding_profile(&cfg).unwrap();
    assert_eq!(
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM memory_embeddings", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let result = s
        .store
        .index_embedding_batch_with(|_, inputs| {
            s.store.forget(MAIN, None, Some(id)).unwrap();
            Ok(vec![vec![1., 0., 0.]; inputs.len()])
        })
        .unwrap();
    assert_eq!(
        result, 0,
        "An in-flight response must not resurrect forgotten data"
    );
    assert_eq!(
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM memory_embeddings", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn profile_aba_cannot_accept_an_old_embedding_lease() {
    let s = Sandbox::new();
    let cfg = config();
    configure_embeddings(&s.root, &cfg, None).unwrap();
    let source = s
        .store
        .ingest("race", Some(MAIN), "relay", "source", "evidence")
        .unwrap();
    s.store
        .write_memory(MAIN, &note(source, "short observation"))
        .unwrap();
    let result=s.store.index_embedding_batch_with(|_,inputs| {
        let mut other=cfg.clone(); other.model="other".into();
        s.store.embedding_profile(&other).unwrap();
        s.store.embedding_profile(&cfg).unwrap();
        // Reclaim the same memory under a new token before the old request completes.
        s.store.db.lock().unwrap().execute("UPDATE embedding_jobs SET state='running',attempts=1,lease_until=unixepoch()+180,lease_token='new-lease'",[]).unwrap();
        Ok(vec![vec![1.,0.,0.];inputs.len()])
    }).unwrap();
    assert_eq!(result, 0);
    assert_eq!(
        s.store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM memory_embeddings", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
