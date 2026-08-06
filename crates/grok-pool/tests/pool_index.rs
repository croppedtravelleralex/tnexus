//! G3-P1 `poolindex` 原语集成测试（迁移 Go `poolindex_test.go` + `web_drr_test.go`）。
//!
//! 覆盖：`DispatchIndex` 排序（priority/quota/fairness/移除）、`dispatch_quota` 从
//! Billing/QuotaRecovery 推导、`DueHeap` 到期语义、`DRRScheduler` 5:3:2 与 7:3 权重、
//! `WebDRRScheduler` 5:3:2 权重。

use chrono::{DateTime, TimeZone, Utc};
use grok_domain::{Billing, QuotaRecovery, QuotaRecoveryStatus};
use grok_pool::poolindex::{
    dispatch_quota, DRRScheduler, DispatchEntry, DispatchIndex, DueHeap, Lane, WebDRRScheduler,
    WebMaintenanceLane,
};

fn utc(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).unwrap()
}

// === DispatchIndex（移译 TestDispatchIndexOrdersByPriorityQuotaAndFairness） ===

#[test]
fn dispatch_index_orders_by_priority_quota_and_fairness() {
    let mut idx = DispatchIndex::new();
    let now = utc(1000);
    idx.upsert(DispatchEntry {
        id: 1,
        priority: 10,
        quota_remaining: 5.0,
        quota_known: true,
        last_selected_at: now,
    });
    idx.upsert(DispatchEntry {
        id: 2,
        priority: 20,
        quota_remaining: 9.0,
        quota_known: true,
        last_selected_at: now,
    });
    idx.upsert(DispatchEntry {
        id: 3,
        priority: 20,
        quota_remaining: 9.0,
        quota_known: true,
        last_selected_at: now - chrono::Duration::hours(1),
    });

    let got = idx.ascend(10);
    assert_eq!(got.len(), 3, "expect 3 entries");
    assert_eq!(got[0].id, 3, "highest priority + earliest -> id=3");
    assert_eq!(got[1].id, 2);
    assert_eq!(got[2].id, 1);

    idx.touch_selected(3, now + chrono::Duration::hours(1));
    let got = idx.ascend(1);
    assert_eq!(
        got[0].id, 2,
        "after touch id=3 last-selected newest, id=2 first"
    );

    idx.remove(2);
    assert!(!idx.contains(2), "id=2 removed");
    assert_eq!(idx.len(), 2);
}

// === DispatchIndex 同优先级按额度（TestDispatchIndexOrdersByQuotaAtSamePriority） ===

#[test]
fn dispatch_index_orders_by_quota_at_same_priority() {
    let mut idx = DispatchIndex::new();
    let now = utc(1000);
    idx.upsert(DispatchEntry {
        id: 1,
        priority: 10,
        quota_remaining: 5.0,
        quota_known: true,
        last_selected_at: now,
    });
    idx.upsert(DispatchEntry {
        id: 2,
        priority: 10,
        quota_remaining: 20.0,
        quota_known: true,
        last_selected_at: now,
    });
    idx.upsert(DispatchEntry {
        id: 3,
        priority: 10,
        quota_remaining: 0.0,
        quota_known: false,
        last_selected_at: now,
    });

    let got = idx.ascend(10);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].id, 2, "same priority -> higher quota first");
    assert_eq!(got[1].id, 1);
    assert_eq!(got[2].id, 3, "unknown quota sorted last");
}

// === dispatch_quota（TestDispatchQuotaFromBillingAndRecovery） ===

#[test]
fn dispatch_quota_from_billing_and_recovery() {
    // monthly cap with used -> remaining.
    let b = Billing {
        monthly_limit: 100.0,
        used: 25.0,
        ..Default::default()
    };
    let (known, rem) = dispatch_quota(Some(&b), None);
    assert!(known);
    assert_eq!(rem, 75.0);

    // active recovery -> confirmed_limit - confirmed_used.
    let r = QuotaRecovery {
        status: QuotaRecoveryStatus::Active,
        confirmed_limit: 1000,
        confirmed_used: 400,
        ..Default::default()
    };
    let (known, rem) = dispatch_quota(None, Some(&r));
    assert!(known);
    assert_eq!(rem, 600.0);

    // both empty -> unknown, zero.
    let (known, rem) = dispatch_quota(None, None);
    assert!(!known);
    assert_eq!(rem, 0.0);
}

// === DueHeap（TestDueHeapPeekAndPopRespectDueAt） ===

#[test]
fn due_heap_peek_and_pop_respect_due_at() {
    let mut h = DueHeap::new();
    let now = utc(2000);
    h.upsert(1, now + chrono::Duration::minutes(1));
    h.upsert(2, now - chrono::Duration::seconds(1));

    assert_eq!(h.peek_due(now), Some(2), "earliest due -> id=2");
    assert_eq!(h.pop_due(now), Some(2));
    assert_eq!(h.pop_due(now), None, "future item must not pop at now");

    assert_eq!(h.pop_any(), Some(1), "pop_any ignores due time");
}

// === DRRScheduler（TestDRRApproximatesWeights） ===

#[test]
fn drr_approximates_weights() {
    // 5:3:2 across 1000 full-work iterations.
    let mut d = DRRScheduler::new();
    let mut counts = [0usize; 3];
    let has = [true, true, true];
    for _ in 0..1000 {
        let lane = d.next(has).expect("expected lane");
        counts[lane as usize] += 1;
    }
    assert!(
        (450..=550).contains(&counts[0]),
        "verify {} ~500",
        counts[0]
    );
    assert!(
        (250..=350).contains(&counts[1]),
        "normal {} ~300",
        counts[1]
    );
    assert!(
        (150..=250).contains(&counts[2]),
        "delete {} ~200",
        counts[2]
    );

    // 7:3 fallback when verify empty.
    let mut d2 = DRRScheduler::new();
    let mut counts2 = [0usize; 3];
    let has2 = [false, true, true];
    for _ in 0..1000 {
        let lane = d2.next(has2).expect("expected lane");
        counts2[lane as usize] += 1;
    }
    assert_eq!(counts2[0], 0, "verify lane empty -> never selected");
    assert!(
        (650..=750).contains(&counts2[1]),
        "normal {} ~700",
        counts2[1]
    );
    assert!(
        (250..=350).contains(&counts2[2]),
        "delete {} ~300",
        counts2[2]
    );
}

// === WebDRRScheduler（TestWebDRRSchedulerRatio） ===

#[test]
fn web_drr_scheduler_ratio() {
    let mut s = WebDRRScheduler::new();
    let has = [true, true, true];
    let mut counts = [0usize; 3];
    for _ in 0..100 {
        let lane = s.next(has).expect("expected lane");
        counts[lane as usize] += 1;
    }
    let verify = counts[WebMaintenanceLane::RecoveryVerify as usize];
    assert!(
        (40..=60).contains(&verify),
        "verify ratio unexpected: {counts:?}"
    );
}

// 保留 Lane from_index 的防御性引用，标注未使用的枚举不含 dead_code（已 pub re-export）。
const _: fn(usize) -> Option<Lane> = Lane::from_index;
