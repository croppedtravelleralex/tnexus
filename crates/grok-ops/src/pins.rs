//! Pin 同步（对齐 Go `SyncImageDispatchPins` + `web_pool_pins.go`）。
//!
//! Go 语义：读取 `grok_model_route_accounts` 里某路由已 pin 的账号集合，与当前
//! pool dispatch 集合对齐 —— 新增不在 pool 但应 pin 的账号、移除已撤销的 pin，
//! 并把最终 pin 集合应用到 pool（`RebuildWebPoolIndex`）。
//!
//! Rust 移植边界：pool 当前是**简化单池**（G1），pin 由 `SimplifiedPool::pin`
//! 表达（单一 pin 位，无 Go 的路由绑定集合模型）。故：
//! - `RoutePinRepository::read_bound_ids` 读路由 pin 账号集（测试用内存 fake）。
//! - `PinSyncTask::run_once` 把「首个应 pin 账号」应用到 pool 的单一 pin 位；
//!   集合差异（added/removed）以 [`PinSyncResult`] 汇报，供后续接入多 pin 模型。

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use grok_domain::Provider;
use grok_pool::SimplifiedPool;

use crate::error::OpsResult;

/// 路由 pin 绑定仓储端口。生产可桥接 `grok_model_route_accounts` 查询。
#[async_trait]
pub trait RoutePinRepository: Send + Sync {
    /// 返回指定 provider + route 上已 pin 的账号 id 集合。
    async fn read_bound_ids(&self, provider: Provider, route: &str) -> OpsResult<BTreeSet<i64>>;
}

/// 一次 pin 同步的差异汇报。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinSyncResult {
    /// 是否发生了 pin 集合变更并应用。
    pub changed: bool,
    /// 目标（应 pin）账号集合。
    pub target_ids: BTreeSet<i64>,
    /// 变更前已 pin 账号集合。
    pub previous_ids: BTreeSet<i64>,
    /// 本轮新增 pin 的账号。
    pub added_ids: BTreeSet<i64>,
    /// 本轮移除 pin 的账号。
    pub removed_ids: BTreeSet<i64>,
}

/// Pin 同步后台任务。
///
/// - `run_once()`：读路由 pin → 计算差异 → 应用到 pool 单一 pin 位。
/// - `spawn_loop(interval)`：按 interval 循环（可禁用）。
pub struct PinSyncTask {
    pool: Arc<SimplifiedPool>,
    routes: Arc<dyn RoutePinRepository>,
    provider: Provider,
    route: String,
}

impl PinSyncTask {
    pub fn new(
        pool: Arc<SimplifiedPool>,
        routes: Arc<dyn RoutePinRepository>,
        route: &str,
    ) -> Self {
        Self {
            pool,
            routes,
            provider: Provider::GrokWeb,
            route: route.to_string(),
        }
    }

    /// 单轮 pin 同步：读当前绑定 → 计算差异 → 应用单 pin 位。
    pub async fn run_once(&self) -> OpsResult<PinSyncResult> {
        let target_ids = self
            .routes
            .read_bound_ids(self.provider, &self.route)
            .await?;
        let previous_ids: BTreeSet<i64> = match self.pool.pinned().await {
            Some(id) => BTreeSet::from([id]),
            None => BTreeSet::new(),
        };
        let added_ids: BTreeSet<i64> = target_ids.difference(&previous_ids).copied().collect();
        let removed_ids: BTreeSet<i64> = previous_ids.difference(&target_ids).copied().collect();
        let changed = !added_ids.is_empty() || !removed_ids.is_empty();

        // 简化单池：只保留目标里的一个 pin 位。取目标集合最小值保证确定性。
        match target_ids.iter().next() {
            Some(&pin_id) => {
                if previous_ids != target_ids {
                    self.pool.pin(pin_id).await;
                }
            }
            None => {
                if previous_ids.is_empty() {
                    // 无变更
                } else {
                    self.pool.unpin().await;
                }
            }
        }

        Ok(PinSyncResult {
            changed,
            target_ids,
            previous_ids,
            added_ids,
            removed_ids,
        })
    }

    /// 循环执行 pin 同步。
    pub async fn spawn_loop(self, interval: std::time::Duration) {
        let task = Arc::new(self);
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let this = Arc::clone(&task);
            match tokio::task::spawn(async move { this.run_once().await }).await {
                Ok(Ok(r)) => tracing::info!(
                    "pin_sync_succeeded changed={} added={} removed={}",
                    r.changed,
                    r.added_ids.len(),
                    r.removed_ids.len()
                ),
                Ok(Err(e)) => tracing::warn!("pin_sync_failed: {e}"),
                Err(e) => tracing::warn!("pin_sync_task_panicked: {e}"),
            }
        }
    }
}
