//! G3-P3 Build 探针集成测试（迁移 Go `build_probe_monitor_test.go`）。
//!
//! 覆盖：阻塞探针适配器 → 状态 Running/Current → 释放 → 403 permission-denied →
//! deletable 迁移 → 统计（attempts/failed/deletable/lane）/ 池汇总 / recent。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use grok_domain::{Account, AuthStatus, Billing, Provider, QuotaRecovery};
use grok_ops::build_probe::{BuildProbeMode, BuildProbeOutcome, ProbeFailure};
use grok_ops::{BuildFourPool, BuildProbeOps, OpsResult, TickResult};
use tokio::sync::{mpsc, Notify};

struct FakeBuildProbeOps {
    accounts: Arc<Mutex<Vec<Account>>>,
    started: mpsc::Sender<i64>,
    release: Arc<Notify>,
}

impl FakeBuildProbeOps {
    fn new(accounts: Vec<Account>) -> (Self, mpsc::Receiver<i64>, Arc<Notify>) {
        let release = Arc::new(Notify::new());
        let (started, rx) = mpsc::channel(1);
        (
            Self {
                accounts: Arc::new(Mutex::new(accounts)),
                started,
                release: release.clone(),
            },
            rx,
            release,
        )
    }

    fn account(&self, id: i64) -> Option<Account> {
        self.accounts
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }
}

#[async_trait::async_trait]
impl BuildProbeOps for FakeBuildProbeOps {
    async fn get_account(&self, id: i64) -> OpsResult<Option<Account>> {
        Ok(self.account(id))
    }

    async fn list_build_accounts(&self, _now: DateTime<Utc>) -> OpsResult<Vec<Account>> {
        Ok(self.accounts.lock().unwrap().clone())
    }

    async fn recoveries_for(&self, _ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>> {
        Ok(HashMap::new())
    }

    async fn billings_for(&self, _ids: &[i64]) -> OpsResult<HashMap<i64, Billing>> {
        Ok(HashMap::new())
    }

    async fn get_recovery(&self, _id: i64) -> OpsResult<Option<QuotaRecovery>> {
        Ok(None)
    }

    async fn get_billing(&self, _id: i64) -> OpsResult<Option<Billing>> {
        Ok(None)
    }

    async fn prepare_credential(
        &self,
        account: &Account,
        _refresh_tokens: bool,
    ) -> OpsResult<Account> {
        Ok(account.clone())
    }

    async fn refresh_billing(&self, _id: i64) -> OpsResult<()> {
        Ok(())
    }

    /// 对齐 Go `probeBuildChatCredential` 的 403(permission-denied) 分支：
    /// mark_deletable + Err("Build Chat 权限不足")。阻塞等待 release。
    async fn probe_chat_credential(&self, account: &Account) -> Result<String, String> {
        self.started.send(account.id).await.ok();
        self.release.notified().await;
        self.mark_deletable(account.id, "grok_build chat endpoint access denied")
            .await
            .ok();
        Err("Build Chat 权限不足".into())
    }

    async fn probe_chat_capability(&self, account: &Account) -> Result<String, String> {
        Ok(format!("model-of-{}", account.id))
    }

    async fn observe_model(&self, id: i64, model: &str) -> OpsResult<()> {
        if let Some(a) = self
            .accounts
            .lock()
            .unwrap()
            .iter_mut()
            .find(|a| a.id == id)
        {
            a.observed_model = Some(model.to_string());
        }
        Ok(())
    }

    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        reason: &str,
        _reset_last_success: bool,
    ) -> OpsResult<()> {
        if let Some(a) = self
            .accounts
            .lock()
            .unwrap()
            .iter_mut()
            .find(|a| a.id == id)
        {
            a.failure_count = failure_count;
            a.cooldown_until = cooldown_until;
            if !reason.is_empty() {
                a.last_error = Some(reason.to_string());
            }
        }
        Ok(())
    }

    async fn mark_deletable(&self, id: i64, reason: &str) -> OpsResult<()> {
        // 对齐 Go `markBuildDeletable`：禁用 + reauth + 去冷却 + deletable: 前缀。
        if let Some(a) = self
            .accounts
            .lock()
            .unwrap()
            .iter_mut()
            .find(|a| a.id == id)
        {
            a.enabled = false;
            a.auth_status = AuthStatus::ReauthRequired;
            a.cooldown_until = None;
            let mut text = format!("deletable: {reason}");
            text.truncate(512);
            a.last_error = Some(text);
        }
        Ok(())
    }

    async fn clear_recovery(&self, _id: i64) -> OpsResult<()> {
        Ok(())
    }

    async fn delete_account(&self, id: i64) -> OpsResult<()> {
        self.accounts.lock().unwrap().retain(|a| a.id != id);
        Ok(())
    }
}

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}

fn build_probe_account(id: i64) -> Account {
    Account {
        id,
        identity_key: format!("visual-probe-{id}"),
        provider: Provider::GrokBuild,
        enabled: true,
        auth_status: AuthStatus::Active,
        // 无 observed_model → verification 池
        observed_model: None,
        created_at: Some(base_time() - Duration::hours(1)),
        ..Default::default()
    }
}

#[tokio::test]
async fn monitor_tracks_running_account_and_delete_transition() {
    let (ops, mut started, release) = FakeBuildProbeOps::new(vec![build_probe_account(42)]);
    let pool = Arc::new(BuildFourPool::with_clock(Arc::new(ops), base_time));
    pool.configure(
        Duration::seconds(30),
        Duration::minutes(5),
        Duration::minutes(2),
    );

    let pool2 = pool.clone();
    let handle = tokio::spawn(async move { pool2.maintenance_tick().await });

    // 探针已开始（Go：adapter.started 收到 credential）
    let started_id = tokio::time::timeout(std::time::Duration::from_secs(2), started.recv())
        .await
        .expect("probe did not start")
        .expect("channel closed");
    assert_eq!(started_id, 42);

    // 运行中快照（Go：running.Enabled/Running/Current/Mode == verification）
    let running = pool.status().await.expect("status");
    assert!(running.enabled, "enabled = {}", running.enabled);
    assert!(running.running);
    let current = running.current.expect("current probe");
    assert_eq!(current.account_id, 42);
    assert_eq!(current.mode, BuildProbeMode::Verification);

    // 释放阻塞探针 → 403 permission-denied → deletable 迁移
    release.notify_one();
    let tick: TickResult = handle.await.expect("task panicked").expect("tick ok");
    assert!(tick.found);
    assert_eq!(tick.account_id, 42);
    assert!(matches!(tick.failure, Some(ProbeFailure::Other(ref t)) if t == "Build Chat 权限不足"));

    // 完成后快照（Go：Running=false、统计、池汇总、recent）
    let completed = pool.status().await.expect("status");
    assert!(!completed.running);
    let s = &completed.statistics;
    assert_eq!(s.attempts, 1);
    assert_eq!(s.failed, 1);
    assert_eq!(s.deletable, 1);
    assert_eq!(s.succeeded, 0);
    assert_eq!(s.lane_attempts.verification, 1);
    assert_eq!(completed.pools.delete, 1, "account moved to delete pool");
    assert_eq!(completed.pools.verification, 0);
    assert_eq!(completed.recent.len(), 1);
    assert_eq!(completed.recent[0].outcome, BuildProbeOutcome::Deletable);
    assert_eq!(completed.last_completed_at, Some(base_time()));
}

#[tokio::test]
async fn purge_apply_disabled_blocks_delete_lane() {
    let (ops, mut _started, _release) = FakeBuildProbeOps::new(vec![Account {
        id: 7,
        identity_key: "dead".into(),
        provider: Provider::GrokBuild,
        enabled: false,
        auth_status: AuthStatus::ReauthRequired,
        last_error: Some("deletable: old".into()),
        updated_at: Some(base_time() - Duration::hours(1)),
        ..Default::default()
    }]);
    let pool = Arc::new(BuildFourPool::with_clock(Arc::new(ops), base_time));
    pool.configure(
        Duration::seconds(30),
        Duration::minutes(5),
        Duration::minutes(2),
    );
    // purge apply 默认关闭（Go `ConfigureBuildProbePurgeApply` 未开启）
    assert!(!pool.monitor().purge_apply_enabled());

    let tick = pool.maintenance_tick().await.expect("tick");
    assert!(tick.found);
    assert_eq!(tick.account_id, 7);
    assert!(matches!(tick.failure, Some(ProbeFailure::PurgeDeletable)));

    let status = pool.status().await.expect("status");
    assert_eq!(status.statistics.deletable, 1);
    assert_eq!(status.statistics.lane_attempts.delete, 1);
    assert_eq!(status.pools.delete, 1);
}
