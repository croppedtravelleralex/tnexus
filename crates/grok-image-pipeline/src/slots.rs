//! 生图流水线自身并发槽（`imagine slots`）管理。
//!
//! 与 egress 的出口并发闸门（`grok-egress::LeaseManager`）不同：这是 image
//! pipeline 在 PS / SS 等阶段的**有界槽位池**，语义对齐 Go
//! `application/imagepipeline/scheduler.go` 的 `acquirePoolSlot`（psSlots / ssSlots）。
//!
//! G2 为单实例内存实现（`tokio::sync::Semaphore`），Redis 跨实例租约留 G3。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::PipelineError;

/// 某个具名阶段的槽位池（如 `"ps"`、`"ss"`）。
#[derive(Debug)]
pub struct SlotPool {
    name: &'static str,
    limit: usize,
    sem: Arc<Semaphore>,
}

impl SlotPool {
    /// 新建具名槽位池，`limit` 为并发上限。
    pub fn new(name: &'static str, limit: usize) -> Self {
        Self {
            name,
            limit,
            sem: Arc::new(Semaphore::new(limit)),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// 当前已占用槽位数。
    pub fn active(&self) -> usize {
        self.limit.saturating_sub(self.sem.available_permits())
    }

    /// 尝试在原子上获得一个槽；taken 后又因故未能使用，应立刻归还。
    pub(crate) fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.sem.clone().try_acquire_owned().ok()
    }

    pub(crate) async fn acquire_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<OwnedSemaphorePermit, PipelineError> {
        match tokio::time::timeout(timeout, self.sem.clone().acquire_owned()).await {
            Ok(Ok(p)) => Ok(p),
            Ok(Err(_)) => Err(PipelineError::SlotPoolClosed(self.name.to_string())),
            Err(_) => Err(PipelineError::SlotTimeout {
                pool: self.name.to_string(),
                timeout,
            }),
        }
    }
}

/// 一次成功获得的槽位；`Drop` 时自动归还（`OwnedSemaphorePermit` RAII）。
pub struct SlotGuard {
    pool: &'static str,
    slot: usize,
    /// 底层信号量 permit；字段 drop 自动归还。
    _permit: OwnedSemaphorePermit,
}

impl SlotGuard {
    fn new(pool: &'static str, slot: usize, permit: OwnedSemaphorePermit) -> Self {
        Self {
            pool,
            slot,
            _permit: permit,
        }
    }

    pub fn pool(&self) -> &'static str {
        self.pool
    }

    /// 分配到的槽位下标（0..limit）。
    pub fn slot(&self) -> usize {
        self.slot
    }
}

/// 生图流水线槽位管理器：持有多个具名 `SlotPool`（PS / SS / …）。
#[derive(Debug, Clone)]
pub struct SlotManager {
    pools: Arc<Vec<Arc<SlotPool>>>,
}

impl SlotManager {
    /// 从 `[(stage_name, limit), …]` 构建槽位池。
    pub fn new(pools: &[(&'static str, usize)]) -> Self {
        Self {
            pools: Arc::new(
                pools
                    .iter()
                    .map(|(n, l)| Arc::new(SlotPool::new(n, *l)))
                    .collect(),
            ),
        }
    }

    pub fn pool(&self, name: &str) -> Option<Arc<SlotPool>> {
        self.pools.iter().find(|p| p.name() == name).cloned()
    }

    /// 池上的现占用数（缺省 0）。
    pub fn active(&self, name: &str) -> usize {
        self.pool(name).map(|p| p.active()).unwrap_or(0)
    }

    /// 非阻塞获取一个槽位。
    pub fn try_reserve(&self, pool: &str) -> Result<SlotGuard, PipelineError> {
        let p = self
            .pool(pool)
            .ok_or_else(|| PipelineError::UnknownPool(pool.to_string()))?;
        // 分配槽位下标：active 位 + (队列外)。信号量许可即互斥保证。
        let slot_idx = p.active().min(p.limit().saturating_sub(1));
        let permit = p.try_acquire().ok_or_else(|| PipelineError::PoolFull {
            pool: pool.to_string(),
        })?;
        Ok(SlotGuard::new(p.name(), slot_idx, permit))
    }

    /// 阻塞式获取（带超时）。
    pub async fn reserve(&self, pool: &str, timeout: Duration) -> Result<SlotGuard, PipelineError> {
        let p = self
            .pool(pool)
            .ok_or_else(|| PipelineError::UnknownPool(pool.to_string()))?;
        let permit = p.acquire_with_timeout(timeout).await?;
        let slot_idx = p.active().min(p.limit().saturating_sub(1));
        Ok(SlotGuard::new(p.name(), slot_idx, permit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> SlotManager {
        SlotManager::new(&[("ps", 2), ("ss", 1)])
    }

    #[tokio::test]
    async fn reserve_then_drop_restores() {
        let m = manager();
        let g = m
            .reserve("ps", Duration::from_secs(1))
            .await
            .expect("acquire");
        assert_eq!(g.pool(), "ps");
        assert!(g.slot() < 2);
        assert_eq!(m.active("ps"), 1);
        drop(g);
        assert_eq!(m.active("ps"), 0);
    }

    #[tokio::test]
    async fn slot_limit_blocks_with_timeout() {
        let m = manager(); // ss limit=1
        let _g1 = m.reserve("ss", Duration::from_secs(1)).await.expect("g1");
        // 第二人争用 30ms 内拿不到 → 超时
        let r = m.reserve("ss", Duration::from_millis(30)).await;
        assert!(matches!(r, Err(PipelineError::SlotTimeout { .. })));
    }

    #[tokio::test]
    async fn distinct_pools_are_independent() {
        let m = manager();
        let gp = m.reserve("ps", Duration::from_secs(1)).await.unwrap();
        let gs = m.reserve("ss", Duration::from_secs(1)).await.unwrap();
        assert_eq!(m.active("ps"), 1);
        assert_eq!(m.active("ss"), 1);
        drop(gp);
        drop(gs);
    }

    #[tokio::test]
    async fn unknown_pool_rejected() {
        let m = manager();
        let r = m.reserve("nope", Duration::from_secs(1)).await;
        assert!(matches!(r, Err(PipelineError::UnknownPool(_))));
    }

    #[tokio::test]
    async fn try_reserve_nonblocking() {
        let m = manager();
        let g1 = m.try_reserve("ss").expect("g1");
        // 池满 → PoolFull
        assert!(matches!(
            m.try_reserve("ss"),
            Err(PipelineError::PoolFull { .. })
        ));
        drop(g1);
        assert!(m.try_reserve("ss").is_ok());
    }
}
