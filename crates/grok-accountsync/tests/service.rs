//! G4-P5 账号同步服务集成测试（迁移 Go `accountsync/service_test.go`）。
//!
//! 覆盖：跳过已同步 / 只补齐缺失 / Console 走 quota / 声明策略覆盖 /
//! Sync 去重等待 / SyncStream 流式去重 / observer 进度 / 失败计入 / 并发上限。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use grok_accountsync::{AccountSyncService, Error, Provider, QuotaKind, SyncBackend, SyncResult};
use tokio::sync::mpsc;

const TRUE: usize = 1;

// ── fakes ─────────────────────────────────────────────────────────

#[derive(Default)]
struct BillingStub {
    has_snapshot: AtomicUsize,
    checks: AtomicUsize,
    syncs: AtomicUsize,
    check_err: std::sync::atomic::AtomicBool,
    sync_err: std::sync::atomic::AtomicBool,
    /// 每轮 refresh 阻塞毫秒（并发上限测试用）。
    delay_ms: std::sync::atomic::AtomicU64,
}

impl BillingStub {
    fn counts(&self) -> (usize, usize) {
        (
            self.checks.load(Ordering::Relaxed),
            self.syncs.load(Ordering::Relaxed),
        )
    }
}

#[derive(Default)]
struct QuotaStub {
    has_snapshot: AtomicUsize,
    checks: AtomicUsize,
    syncs: AtomicUsize,
}

#[derive(Default)]
struct ModelStub {
    has_snapshot: AtomicUsize,
    checks: AtomicUsize,
    syncs: AtomicUsize,
    check_err: std::sync::atomic::AtomicBool,
    sync_err: std::sync::atomic::AtomicBool,
}

impl ModelStub {
    fn counts(&self) -> (usize, usize) {
        (
            self.checks.load(Ordering::Relaxed),
            self.syncs.load(Ordering::Relaxed),
        )
    }
}

struct AccountReader {
    provider: Provider,
    quota: Option<QuotaKind>,
}

/// 并发上限探针：记录 refresh 阶段 active 峰值。
#[derive(Default)]
struct ConcurrencyProbe {
    active: AtomicUsize,
    peak: AtomicUsize,
}

struct Backend {
    provider: AccountReader,
    billing: BillingStub,
    quota: QuotaStub,
    models: ModelStub,
    probe: ConcurrencyProbe,
}

impl Backend {
    fn build(provider: Provider) -> Self {
        Self {
            provider: AccountReader {
                provider,
                quota: None,
            },
            billing: Default::default(),
            quota: Default::default(),
            models: Default::default(),
            probe: Default::default(),
        }
    }

    fn with_quota_policy(provider: Provider, quota: QuotaKind) -> Self {
        Self {
            provider: AccountReader {
                provider,
                quota: Some(quota),
            },
            billing: Default::default(),
            quota: Default::default(),
            models: Default::default(),
            probe: Default::default(),
        }
    }
}

fn quota_kind_for(reader: &AccountReader, provider: Provider) -> QuotaKind {
    if let Some(q) = reader.quota {
        return q;
    }
    match provider {
        Provider::GrokBuild => QuotaKind::Billing,
        Provider::GrokWeb => QuotaKind::RemoteWindow,
        Provider::GrokConsole => QuotaKind::LocalWindow,
    }
}

#[async_trait::async_trait]
impl SyncBackend for Backend {
    async fn get_provider(&self, _account_id: i64) -> Result<(Provider, QuotaKind), Error> {
        Ok((
            self.provider.provider,
            quota_kind_for(&self.provider, self.provider.provider),
        ))
    }

    async fn has_billing(&self, _account_id: i64) -> Result<bool, Error> {
        self.billing.checks.fetch_add(1, Ordering::Relaxed);
        if self.billing.check_err.load(Ordering::Relaxed) {
            return Err(Error::Backend("billing check failed".into()));
        }
        Ok(self.billing.has_snapshot.load(Ordering::Relaxed) == TRUE)
    }

    async fn refresh_billing(&self, _account_id: i64) -> Result<(), Error> {
        self.billing.syncs.fetch_add(1, Ordering::Relaxed);
        if self.billing.delay_ms.load(Ordering::Relaxed) > 0 {
            self.probe.active.fetch_add(1, Ordering::Relaxed);
            let cur = self.probe.active.load(Ordering::Relaxed);
            self.probe.peak.fetch_max(cur, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(
                self.billing.delay_ms.load(Ordering::Relaxed),
            ))
            .await;
            self.probe.active.fetch_sub(1, Ordering::Relaxed);
        }
        if self.billing.sync_err.load(Ordering::Relaxed) {
            return Err(Error::Backend("billing unavailable".into()));
        }
        Ok(())
    }

    async fn has_quota(&self, _account_id: i64) -> Result<bool, Error> {
        self.quota.checks.fetch_add(1, Ordering::Relaxed);
        Ok(self.quota.has_snapshot.load(Ordering::Relaxed) == TRUE)
    }

    async fn refresh_quota(&self, _account_id: i64) -> Result<(), Error> {
        self.quota.syncs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn has_models(&self, _account_id: i64) -> Result<bool, Error> {
        self.models.checks.fetch_add(1, Ordering::Relaxed);
        if self.models.check_err.load(Ordering::Relaxed) {
            return Err(Error::Backend("model check failed".into()));
        }
        Ok(self.models.has_snapshot.load(Ordering::Relaxed) == TRUE)
    }

    async fn sync_models(&self, _account_id: i64) -> Result<(), Error> {
        self.models.syncs.fetch_add(1, Ordering::Relaxed);
        if self.models.sync_err.load(Ordering::Relaxed) {
            return Err(Error::Backend("models unavailable".into()));
        }
        Ok(())
    }
}

// ── syncAccount（单账号） ────────────────────────────────────────

#[tokio::test]
async fn sync_account_skips_existing_snapshots() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    backend.billing.has_snapshot.store(TRUE, Ordering::Relaxed);
    backend.models.has_snapshot.store(TRUE, Ordering::Relaxed);
    let service = AccountSyncService::new(backend.clone());

    service.sync_account(1).await.expect("sync ok");
    let (bc, bs) = backend.billing.counts();
    let (mc, ms) = backend.models.counts();
    assert_eq!(bc, 1, "billing checks");
    assert_eq!(bs, 0, "billing syncs");
    assert_eq!(mc, 1, "model checks");
    assert_eq!(ms, 0, "model syncs");
}

#[tokio::test]
async fn sync_account_fetches_only_missing_snapshots() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    backend.billing.has_snapshot.store(TRUE, Ordering::Relaxed);
    let service = AccountSyncService::new(backend.clone());

    service.sync_account(7).await.expect("sync ok");
    let (_, bs) = backend.billing.counts();
    let (_, ms) = backend.models.counts();
    assert_eq!(bs, 0);
    assert_eq!(ms, 1);
}

#[tokio::test]
async fn sync_account_uses_quota_for_console_provider() {
    let backend = Arc::new(Backend::build(Provider::GrokConsole));
    backend.models.has_snapshot.store(TRUE, Ordering::Relaxed);
    let service = AccountSyncService::new(backend.clone());

    service.sync_account(9).await.expect("sync ok");
    let (bc, bs) = backend.billing.counts();
    assert_eq!(bc, 0);
    assert_eq!(bs, 0);
    assert_eq!(backend.quota.checks.load(Ordering::Relaxed), 1);
    assert_eq!(backend.quota.syncs.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn sync_account_uses_declared_quota_policy_instead_of_provider_name() {
    let backend = Arc::new(Backend::with_quota_policy(
        Provider::GrokBuild,
        QuotaKind::RemoteWindow,
    ));
    backend.models.has_snapshot.store(TRUE, Ordering::Relaxed);
    let service = AccountSyncService::new(backend.clone());

    service.sync_account(10).await.expect("sync ok");
    let (bc, bs) = backend.billing.counts();
    assert_eq!(bc, 0);
    assert_eq!(bs, 0);
    assert_eq!(backend.quota.checks.load(Ordering::Relaxed), 1);
    assert_eq!(backend.quota.syncs.load(Ordering::Relaxed), 1);
}

// ── Sync / SyncStream ────────────────────────────────────────────

#[tokio::test]
async fn sync_deduplicates_accounts_and_waits_for_completion() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    let service = AccountSyncService::new(backend.clone());

    let result = service.sync(&[1, 1, 2, 0]).await;
    assert_eq!(
        result,
        SyncResult {
            succeeded: 2,
            failed: 0
        }
    );

    let (bc, bs) = backend.billing.counts();
    let (mc, ms) = backend.models.counts();
    assert_eq!(bc, 2);
    assert_eq!(bs, 2);
    assert_eq!(mc, 2);
    assert_eq!(ms, 2);
}

#[tokio::test]
async fn sync_stream_starts_before_import_completes_and_deduplicates() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    let service = AccountSyncService::with_workers(backend.clone(), 10);

    let (tx, rx) = mpsc::unbounded_channel();
    let service2 = service.clone();
    let handle = tokio::spawn(async move { service2.sync_stream(rx).await });

    tx.send(1).unwrap();
    // 等待 sync 实际开始（has_billing 被调用）。
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1000);
    loop {
        let checks = backend.billing.checks.load(Ordering::Relaxed);
        if checks > 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "stream did not start"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    drop(tx);

    let result = handle.await.expect("sync stream finished");
    assert_eq!(
        result,
        SyncResult {
            succeeded: 2,
            failed: 0
        }
    );
    let (checks, syncs) = backend.billing.counts();
    assert_eq!(checks, 2);
    assert_eq!(syncs, 2);
}

#[tokio::test]
async fn sync_stream_observed_reports_deduplicated_completion() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    let service = AccountSyncService::new(backend.clone());

    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(1).unwrap();
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    drop(tx);

    let progress = Arc::new(Mutex::new(Vec::new()));
    let totals = Arc::new(Mutex::new(Vec::new()));
    let p = Arc::clone(&progress);
    let t = Arc::clone(&totals);
    let observer = Arc::new(Mutex::new(move |completed: usize, total: usize| {
        p.lock().unwrap().push(completed);
        t.lock().unwrap().push(total);
    }));

    let result = service.sync_stream_observed(rx, observer).await;
    assert_eq!(
        result,
        SyncResult {
            succeeded: 2,
            failed: 0
        }
    );
    let progress = progress.lock().unwrap();
    let totals = totals.lock().unwrap();
    assert_eq!(progress.len(), 2);
    assert_eq!(progress[0], 1);
    assert_eq!(progress[1], 2);
    assert_eq!(totals.len(), 2);
    assert_eq!(totals[1], 2);
    assert!(totals[0] >= progress[0]);
}

#[tokio::test]
async fn sync_reports_initial_sync_failure() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    backend.billing.sync_err.store(true, Ordering::Relaxed);
    backend.models.sync_err.store(true, Ordering::Relaxed);
    let service = AccountSyncService::new(backend.clone());

    let result = service.sync(&[9]).await;
    assert_eq!(
        result,
        SyncResult {
            succeeded: 0,
            failed: 1
        }
    );
    let (_, bs) = backend.billing.counts();
    let (_, ms) = backend.models.counts();
    assert_eq!(bs, 1);
    assert_eq!(ms, 1);
}

#[tokio::test]
async fn concurrency_is_bounded_by_worker_count() {
    const WORKERS: usize = 5;
    const ACCOUNTS: usize = 40;
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    backend.models.has_snapshot.store(TRUE, Ordering::Relaxed);
    backend.billing.delay_ms.store(20, Ordering::Relaxed);
    let service = AccountSyncService::with_workers(backend.clone(), WORKERS);

    let ids: Vec<i64> = (1..=ACCOUNTS as i64).collect();
    let result = service.sync(&ids).await;
    assert_eq!(
        result,
        SyncResult {
            succeeded: ACCOUNTS,
            failed: 0
        }
    );
    let peak = backend.probe.peak.load(Ordering::Relaxed);
    assert!(peak <= WORKERS, "peak concurrency {peak} > {WORKERS}");
    assert!(peak >= 1);
}

#[tokio::test]
async fn zero_account_id_is_ignored() {
    let backend = Arc::new(Backend::build(Provider::GrokBuild));
    let service = AccountSyncService::new(backend.clone());

    let result = service.sync(&[0, 3, 0]).await;
    assert_eq!(
        result,
        SyncResult {
            succeeded: 1,
            failed: 0
        }
    );
    assert_eq!(backend.billing.checks.load(Ordering::Relaxed), 1);
}
