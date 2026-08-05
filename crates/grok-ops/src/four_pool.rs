//! Build 四池探针编排（对齐 Go `account/four_pool_probe.go` 的 tick 组）。
//!
//! - [`BuildFourPool::maintenance_tick`] ← Go `MaintenanceProbeTick`（DRR 选验证/普通/删除）
//! - [`BuildFourPool::dispatch_tick`] ← Go `DispatchProbeTick`（专用 dispatch 探针堆）
//! - [`BuildProbeOps`]：对上游/DB 的 IO 抽象，测试注入 fake（对齐 Go Service 的 repo + provider）
//!
//! 纯状态（四池索引 / 监控）在 `grok-pool::build_pool` 与
//! `crate::build_probe::BuildProbeMonitor`，本模块只做编排与副作用映射。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use grok_domain::{Account, Billing, QuotaRecovery};
use grok_pool::build_pool::{
    build_account_pool_at, summarize_build_probe_pools, BuildPool, BuildPoolIndex,
};
use crate::build_probe::{BuildProbeMode, BuildProbeMonitor, BuildProbeStatus, ProbeFailure};
use crate::error::OpsResult;

/// dispatch 探针连续失败上限（Go `buildDispatchFailLimit`）。
pub const BUILD_DISPATCH_FAIL_LIMIT: i32 = 2;
/// 普通/调度探针失败的冷却时长（Go 15 分钟）。
pub const PROBE_COOLDOWN: Duration = Duration::minutes(15);

/// 单次 tick 结果（Go `MaintenanceProbeTick`/`DispatchProbeTick` 的 (id, found, err)）。
#[derive(Debug, Clone)]
pub struct TickResult {
    pub account_id: i64,
    pub found: bool,
    /// 探针失败分类；`None` = 成功（或无需探针）。
    pub failure: Option<ProbeFailure>,
}

impl TickResult {
    pub fn none() -> Self {
        Self { account_id: 0, found: false, failure: None }
    }
    fn account(id: i64, failure: Option<ProbeFailure>) -> Self {
        Self { account_id: id, found: true, failure }
    }
}

/// Build 探针 IO 抽象（对齐 Go Service 依赖的 repo + provider）。
#[async_trait]
pub trait BuildProbeOps: Send + Sync {
    async fn get_account(&self, id: i64) -> OpsResult<Option<Account>>;
    async fn list_build_accounts(&self, now: DateTime<Utc>) -> OpsResult<Vec<Account>>;
    async fn recoveries_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>>;
    async fn billings_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, Billing>>;
    async fn get_recovery(&self, id: i64) -> OpsResult<Option<QuotaRecovery>>;
    async fn get_billing(&self, id: i64) -> OpsResult<Option<Billing>>;

    /// 对齐 Go `prepareBuildProbeCredential`：必要时刷新令牌，返回 ready 账号。
    async fn prepare_credential(&self, account: &Account, refresh_tokens: bool) -> OpsResult<Account>;
    /// 对齐 Go `refreshBuildProbeBilling`。
    async fn refresh_billing(&self, id: i64) -> OpsResult<()>;

    /// 对齐 Go `probeBuildChatCredential`（验证轨）：2xx → observe_model + 健康清零并返回
    /// `Ok(model)`；401 / 403(permission-denied) → `mark_deletable` 并返回 `Err(消息)`；
    /// 其它状态 → 冷却并返回 `Err(消息)`。
    async fn probe_chat_credential(&self, account: &Account) -> Result<String, String>;
    /// 对齐 Go `probeBuildChatCapabilityOnly`（normal/dispatch 轨）：无副作用，仅探测；
    /// 非 2xx 返回 `Err("status N: snippet")`。
    async fn probe_chat_capability(&self, account: &Account) -> Result<String, String>;

    async fn observe_model(&self, id: i64, model: &str) -> OpsResult<()>;
    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        reason: &str,
        reset_last_success: bool,
    ) -> OpsResult<()>;
    async fn mark_deletable(&self, id: i64, reason: &str) -> OpsResult<()>;
    async fn clear_recovery(&self, id: i64) -> OpsResult<()>;
    async fn delete_account(&self, id: i64) -> OpsResult<()>;
}

/// Build 四池探针编排（Go `four_pool_probe.go` 的 Service 方法组）。
pub struct BuildFourPool {
    index: Mutex<BuildPoolIndex>,
    ops: Arc<dyn BuildProbeOps>,
    monitor: BuildProbeMonitor,
    now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl BuildFourPool {
    pub fn new(ops: Arc<dyn BuildProbeOps>) -> Self {
        Self::with_clock(ops, utc_now)
    }

    /// 注入时钟便于测试。
    pub fn with_clock<F>(ops: Arc<dyn BuildProbeOps>, now: F) -> Self
    where
        F: Fn() -> DateTime<Utc> + Send + Sync + 'static,
    {
        Self {
            index: Mutex::new(BuildPoolIndex::new()),
            ops,
            monitor: BuildProbeMonitor::new(),
            now: Box::new(now),
        }
    }

    pub fn configure(&self, interval: Duration, idle_interval: Duration, initial_delay: Duration) {
        self.monitor.configure((self.now)(), interval, idle_interval, initial_delay);
    }

    pub fn set_purge_apply(&self, enabled: bool) {
        self.monitor.set_purge_apply(enabled);
    }

    pub fn monitor(&self) -> &BuildProbeMonitor {
        &self.monitor
    }

    /// 全量重建四池索引（对齐 Go `RebuildBuildPoolIndex`）。
    pub async fn rebuild_index(&self) -> OpsResult<()> {
        let now = (self.now)();
        let accounts = self.ops.list_build_accounts(now).await?;
        let ids: Vec<i64> = accounts.iter().map(|a| a.id).collect();
        let recoveries = self.ops.recoveries_for(&ids).await?;
        let billings = self.ops.billings_for(&ids).await?;
        self.index.lock().unwrap().rebuild(&accounts, &recoveries, &billings, now);
        Ok(())
    }

    /// 索引全空时兜底重建（对齐 `ensurePoolIndexWarm`）。
    pub async fn ensure_warm(&self) -> OpsResult<()> {
        if self.index.lock().unwrap().is_empty() {
            self.rebuild_index().await?;
        }
        Ok(())
    }

    /// 单账号重新入池（对齐 `syncAccountIndex`）。
    pub async fn sync_account_index(&self, id: i64) {
        let now = (self.now)();
        let account = match self.ops.get_account(id).await {
            Ok(Some(a)) => a,
            Ok(None) | Err(_) => {
                self.index.lock().unwrap().remove(id);
                return;
            }
        };
        let recovery = self.ops.get_recovery(id).await.ok().flatten();
        let pool = build_account_pool_at(&account, now, recovery.as_ref());
        let billing = if pool == Some(BuildPool::Dispatch) {
            self.ops.get_billing(id).await.ok().flatten()
        } else {
            None
        };
        self.index
            .lock()
            .unwrap()
            .sync_account(&account, recovery.as_ref(), billing.as_ref(), now);
    }

    /// 维护探针（对齐 Go `MaintenanceProbeTick`）：DRR 选验证/普通/删除车道。
    pub async fn maintenance_tick(&self) -> OpsResult<TickResult> {
        self.ensure_warm().await?;
        let now = (self.now)();
        let pick = self.index.lock().unwrap().maintenance_next(now);
        let Some((lane, id)) = pick else {
            return Ok(TickResult::none());
        };
        let candidate = match self.ops.get_account(id).await {
            Ok(Some(a)) => a,
            Ok(None) | Err(_) => {
                self.sync_account_index(id).await;
                return Ok(TickResult::none());
            }
        };
        let mode = match lane {
            grok_pool::poolindex::Lane::Verification => BuildProbeMode::Verification,
            grok_pool::poolindex::Lane::Normal => BuildProbeMode::Normal,
            grok_pool::poolindex::Lane::Delete => BuildProbeMode::Delete,
        };
        Ok(self.observe(candidate, mode).await)
    }

    /// 调度探针（对齐 Go `DispatchProbeTick`）：只巡检调度池。
    pub async fn dispatch_tick(&self) -> OpsResult<TickResult> {
        self.ensure_warm().await?;
        let now = (self.now)();
        let id = match self.index.lock().unwrap().pop_dispatch_probe(now) {
            Some(id) => id,
            None => return Ok(TickResult::none()),
        };
        let candidate = match self.ops.get_account(id).await {
            Ok(Some(a)) => a,
            Ok(None) | Err(_) => {
                self.sync_account_index(id).await;
                return Ok(TickResult::none());
            }
        };
        let recovery = self.ops.get_recovery(id).await.ok().flatten();
        if build_account_pool_at(&candidate, now, recovery.as_ref()) != Some(BuildPool::Dispatch)
        {
            self.sync_account_index(id).await;
            return Ok(TickResult::account(id, None));
        }
        Ok(self.observe(candidate, BuildProbeMode::Dispatch).await)
    }

    /// 探针状态快照（监控 + 池规模汇总；对齐 Go `BuildProbeStatus`）。
    pub async fn status(&self) -> OpsResult<BuildProbeStatus> {
        let mut status = self.monitor.snapshot(Default::default());
        let now = (self.now)();
        let accounts = self.ops.list_build_accounts(now).await?;
        let ids: Vec<i64> = accounts.iter().map(|a| a.id).collect();
        let recoveries = self.ops.recoveries_for(&ids).await?;
        status.pools = summarize_build_probe_pools(&accounts, &recoveries, now);
        Ok(status)
    }

    // ── tick 执行 ────────────────────────────────────────────────

    async fn observe(&self, candidate: Account, mode: BuildProbeMode) -> TickResult {
        let started = (self.now)();
        self.monitor.start(&candidate, mode, started);
        let tick = match mode {
            BuildProbeMode::Verification => self.run_verification(candidate.clone()).await,
            BuildProbeMode::Normal => self.run_normal(candidate.clone()).await,
            BuildProbeMode::Dispatch => self.run_dispatch(candidate.clone()).await,
            BuildProbeMode::Delete => self.run_delete(candidate.clone()).await,
        };
        let completed = (self.now)();
        let updated = self
            .ops
            .get_account(candidate.id)
            .await
            .ok()
            .flatten()
            .unwrap_or(candidate);
        self.monitor
            .finish(&updated, mode, started, completed, tick.failure.as_ref());
        tick
    }

    /// 验证轨（对齐 Go `runVerificationProbe`）：prepare → billing → 凭据探测。
    async fn run_verification(&self, candidate: Account) -> TickResult {
        let ready = match self.ops.prepare_credential(&candidate, false).await {
            Ok(ready) => ready,
            Err(e) => {
                if is_terminal(&e) {
                    self.mark_deletable_and_index(candidate.id, &format!("verification refresh failed: {e}")).await;
                } else {
                    self.cooldown_health(candidate.id, candidate.failure_count, "", "probe", 0).await;
                    self.sync_account_index(candidate.id).await;
                }
                return TickResult::account(candidate.id, Some(ProbeFailure::Other(e.to_string())));
            }
        };
        let _ = self.ops.refresh_billing(ready.id).await;
        let failure = match self.ops.probe_chat_credential(&ready).await {
            Ok(_) => None,
            Err(text) => Some(ProbeFailure::Other(text)),
        };
        self.sync_account_index(candidate.id).await;
        TickResult::account(candidate.id, failure)
    }

    /// 普通轨（对齐 Go `runNormalProbe`）。
    async fn run_normal(&self, candidate: Account) -> TickResult {
        let _ = self.ops.refresh_billing(candidate.id).await;
        let ready = match self.ops.prepare_credential(&candidate, false).await {
            Ok(ready) => ready,
            Err(e) => {
                if is_terminal(&e) {
                    self.mark_deletable_and_index(candidate.id, &format!("normal refresh failed: {e}")).await;
                    return TickResult::account(candidate.id, Some(ProbeFailure::PurgeDeletable));
                }
                self.cooldown_health(candidate.id, candidate.failure_count, "normal probe refresh failed", "normal", 0).await;
                self.sync_account_index(candidate.id).await;
                return TickResult::account(candidate.id, Some(ProbeFailure::Other(e.to_string())));
            }
        };
        match self.ops.probe_chat_capability(&ready).await {
            Ok(observed) => {
                if !observed.trim().is_empty() {
                    let _ = self.ops.observe_model(ready.id, observed.trim()).await;
                }
                let _ = self.ops.update_health(ready.id, 0, None, "", true).await;
                let _ = self.ops.clear_recovery(ready.id).await;
                self.sync_account_index(ready.id).await;
                TickResult::account(ready.id, None)
            }
            Err(text) => {
                if is_terminal(&text) {
                    self.mark_deletable_and_index(ready.id, &text).await;
                    return TickResult::account(ready.id, Some(ProbeFailure::PurgeDeletable));
                }
                self.cooldown_health(ready.id, ready.failure_count, &format!("normal probe: {text}"), "normal", 0).await;
                self.sync_account_index(ready.id).await;
                TickResult::account(ready.id, Some(ProbeFailure::Other(text)))
            }
        }
    }

    /// 调度轨（对齐 Go `runDispatchProbe`；连续失败 ≥2 判可删）。
    async fn run_dispatch(&self, candidate: Account) -> TickResult {
        let ready = match self.ops.prepare_credential(&candidate, false).await {
            Ok(ready) => ready,
            Err(e) => {
                if is_terminal(&e) {
                    self.mark_deletable_and_index(candidate.id, &format!("dispatch refresh failed: {e}")).await;
                    return TickResult::account(candidate.id, Some(ProbeFailure::PurgeDeletable));
                }
                self.cooldown_health(candidate.id, candidate.failure_count, "dispatch probe refresh failed", "dispatch", 0).await;
                self.sync_account_index(candidate.id).await;
                return TickResult::account(candidate.id, Some(ProbeFailure::Other(e.to_string())));
            }
        };
        let _ = self.ops.refresh_billing(ready.id).await;
        match self.ops.probe_chat_capability(&ready).await {
            Ok(_) => {
                let _ = self.ops.update_health(ready.id, 0, None, "", true).await;
                self.sync_account_index(ready.id).await;
                self.index
                    .lock()
                    .unwrap()
                    .resched_dispatch_probe(ready.id, (self.now)());
                TickResult::account(ready.id, None)
            }
            Err(text) => {
                if is_terminal(&text) || ready.failure_count + 1 >= BUILD_DISPATCH_FAIL_LIMIT {
                    self.mark_deletable_and_index(ready.id, &text).await;
                    return TickResult::account(ready.id, Some(ProbeFailure::PurgeDeletable));
                }
                self.cooldown_health(ready.id, ready.failure_count, &format!("dispatch probe: {text}"), "dispatch", 0).await;
                self.sync_account_index(ready.id).await;
                TickResult::account(ready.id, Some(ProbeFailure::Other(text)))
            }
        }
    }

    /// 删除轨（对齐 Go `runDeleteProbe`）。
    async fn run_delete(&self, candidate: Account) -> TickResult {
        if !self.monitor.purge_apply_enabled() {
            return TickResult::account(
                candidate.id,
                Some(ProbeFailure::PurgeDeletable),
            );
        }
        let err_text = candidate
            .last_error
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if candidate.enabled
            && !err_text.starts_with(grok_pool::build_pool::DELETABLE_PREFIX)
            && !err_text.starts_with("retired:")
        {
            self.sync_account_index(candidate.id).await;
            return TickResult::account(candidate.id, None);
        }
        if self.ops.delete_account(candidate.id).await.is_err() {
            self.sync_account_index(candidate.id).await;
            return TickResult::account(candidate.id, Some(ProbeFailure::Other("delete failed".into())));
        }
        self.sync_account_index(candidate.id).await;
        TickResult::account(candidate.id, Some(ProbeFailure::PurgeDeleted))
    }

    // ── 副作用辅助 ───────────────────────────────────────────────

    async fn mark_deletable_and_index(&self, id: i64, reason: &str) {
        let _ = self.ops.mark_deletable(id, reason).await;
        self.sync_account_index(id).await;
    }

    /// 对齐 Go `cooldownBuildProbe`：UpdateHealth(fail+1, now+15min, reason)。
    #[allow(clippy::too_many_arguments)]
    async fn cooldown_health(
        &self,
        id: i64,
        failure_count: i32,
        reason: &str,
        _lane: &str,
        status: i32,
    ) {
        let msg = if reason.is_empty() {
            format!("build chat capability probe status {status}")
        } else {
            reason.to_string()
        };
        let _ = self
            .ops
            .update_health(id, failure_count + 1, Some((self.now)() + PROBE_COOLDOWN), &msg, false)
            .await;
    }
}

fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

fn is_terminal<E: std::fmt::Display>(e: &E) -> bool {
    grok_pool::build_pool::is_terminal_build_error(&e.to_string())
}