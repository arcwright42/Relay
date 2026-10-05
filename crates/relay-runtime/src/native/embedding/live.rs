//! Opt-in paid network evaluation with synthetic evidence only.
use super::super::{
    retrieval::SearchFilter,
    tests::{Sandbox, note},
};
use super::*;

#[test]
#[ignore = "requires explicit RELAY_EMBEDDING_TEST_CONFIG and uses the configured API"]
fn real_embedding_recall() -> Result<()> {
    let configured_root = PathBuf::from(
        std::env::var("RELAY_EMBEDDING_TEST_CONFIG")
            .context("Set the configured diagnostic data directory")?,
    );
    let cfg = EmbeddingConfig::load(&configured_root)?.context("Missing embedding config")?;
    let s = Sandbox::new();
    ensure!(
        std::env::var("RELAY_EMBEDDING_API_KEY").is_ok(),
        "Pass the API key through the evaluation process environment"
    );
    configure_embeddings(&s.root, &cfg, None)?;
    let fixtures = [
        (
            "离线存储决定",
            "我们决定使用 SQLite 保存本地记忆，因为它方便离线运行。",
        ),
        (
            "服务重试约定",
            "支付请求遇到超时，最多重试两次，间隔五秒；余额不足不重试。",
        ),
        (
            "凭据存放",
            "访问密钥保存在操作系统钥匙串，不写入配置文件、日志或仓库。",
        ),
        (
            "遗忘传播",
            "删除原始来源时，派生的观察、会话总结、全文索引和向量都必须一起失效。",
        ),
        (
            "崩溃恢复",
            "应用重启后通过持久队列恢复记忆提炼；未确认送达的用户任务不能自动重发。",
        ),
        (
            "发布门槛",
            "代码写完不代表交付完成，必须通过测试并记录仍未验证的事项。",
        ),
        ("缓存约定", "服务端缓存采用 Redis，缓存条目三十分钟到期。"),
        ("界面风格", "桌面界面采用深色背景，正文保持高对比度。"),
        ("餐饮偏好", "用户喜欢清淡的晚餐，不喜欢太甜的饮料。"),
        ("HTTP 会话", "HTTP 客户端使用连接池，关闭时释放空闲连接。"),
    ];
    let mut ids = vec![];
    for (i, (title, body)) in fixtures.iter().enumerate() {
        let source = s
            .store
            .ingest(&format!("fixture:{i}"), Some(MAIN), "relay", title, body)?;
        let mut n = note(source, body);
        n["title"] = json!(title);
        n["topics"] = json!([if i == 0 { "storage" } else { "other" }]);
        ids.push(s.store.write_memory(MAIN, &n)?);
    }
    let task = s
        .store
        .create_task("private", "Private task", "Only this task")?;
    let source = s.store.ingest(
        "private",
        Some(ThreadId(task)),
        "relay",
        "私有存储方案",
        "私有任务的账本用 PostgreSQL 保存，只对本任务可见。",
    )?;
    let mut n = note(source, "私有任务的账本用 PostgreSQL 保存，只对本任务可见。");
    n["scope"] = json!(task);
    let private = s.store.write_memory(MAIN, &n)?;
    let other = s
        .store
        .create_task("other", "Other task", "No private access")?;
    let mut indexed = 0;
    while s.store.index_embedding_batch()? > 0 {
        indexed += 1;
    }
    ensure!(
        indexed == 11,
        "Index did not finish: {}",
        s.store.embedding_status()?
    );
    let cases = [
        ("之前为什么选了SQLite？", 0),
        ("我们上次决定用什么数据库？", 0),
        (
            "Which persistence choice lets the app work without an internet connection?",
            0,
        ),
        ("支付接口一直不响应，应该重试几次、等多久？", 1),
        (
            "Where should credentials live so they do not end up in git?",
            2,
        ),
        ("想彻底忘掉一条信息，需要连带清掉什么？", 3),
        ("程序意外退出后，哪些工作可以续跑，哪些不能再次派发？", 4),
        ("写好代码就能声称做完了吗？", 5),
    ];
    let mut reports = vec![];
    let mut top1 = 0;
    let mut top3 = 0;
    for (query, expected) in cases {
        let report =
            s.store
                .search_report(ThreadId(other), query, false, &SearchFilter::default())?;
        ensure!(
            report["retrieval"]["mode"] == "hybrid",
            "Semantic retrieval degraded: {}",
            report["retrieval"]
        );
        let results = report["results"].as_array().context("results")?;
        ensure!(
            !results.iter().any(|r| r["id"] == private),
            "Private memory leaked"
        );
        let rank = results
            .iter()
            .position(|r| r["id"] == ids[expected])
            .map(|i| i + 1);
        top1 += usize::from(rank == Some(1));
        top3 += usize::from(rank.is_some_and(|r| r <= 3));
        reports.push(json!({"query":query,"expected_id":ids[expected],"rank":rank,"retrieval":report["retrieval"],"top3":results.iter().take(3).collect::<Vec<_>>()}));
    }
    let filtered = s.store.search_report(
        MAIN,
        "offline database",
        false,
        &SearchFilter {
            topic: "storage".into(),
            ..Default::default()
        },
    )?;
    let filtered = filtered["results"].as_array().context("results")?;
    ensure!(
        filtered.len() == 1 && filtered[0]["id"] == ids[0],
        "Topic filter lost the target or leaked results"
    );
    s.store.forget(MAIN, None, Some(ids[0]))?;
    let forgotten =
        s.store
            .search_report(MAIN, "offline database", false, &SearchFilter::default())?;
    ensure!(
        !forgotten["results"]
            .as_array()
            .context("results")?
            .iter()
            .any(|r| r["id"] == ids[0]),
        "Forgotten vector returned"
    );
    let report = json!({"model":cfg.model,"indexed":indexed,"queries":cases.len(),"top1":top1,"top3":top3,"scope_and_topic_filters":true,"forget_excludes_vectors":true,"cases":reports});
    if let Ok(path) = std::env::var("RELAY_RECALL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!(
        "{}",
        json!({"model":cfg.model,"indexed":indexed,"queries":cases.len(),"top1":top1,"top3":top3})
    );
    ensure!(
        top3 == cases.len(),
        "Recall@3 missed {} cases; inspect the report",
        cases.len() - top3
    );
    Ok(())
}
