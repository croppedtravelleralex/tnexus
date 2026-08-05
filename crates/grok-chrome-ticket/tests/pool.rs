//! Chrome 票池测试（迁移 Go `chrometicket/pool_test.go` + 过期/排序语义）。

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::Duration;
use grok_chrome_ticket::domain::{
    AccountCount, STATUS_CONSUMED, STATUS_EXPIRED,
};
use grok_chrome_ticket::{
    normalize_push_input, normalize_push_input_from_fields, MemoryChromeTicketRepository, Pool,
    TicketError,
};

fn mem_pool() -> Pool {
    Pool::new(Arc::new(MemoryChromeTicketRepository::new()))
}

#[tokio::test]
async fn pool_push_pop_sweep() {
    let pool = mem_pool();

    let pushed = pool
        .push(grok_chrome_ticket::PushInput {
            account_id: 1467,
            statsig_meta: "meta-abc".into(),
            device_cookie: "grok_device_id=dev1".into(),
            sign_source: "chrome".into(),
            ..Default::default()
        })
        .await
        .expect("push ok");
    assert!(!pushed.id.is_empty(), "expected ticket id");
    assert_eq!(pushed.status, "available");

    let popped = pool
        .pop_for_account(1467)
        .await
        .expect("pop ok");
    assert_eq!(popped.statsig_meta, "meta-abc");
    assert_eq!(popped.device_cookie, "grok_device_id=dev1");
    assert_eq!(popped.status, STATUS_CONSUMED);
    assert!(popped.consumed_at.is_some());

    // 已消费 → 再取报 NotFound
    let err = pool.pop_for_account(1467).await.expect_err("expected not found");
    assert_eq!(err, TicketError::NotFound);

    // 推一张毫秒过期的票 → sweep 标记 expired → stats 计数
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 1467,
        statsig_meta: "meta-old".into(),
        ttl: Duration::milliseconds(1),
        ..Default::default()
    })
    .await
    .expect("push expired");
    tokio::time::sleep(StdDuration::from_millis(5)).await;
    let n = pool.sweep().await.expect("sweep ok");
    assert!(n >= 1, "expected sweep count >= 1, got {n}");
    let stats = pool.stats().await.expect("stats ok");
    assert_eq!(stats.by_status.get(STATUS_CONSUMED), Some(&1), "one consumed");
    assert_eq!(stats.by_status.get(STATUS_EXPIRED), Some(&1), "one expired");
}

#[test]
fn normalize_push_input_accepts_python_minter_field_names() {
    let mut raw = serde_json::Map::new();
    raw.insert("account_id".into(), serde_json::json!(42));
    raw.insert("statsigMeta".into(), serde_json::json!("meta"));
    raw.insert("cookie".into(), serde_json::json!("grok_device_id=x"));
    raw.insert("sign_source".into(), serde_json::json!("chrome"));
    let input = normalize_push_input(&raw, Duration::hours(2)).expect("normalize ok");
    assert_eq!(input.account_id, 42);
    assert_eq!(input.statsig_meta, "meta");
    assert_eq!(input.device_cookie, "grok_device_id=x");
    assert_eq!(input.sign_source, "chrome");
    assert_eq!(input.ttl, Duration::hours(2));
}

#[test]
fn normalize_push_input_rejects_bad_account_or_missing_meta() {
    let mut raw = serde_json::Map::new();
    raw.insert("statsig_meta".into(), serde_json::json!("meta"));
    // 无 account_id → 无效
    assert_eq!(
        normalize_push_input(&raw, Duration::hours(1)),
        Err(TicketError::InvalidAccount)
    );
    // account_id 为 0 / 负 → 无效
    raw.insert("account_id".into(), serde_json::json!(0));
    assert_eq!(
        normalize_push_input(&raw, Duration::hours(1)),
        Err(TicketError::InvalidAccount)
    );
    // 空 meta → EmptyMeta
    let mut raw = serde_json::Map::new();
    raw.insert("account_id".into(), serde_json::json!(7));
    raw.insert("statsig_meta".into(), serde_json::json!("   "));
    assert_eq!(
        normalize_push_input(&raw, Duration::hours(1)),
        Err(TicketError::EmptyMeta)
    );
}

#[test]
fn normalize_push_input_from_fields_builds_struct() {
    let input = normalize_push_input_from_fields(
        42,
        "meta",
        "cookie",
        "ua",
        "chrome",
        Duration::hours(3),
    );
    assert_eq!(
        input,
        grok_chrome_ticket::PushInput {
            account_id: 42,
            statsig_meta: "meta".into(),
            device_cookie: "cookie".into(),
            user_agent: "ua".into(),
            sign_source: "chrome".into(),
            ttl: Duration::hours(3),
        }
    );
}

#[tokio::test]
async fn pop_returns_oldest_available_first() {
    let pool = mem_pool();
    for (i, meta) in ["first", "second", "third"].iter().enumerate() {
        pool.push(grok_chrome_ticket::PushInput {
            account_id: 9,
            statsig_meta: meta.to_string(),
            ..Default::default()
        })
        .await
        .expect("push");
        // 制造 created_at 差异（内存实现按入池顺序；用时间窗保证顺序）
        if i < 2 {
            tokio::time::sleep(StdDuration::from_millis(2)).await;
        }
    }
    for expect in ["first", "second", "third"] {
        let popped = pool.pop_for_account(9).await.expect("pop");
        assert_eq!(popped.statsig_meta, expect);
    }
}

#[tokio::test]
async fn pop_does_not_consume_other_accounts_tickets() {
    let pool = mem_pool();
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 1,
        statsig_meta: "a1".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 2,
        statsig_meta: "a2".into(),
        ..Default::default()
    })
    .await
    .unwrap();

    let popped = pool.pop_for_account(1).await.expect("pop a1");
    assert_eq!(popped.statsig_meta, "a1");
    // 账号 2 的票不受影响
    let popped2 = pool.pop_for_account(2).await.expect("pop a2");
    assert_eq!(popped2.statsig_meta, "a2");
    assert_eq!(
        pool.pop_for_account(1).await,
        Err(TicketError::NotFound)
    );
}

#[tokio::test]
async fn stats_summarizes_status_accounts_and_ttl() {
    let repo = MemoryChromeTicketRepository::new();
    let pool = Pool::with_ttl(
        Arc::new(repo),
        Duration::hours(2), // push 默认 TTL 2h → 剩余 ~7199s → "1-3h" 桶
    );
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 11,
        statsig_meta: "m1".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 11,
        statsig_meta: "m2".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 22,
        statsig_meta: "m3".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    pool.pop_for_account(11).await.expect("consume one");

    let stats = pool.stats().await.expect("stats");
    assert_eq!(stats.by_status.get("available"), Some(&2));
    assert_eq!(stats.by_status.get(STATUS_CONSUMED), Some(&1));
    // 可用票按账号分布：11 → 1 张，22 → 1 张（按 count 降序）
    assert_eq!(
        stats.available_by_account,
        vec![
            AccountCount { account_id: 11, count: 1 },
            AccountCount { account_id: 22, count: 1 },
        ]
    );
    assert_eq!(stats.available_tickets.len(), 2);
    assert!(stats.earliest_expires_at.is_some());
    assert!(stats.earliest_expires_in_sec > 0);
    // 剩余 TTL ~2h → "1-3h" 桶
    assert_eq!(stats.ttl_distribution.get("1-3h"), Some(&2));
}

#[tokio::test]
async fn sweep_expired_marks_only_available_tickets() {
    let pool = mem_pool();
    // 一张即将过期的票（显式 1ms TTL，绕过 Pool 默认 12h）
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 1,
        statsig_meta: "expiring".into(),
        ttl: Duration::milliseconds(1),
        ..Default::default()
    })
    .await
    .unwrap();
    // 一张正常票并先消费（consumed 不应被 sweep 改状态）
    pool.push(grok_chrome_ticket::PushInput {
        account_id: 2,
        statsig_meta: "consumed-now".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let popped = pool.pop_for_account(2).await.expect("consume ticket");
    assert_eq!(popped.status, STATUS_CONSUMED);

    tokio::time::sleep(StdDuration::from_millis(5)).await;
    let n = pool.sweep().await.expect("sweep");
    assert_eq!(n, 1, "only the expired available ticket is swept");
    let stats = pool.stats().await.expect("stats");
    assert_eq!(stats.by_status.get(STATUS_EXPIRED), Some(&1));
    assert_eq!(stats.by_status.get(STATUS_CONSUMED), Some(&1), "consumed untouched");
}

#[test]
fn ttl_bucket_boundaries() {
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(0), "<1h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(3599), "<1h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(3600), "1-3h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(10799), "1-3h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(10800), "3-6h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(21599), "3-6h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(21600), "6-12h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(43199), "6-12h");
    assert_eq!(grok_chrome_ticket::domain::ttl_bucket(43200), ">12h");
}
