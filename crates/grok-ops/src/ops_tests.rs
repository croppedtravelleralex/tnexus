//! grok-ops 单元测试：mock repo / pool 下验证三个后台任务 run_once 行为。

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use grok_domain::{Account, Provider, QuotaWindow, WebLane};
use grok_pool::SimplifiedPool;

use crate::error::OpsResult;
use crate::pins::{PinSyncResult, PinSyncTask, RoutePinRepository};
use crate::probe::{ProbeBackend, WebDispatchProbe};
use crate::quota::{QuotaRefreshResult, QuotaRefresher, QuotaStore, WebQuotaRefresh};

// ─────────────────────────── probe fakes ───────────────────────────

struct FakeProbeBackend {
    calls: Mutex<HashMap<i64, usize>>,
    ok: bool,
}

#[async_trait]
impl ProbeBackend for FakeProbeBackend {
    async fn dispatch_probe(&self, account: &Account, _lane: WebLane) -> OpsResult<bool> {
        *self.calls.lock().unwrap().entry(account.id).or_insert(0) += 1;
        Ok(self.ok)
    }
}

#[tokio::test]
async fn probe_run_once_success_accounts_success() {
    let pool = Arc::new(SimplifiedPool::with_seed_and_cooldown(
        1,
        std::time::Duration::from_secs(2),
    ));
    let accounts: Vec<Account> = (1..=3)
        .map(|id| Account {
            id,
            enabled: true,
            ..Default::default()
        })
        .collect();
    pool.load_in_memory(accounts).await;

    let backend = Arc::new(FakeProbeBackend {
        calls: Mutex::new(HashMap::new()),
        ok: true,
    });
    let task = WebDispatchProbe::new(pool.clone(), backend.clone());

    let probed = task.run_once(WebLane::Image).await.expect("probe ok");

    assert_eq!(probed, 3);
    for id in 1..=3 {
        assert_eq!(pool.success_count(id).await, 1, "account {id} success +1");
        assert_eq!(*backend.calls.lock().unwrap().get(&id).unwrap(), 1);
    }
}

#[tokio::test]
async fn probe_run_once_failure_enters_cooldown() {
    let pool = Arc::new(SimplifiedPool::with_cooldown(
        std::time::Duration::from_secs(60),
    ));
    pool.load_in_memory(vec![Account {
        id: 7,
        enabled: true,
        ..Default::default()
    }])
    .await;

    let backend = Arc::new(FakeProbeBackend {
        calls: Mutex::new(HashMap::new()),
        ok: false,
    });
    let task = WebDispatchProbe::new(pool.clone(), backend);

    let probed = task.run_once(WebLane::Chat).await.expect("probe ok");

    assert_eq!(probed, 1);
    assert_eq!(pool.failure_count(7).await, 1, "failure +1");
    assert!(pool.in_cooldown(7).await, "failed account enters cooldown");
}

// ─────────────────────────── quota fakes ───────────────────────────

#[derive(Clone)]
struct FakeQuotaStore {
    windows: Arc<Mutex<HashMap<i64, Vec<QuotaWindow>>>>,
    saved: Arc<Mutex<Vec<QuotaWindow>>>,
}

#[async_trait]
impl QuotaStore for FakeQuotaStore {
    async fn get_windows(&self, account_id: i64) -> OpsResult<Vec<QuotaWindow>> {
        Ok(self
            .windows
            .lock()
            .unwrap()
            .get(&account_id)
            .cloned()
            .unwrap_or_default())
    }
    async fn save_window(&self, window: QuotaWindow) -> OpsResult<()> {
        self.saved.lock().unwrap().push(window);
        Ok(())
    }
}

struct FakeQuotaRefresher {
    next_remaining: AtomicI64,
}

impl FakeQuotaRefresher {
    fn new(next_remaining: i64) -> Self {
        Self {
            next_remaining: AtomicI64::new(next_remaining),
        }
    }
}

#[async_trait]
impl QuotaRefresher for FakeQuotaRefresher {
    async fn sync_quota(&self, account: &Account, mode: &str) -> OpsResult<QuotaWindow> {
        Ok(QuotaWindow {
            account_id: account.id,
            mode: mode.to_string(),
            remaining: self.next_remaining.load(Ordering::SeqCst),
            total: 100,
            reset_at: None,
            ..Default::default()
        })
    }
}

fn account(id: i64) -> Account {
    Account {
        id,
        enabled: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn quota_refresh_reads_window_and_writes_back() {
    let store = FakeQuotaStore {
        windows: Arc::new(Mutex::new(HashMap::from([(
            5,
            vec![QuotaWindow {
                account_id: 5,
                mode: "fast".into(),
                remaining: 30,
                total: 100,
                reset_at: None,
                ..Default::default()
            }],
        )]))),
        saved: Arc::new(Mutex::new(Vec::new())),
    };
    let refresher = Arc::new(FakeQuotaRefresher::new(42));
    let task = WebQuotaRefresh::new(refresher, Arc::new(store.clone()));

    let r = task.run_once(&account(5), None).await.expect("refresh ok");

    assert_eq!(
        r,
        QuotaRefreshResult {
            account_id: 5,
            mode: "fast".into(),
            remaining_after: 42,
            refreshed_weekly: false,
        }
    );
    let saved = store.saved.lock().unwrap();
    assert_eq!(saved.len(), 1, "one window written back");
    assert_eq!(saved[0].account_id, 5);
    assert_eq!(saved[0].remaining, 42);
}

#[tokio::test]
async fn quota_refresh_prefers_weekly_mode() {
    let store = FakeQuotaStore {
        windows: Arc::new(Mutex::new(HashMap::from([(
            9,
            vec![
                QuotaWindow {
                    account_id: 9,
                    mode: "imagine".into(),
                    remaining: 0,
                    total: 50,
                    reset_at: None,
                    ..Default::default()
                },
                QuotaWindow {
                    account_id: 9,
                    mode: "weekly".into(),
                    remaining: 10,
                    total: 100,
                    reset_at: None,
                    ..Default::default()
                },
            ],
        )]))),
        saved: Arc::new(Mutex::new(Vec::new())),
    };
    let refresher = Arc::new(FakeQuotaRefresher::new(0));
    let task = WebQuotaRefresh::new(refresher, Arc::new(store.clone()));

    let r = task.run_once(&account(9), None).await.expect("refresh ok");

    assert_eq!(r.mode, "weekly", "weekly mode preferred over imagine");
    assert!(r.refreshed_weekly);
    let saved = store.saved.lock().unwrap();
    assert_eq!(saved[0].mode, "weekly");
}

#[tokio::test]
async fn quota_refresh_hint_overrides_detection() {
    let store = FakeQuotaStore {
        windows: Arc::new(Mutex::new(HashMap::new())),
        saved: Arc::new(Mutex::new(Vec::new())),
    };
    let refresher = Arc::new(FakeQuotaRefresher::new(1));
    let task = WebQuotaRefresh::new(refresher, Arc::new(store.clone()));

    let r = task
        .run_once(&account(3), Some("imagine"))
        .await
        .expect("refresh ok");

    assert_eq!(r.mode, "imagine");
}

// ─────────────────────────── pin sync fakes ───────────────────────────

struct FakeRoutePins {
    bound: Arc<Mutex<HashMap<String, BTreeSet<i64>>>>,
}

#[async_trait]
impl RoutePinRepository for FakeRoutePins {
    async fn read_bound_ids(&self, _provider: Provider, route: &str) -> OpsResult<BTreeSet<i64>> {
        Ok(self
            .bound
            .lock()
            .unwrap()
            .get(route)
            .cloned()
            .unwrap_or_default())
    }
}

async fn pool_with(pinned: Option<i64>, members: Vec<i64>) -> Arc<SimplifiedPool> {
    let pool = Arc::new(SimplifiedPool::with_cooldown(
        std::time::Duration::from_secs(2),
    ));
    let accounts: Vec<Account> = members
        .into_iter()
        .map(|id| Account {
            id,
            enabled: true,
            ..Default::default()
        })
        .collect();
    pool.load_in_memory(accounts).await;
    if let Some(p) = pinned {
        pool.pin(p).await;
    }
    pool
}

#[tokio::test]
async fn pin_sync_applies_target_pin() {
    let pool = pool_with(None, vec![1, 2, 3]).await;
    let routes = Arc::new(FakeRoutePins {
        bound: Arc::new(Mutex::new(
            [("grok-imagine-image".to_string(), BTreeSet::from([2]))]
                .into_iter()
                .collect::<HashMap<String, BTreeSet<i64>>>(),
        )),
    });
    let task = PinSyncTask::new(pool.clone(), routes, "grok-imagine-image");

    let r = task.run_once().await.expect("pin sync ok");

    assert_eq!(
        r,
        PinSyncResult {
            changed: true,
            target_ids: BTreeSet::from([2]),
            previous_ids: BTreeSet::new(),
            added_ids: BTreeSet::from([2]),
            removed_ids: BTreeSet::new(),
        }
    );
    assert_eq!(
        pool.pinned().await,
        Some(2),
        "pool pin applied to account 2"
    );
}

#[tokio::test]
async fn pin_sync_removes_stale_pin() {
    let pool = pool_with(Some(5), vec![5, 6]).await;
    let routes = Arc::new(FakeRoutePins {
        bound: Arc::new(Mutex::new(HashMap::new())),
    });
    let task = PinSyncTask::new(pool.clone(), routes, "grok-imagine-image");

    let r = task.run_once().await.expect("pin sync ok");

    assert!(r.changed, "pin 5 removed from routes must flag change");
    assert_eq!(r.removed_ids, BTreeSet::from([5]));
    assert!(r.added_ids.is_empty());
    assert_eq!(pool.pinned().await, None, "stale pin 5 removed from pool");
}

#[tokio::test]
async fn pin_sync_no_change_when_matching() {
    let pool = pool_with(Some(4), vec![4]).await;
    let routes = Arc::new(FakeRoutePins {
        bound: Arc::new(Mutex::new(
            [("grok-imagine-image".to_string(), BTreeSet::from([4]))]
                .into_iter()
                .collect::<HashMap<String, BTreeSet<i64>>>(),
        )),
    });
    let task = PinSyncTask::new(pool.clone(), routes, "grok-imagine-image");

    let r = task.run_once().await.expect("pin sync ok");

    assert!(!r.changed, "no pin change when pool pin == route pin");
    assert_eq!(r.added_ids, BTreeSet::new());
    assert_eq!(r.removed_ids, BTreeSet::new());
    assert_eq!(pool.pinned().await, Some(4));
}

#[tokio::test]
async fn pin_sync_empty_target_empty_pool_is_noop() {
    let pool = pool_with(None, vec![]).await;
    let routes = Arc::new(FakeRoutePins {
        bound: Arc::new(Mutex::new(HashMap::new())),
    });
    let task = PinSyncTask::new(pool.clone(), routes, "grok-imagine-image");

    let r = task.run_once().await.expect("pin sync ok");

    assert!(!r.changed);
    assert!(r.target_ids.is_empty());
}

// 编译期引用错误模块与 AtomicBool，确保未误删导入（无运行语义）。
#[allow(dead_code)]
fn _compile_only_sentinels(p: Option<&AtomicBool>, _e: crate::error::OpsError) {
    if let Some(b) = p {
        let _ = b.load(Ordering::Relaxed);
    }
}

// 显式引用 PinSyncResult 字段类型以稳定 API 契约。
#[allow(dead_code)]
fn _pin_result_shape(r: PinSyncResult) -> (BTreeSet<i64>, BTreeSet<i64>) {
    (r.added_ids, r.removed_ids)
}
