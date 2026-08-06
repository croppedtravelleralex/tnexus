//! Web 调度探针（对齐 Go `WebDispatchProbeTick` + `runWebDispatchProbe`）。
//!
//! 语义（Go `web_pool_probe.go`）：从 pool 里取一个待探针账号，对上游做一次
//! dispatch 试探。成功后 `dispatch_success` 记账并 `UpdateHealth` 归零失败计数；
//! 失败则 `dispatch_failure` 记账（进入冷却）。
//!
//! Rust 移植边界：grok-pool 当前是**简化单池**（G1），无 Go 的 `dispatchProbeHeap`
//! 到期堆与 lane 分轨。因此 `WebDispatchProbe::run_once` 直接对池内账号轮询试探，
//! 保留核心闭环（select → probe → 记账），探针抽成 [`ProbeBackend`] trait 让测试
//! 注入 fake，另可在 spawn 循环里多账号轮转。

use std::sync::Arc;

use async_trait::async_trait;
use grok_domain::{Account, WebLane};
use grok_pool::SimplifiedPool;

use crate::error::OpsResult;

/// 每次探查的探针账号数上限（对齐 Go 的 pop-due 单 tick 取 1 的语义放大到分批）。
pub const PROBE_BATCH: usize = 8;

/// 单个账号 dispatch 试探的上行副作用。
///
/// 对齐 Go `runWebDispatchProbe` 的 `probeWebQuotaL0`（L0 = 仅 dispatch 试探，
/// 不做真实推理）。返回是否成功，以及可选的恢复提示。
#[async_trait]
pub trait ProbeBackend: Send + Sync {
    /// 对指定账号在指定轨做一次 dispatch 试探。
    async fn dispatch_probe(&self, account: &Account, lane: WebLane) -> OpsResult<bool>;
}

/// Web 调度探针后台任务。
///
/// - `run_once()`：对池内账号做一轮 dispatch 试探（默认 `PROBE_BATCH` 个），并记账。
/// - `spawn_loop(interval)`：包成 `tokio` task 按 interval 循环，捕获 panic 续跑
///   （G3-A4 需 24h 无 panic）。
#[derive(Clone)]
pub struct WebDispatchProbe {
    pool: Arc<SimplifiedPool>,
    backend: Arc<dyn ProbeBackend>,
}

impl WebDispatchProbe {
    pub fn new(pool: Arc<SimplifiedPool>, backend: Arc<dyn ProbeBackend>) -> Self {
        Self { pool, backend }
    }

    /// 单轮 dispatch 探针：轮询池内账号，逐个 dispatch 试探并按结果记账。
    ///
    /// 返回探针过的账号数。空池 / 全冷却 → 0（对应 Go `found=false` → idle interval）。
    pub async fn run_once(&self, lane: WebLane) -> OpsResult<usize> {
        let ids = self.pool.account_ids().await;
        let mut probed = 0usize;
        for account_id in ids.into_iter().take(PROBE_BATCH) {
            if self.pool.in_cooldown(account_id).await {
                continue;
            }
            let account = self.account(account_id).await?;
            let ok = self.backend.dispatch_probe(&account, lane).await?;
            if ok {
                self.pool.dispatch_success(account_id).await;
            } else {
                self.pool.dispatch_failure(account_id).await;
            }
            probed += 1;
        }
        Ok(probed)
    }

    /// 循环运行探针。每轮之间按 `interval` 等待；对端 panic 被捕获并续跑。
    pub async fn spawn_loop(self, interval: std::time::Duration) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let lane = WebLane::Image; // G1 单池无 lane 分轨，固定 Image 语义
            match tokio::task::spawn({
                let this = self.clone();
                async move { this.run_once(lane).await }
            })
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("web_dispatch_probe_failed: {e}"),
                Err(e) => tracing::warn!("web_dispatch_probe_task_panicked: {e}"),
            }
        }
    }

    async fn account(&self, account_id: i64) -> OpsResult<Account> {
        // 简化单池只暴露账号 id；从池内构造一个最小 Account 供后端使用。
        Ok(Account {
            id: account_id,
            ..Default::default()
        })
    }
}
