//! G3-P3 Build 四池集成测试（迁移 Go `four_pool_probe_test.go` + 池分类）。
//!
//! 覆盖：
//! - `rebuild` 后 dispatch 序按 billing 额度剩余排序（高余量在前）
//! - `build_account_pool_at` 四池分类（verification/delete/normal/dispatch/禁用）
//! - `summarize_build_probe_pools` 汇总

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use grok_domain::{Account, AuthStatus, Billing, Provider, QuotaRecovery, QuotaRecoveryKind, QuotaRecoveryStatus};
use grok_pool::build_pool::{
    build_account_pool_at, summarize_build_probe_pools, BuildPool, BuildPoolIndex,
    BuildProbePoolSummary,
};

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn build_account(id: i64, priority: i32) -> Account {
    Account {
        id,
        identity_key: format!("acc-{id}"),
        provider: Provider::GrokBuild,
        enabled: true,
        auth_status: AuthStatus::Active,
        priority,
        observed_model: Some("grok-4.5-build-free".into()),
        updated_at: Some(now()),
        ..Default::default()
    }
}

fn billing(id: i64, used: f64) -> Billing {
    Billing {
        account_id: id,
        monthly_limit: 100.0,
        used,
        ..Default::default()
    }
}

#[test]
fn rebuild_orders_dispatch_by_billing_quota() {
    // Go `TestRebuildBuildPoolIndexOrdersDispatchByBillingQuota`：low 剩 10，high 剩 90。
    let low = build_account(1, 10);
    let high = build_account(2, 10);
    let accounts = vec![low.clone(), high.clone()];
    let recoveries = HashMap::new();
    let billings = HashMap::from([
        (1i64, billing(1, 90.0)),
        (2i64, billing(2, 10.0)),
    ]);

    let mut index = BuildPoolIndex::new();
    index.rebuild(&accounts, &recoveries, &billings, now());

    let ids = index.ordered_dispatch_ids(10);
    assert_eq!(ids, vec![2, 1], "dispatch order = {ids:?} want [high, low]");
    assert_eq!(index.len(), 2);
}

#[test]
fn pool_classification_matches_go_account_pool_at() {
    let t = now();

    // deletable:/retired: 前缀 → delete（即使 enabled）
    let a = Account {
        id: 1,
        enabled: true,
        last_error: Some("deletable: marked".into()),
        ..build_account(1, 0)
    };
    assert_eq!(build_account_pool_at(&a, t, None), Some(BuildPool::Delete));
    let a = Account {
        id: 1,
        last_error: Some("Retired: 手动退役".into()),
        ..build_account(1, 0)
    };
    assert_eq!(build_account_pool_at(&a, t, None), Some(BuildPool::Delete));

    // 手动禁用（无前缀）→ None，不进索引
    let a = Account { id: 2, enabled: false, ..build_account(2, 0) };
    assert_eq!(build_account_pool_at(&a, t, None), None);

    // reauth_required → delete
    let a = Account {
        id: 3,
        auth_status: AuthStatus::ReauthRequired,
        ..build_account(3, 0)
    };
    assert_eq!(build_account_pool_at(&a, t, None), Some(BuildPool::Delete));

    // Build + 无 observed_model → verification
    let a = Account { id: 4, observed_model: None, ..build_account(4, 0) };
    assert_eq!(build_account_pool_at(&a, t, None), Some(BuildPool::Verification));

    // recovery Exhausted/Probing → normal
    for status in [QuotaRecoveryStatus::Exhausted, QuotaRecoveryStatus::Probing] {
        let r = QuotaRecovery {
            account_id: 5,
            kind: QuotaRecoveryKind::Free,
            status,
            ..Default::default()
        };
        assert_eq!(
            build_account_pool_at(&build_account(5, 0), t, Some(&r)),
            Some(BuildPool::Normal)
        );
    }

    // cooldown 未过 → normal
    let a = Account {
        id: 6,
        cooldown_until: Some(t + Duration::minutes(5)),
        ..build_account(6, 0)
    };
    assert_eq!(build_account_pool_at(&a, t, None), Some(BuildPool::Normal));

    // 其余 → dispatch
    assert_eq!(build_account_pool_at(&build_account(7, 0), t, None), Some(BuildPool::Dispatch));
}

#[test]
fn summarize_counts_four_pools() {
    let t = now();
    let accounts = vec![
        build_account(1, 0),                                            // dispatch
        Account { id: 2, observed_model: None, ..build_account(2, 0) }, // verification
        Account {
            id: 3,
            auth_status: AuthStatus::ReauthRequired,
            ..build_account(3, 0)
        }, // delete
        Account {
            id: 4,
            cooldown_until: Some(t + Duration::minutes(5)),
            ..build_account(4, 0)
        }, // normal
        Account { id: 5, enabled: false, ..build_account(5, 0) }, // 不计
    ];
    let recoveries = HashMap::new();
    let summary = summarize_build_probe_pools(&accounts, &recoveries, t);
    assert_eq!(
        summary,
        BuildProbePoolSummary { dispatch: 1, normal: 1, verification: 1, delete: 1 }
    );
}

#[test]
fn maintenance_drr_picks_verification_first() {
    // Go `TestRebuildBuildPoolIndexOrdersDispatchByBillingQuota` 之外的分轨：验证堆优先。
    let t = now();
    let accounts = vec![
        Account {
            id: 1,
            observed_model: None,
            created_at: Some(t - Duration::hours(2)),
            ..build_account(1, 0)
        }, // verification
        Account {
            id: 2,
            cooldown_until: Some(t + Duration::minutes(30)),
            ..build_account(2, 0)
        }, // normal（冷却到期前不该被选）
        Account { id: 3, last_error: Some("deletable: old".into()), ..build_account(3, 0) },
    ];
    let mut index = BuildPoolIndex::new();
    index.rebuild(&accounts, &HashMap::new(), &HashMap::new(), t);

    let (lane, id) = index.maintenance_next(t).expect("verification lane has work");
    assert_eq!(lane, grok_pool::poolindex::Lane::Verification);
    assert_eq!(id, 1);

    // 验证堆取完后，正常堆的冷却账号未到期 → 删除车道（DRR fallback 7:3 轮到 delete）
    let (_lane, id) = index.maintenance_next(t).expect("delete lane has work");
    assert_eq!(id, 3);
    // 普通池未到期：正常到期时才会被取
    assert!(index.maintenance_next(t).is_none());
}
