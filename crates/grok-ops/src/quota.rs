//! Web quota refresh 后台任务（对齐 Go `runWebQuotaRefresh` + `refreshQuota`）。
//!
//! Go 语义（`account/service.go`）：从 DB 读账号现有额度窗口判断 mode（fast/auto/
//! imagine），然后调上游 `refreshQuota` 同步该账号额度窗口并存回 DB。
//!
//! Rust 移植边界：本 crate 不依赖 grok-gateway。上游 quota 同步抽象为本地
//! [`QuotaRefresher`] trait（测试注入 fake）；DB 读写经本地 [`QuotaStore`] trait
//! 抽象（测试用内存 fake，生产可桥接 grok-storage 的 `PgQuotaRepository`）。

use std::sync::Arc;

use async_trait::async_trait;
use grok_domain::{Account, QuotaWindow};

use crate::error::OpsResult;

/// 单账号单轮 quota 刷新结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaRefreshResult {
    pub account_id: i64,
    /// 本轮刷新的 mode（fast / auto / imagine）。
    pub mode: String,
    /// 刷新后剩余额度。
    pub remaining_after: i64,
    /// 是否命中 weekly 模式（对齐 Go 刷新优先 weekly 的语义）。
    pub refreshed_weekly: bool,
}

/// 额度上游同步端口。对齐 Go `providers.Quota().SyncQuota`。
#[async_trait]
pub trait QuotaRefresher: Send + Sync {
    /// 同步账号在指定 mode 的额度窗口，返回刷新后的窗口。
    async fn sync_quota(&self, account: &Account, mode: &str) -> OpsResult<QuotaWindow>;
}

/// 额度 DB 端口。生产可桥接 `grok-storage::repo::QuotaRepository`（读）与写路径。
#[async_trait]
pub trait QuotaStore: Send + Sync {
    /// 读账号现有额度窗口，用于判断刷新 mode（对齐 Go `GetQuotaWindows`）。
    async fn get_windows(&self, account_id: i64) -> OpsResult<Vec<QuotaWindow>>;
    /// 写回刷新后的额度窗口（对齐 Go `ReplaceQuotaWindows`）。
    async fn save_window(&self, window: QuotaWindow) -> OpsResult<()>;
}

/// Web quota refresh 后台任务。
///
/// - `run_once(account, hint_mode)`：读现有窗口 → 决定 mode → 上游同步 → 写回。
/// - `spawn_loop(interval, candidate_ids)`：周期性刷新一批账号（可禁用）。
#[derive(Clone)]
pub struct WebQuotaRefresh {
    refresher: Arc<dyn QuotaRefresher>,
    store: Arc<dyn QuotaStore>,
}

impl WebQuotaRefresh {
    pub fn new(refresher: Arc<dyn QuotaRefresher>, store: Arc<dyn QuotaStore>) -> Self {
        Self { refresher, store }
    }

    /// 单账号刷新一次。
    ///
    /// - `hint_mode` 为空时，优先取现有窗口中 `weekly` 的 mode（对齐 Go
    ///   `runWebQuotaRefresh` 的 weekly 优先），否则取现有窗口第一个，再退化为 `fast`。
    pub async fn run_once(
        &self,
        account: &Account,
        hint_mode: Option<&str>,
    ) -> OpsResult<QuotaRefreshResult> {
        let existing = self.store.get_windows(account.id).await?;
        let mode = resolve_refresh_mode(&existing, hint_mode);
        let fresh = self.refresher.sync_quota(account, &mode).await?;
        self.store.save_window(fresh.clone()).await?;
        Ok(QuotaRefreshResult {
            account_id: account.id,
            mode: mode.clone(),
            remaining_after: fresh.remaining,
            refreshed_weekly: mode == "weekly",
        })
    }

    /// 周期性刷新一批账号。单个账号失败只记录日志，不中断整轮。
    pub async fn spawn_loop(
        self,
        interval: std::time::Duration,
        candidate_ids: Vec<i64>,
        accounts: Vec<Account>,
    ) {
        let by_id: std::collections::HashMap<i64, Account> =
            accounts.into_iter().map(|a| (a.id, a)).collect();
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            for id in &candidate_ids {
                let Some(account) = by_id.get(id) else {
                    tracing::warn!("quota_refresh_skip_missing_account: {id}");
                    continue;
                };
                let account = account.clone();
                let account_id = account.id;
                match tokio::task::spawn({
                    let this = self.clone();
                    async move { this.run_once(&account, None).await }
                })
                .await
                {
                    Ok(Ok(r)) => tracing::info!(
                        "web_quota_refresh_succeeded account={} mode={} remaining={}",
                        r.account_id,
                        r.mode,
                        r.remaining_after
                    ),
                    Ok(Err(e)) => {
                        tracing::warn!("web_quota_refresh_failed account={account_id} error={e}")
                    }
                    Err(e) => tracing::warn!("web_quota_refresh_task_panicked: {e}"),
                }
            }
        }
    }
}

/// 决定刷新 mode：hint 优先，否则 weekly，否则现有窗口首个，否则 fast。
fn resolve_refresh_mode(existing: &[QuotaWindow], hint: Option<&str>) -> String {
    if let Some(h) = hint {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    if existing.iter().any(|w| w.mode == "weekly") {
        return "weekly".to_string();
    }
    if let Some(first) = existing.first() {
        return first.mode.clone();
    }
    "fast".to_string()
}

// bridge 到 grok-storage 的只读 `QuotaRepository`（读）需调用方保证 Send+Sync；
// 此处不内置 StorageQuotaStore，避免 trait 无 Send+Sync 边界的编译耦合。
