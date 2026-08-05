//! 单实例内存并发闸门实现（G1）。
//!
//! 用 `tokio::sync::Semaphore` 为每个 `(scope, gate)` 维护一个有界槽位池，
//! 语义对应 Go `acquireScope` 的通道闸门。多实例 `Redis` lease（G3）再替换。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use grok_domain::egress::Scope;

use crate::lease::{Error, GateId, Lease, LeaseManager};

/// 单个 (scope, gate) 的闸门：有界信号量 + 上限。
#[derive(Debug, Clone)]
struct Gate {
    sem: Arc<Semaphore>,
}

impl Gate {
    fn new(limit: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(limit)),
        }
    }
}

/// 内存并发闸门。
#[derive(Debug, Clone, Default)]
pub struct InMemoryLeaseManager {
    /// (scope, gate) -> 闸门；按 (scope, gate) 分组，互不影响。
    gates: Arc<Mutex<HashMap<(String, String), Gate>>>,
    /// scope -> 并发上限。G1 仅 grok_web=1；其余 scope 未启用。
    limits: Arc<Mutex<HashMap<String, usize>>>,
}

impl InMemoryLeaseManager {
    /// 新建实例，可覆盖各 scope 并发上限。缺省 `grok_web=1`，其余 scope=1。
    pub fn new(limits: &[(Scope, usize)]) -> Self {
        let m = Self::default();
        {
            let mut map = m.limits.lock().unwrap();
            for (scope, n) in limits {
                map.insert(scope.as_str().to_string(), *n);
            }
        }
        m
    }

    /// scope 并发上限（缺省 1）。
    fn limit_for(&self, scope: Scope) -> usize {
        self.limits
            .lock()
            .unwrap()
            .get(&scope.as_str().to_string())
            .copied()
            .unwrap_or(1)
    }

    /// 取或建闸门（std 登记 + 信号量本身可跨 await）。
    fn gate_for(&self, scope: Scope, gate: &str) -> Gate {
        let key = (scope.as_str().to_string(), gate.to_string());
        let limit = self.limit_for(scope);
        let mut map = self.gates.lock().unwrap();
        map.entry(key).or_insert_with(|| Gate::new(limit)).clone()
    }
}

#[async_trait::async_trait]
impl LeaseManager for InMemoryLeaseManager {
    async fn acquire(&self, scope: Scope, gate: GateId, lease: Duration) -> Result<Lease, Error> {
        // G1 仅启 grok_web；其余 scope 显式拒绝（不静默放宽）。
        if !crate::G1_ENABLED_SCOPES.contains(&scope) {
            return Err(Error::ScopeUnsupported(scope.as_str().to_string()));
        }
        let gate_obj = self.gate_for(scope, &gate);
        let acquired = tokio::time::timeout(lease, gate_obj.sem.clone().acquire_owned()).await;
        let permit: OwnedSemaphorePermit = match acquired {
            Ok(Ok(p)) => p,
            Ok(Err(_)) => return Err(Error::Canceled), // semaphore closed（本实现不关）
            Err(_) => return Err(Error::Timeout(lease)),
        };
        Ok(Lease::new(scope, gate, permit))
    }

    fn release(&self, lease: Lease) {
        // Lease drop 自动归还 permit；此处为 trait 提供的显式通道。
        lease.release();
    }

    fn active(&self, scope: Scope, gate: &str) -> usize {
        let gate_obj = self.gate_for(scope, gate);
        let limit = self.limit_for(scope);
        limit.saturating_sub(gate_obj.sem.available_permits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::LeaseManager;
    use std::time::Duration;

    fn manager() -> InMemoryLeaseManager {
        InMemoryLeaseManager::new(&[(Scope::GrokWeb, 2)])
    }

    #[tokio::test]
    async fn acquire_and_release_then_available() {
        let m = manager();
        let l1 = m
            .acquire(Scope::GrokWeb, "default".into(), Duration::from_secs(2))
            .await
            .expect("first acquire");
        assert_eq!(l1.scope(), Scope::GrokWeb);
        assert_eq!(m.active(Scope::GrokWeb, "default"), 1);

        l1.release();
        assert_eq!(m.active(Scope::GrokWeb, "default"), 0);
    }

    #[tokio::test]
    async fn concurrency_ceiling_blocks_then_free() {
        let m = manager(); // web limit=2
        let a = m
            .acquire(Scope::GrokWeb, "default".into(), Duration::from_secs(5))
            .await
            .expect("a");
        let b = m
            .acquire(Scope::GrokWeb, "default".into(), Duration::from_secs(5))
            .await
            .expect("b");
        assert_eq!(m.active(Scope::GrokWeb, "default"), 2);

        // 第三个在 30ms 内拿不到槽 → Timeout
        let c = m
            .acquire(Scope::GrokWeb, "default".into(), Duration::from_millis(30))
            .await;
        assert!(
            matches!(c, Err(Error::Timeout(_))),
            "expected timeout, got {c:?}"
        );

        // 释放一个后，等待者可获槽
        let m2 = m.clone();
        let handle = tokio::spawn(async move {
            m2.acquire(Scope::GrokWeb, "default".into(), Duration::from_secs(2))
                .await
        });
        tokio::task::yield_now().await;
        a.release();
        let c = handle.await.expect("join").expect("slot after release");
        assert_eq!(m.active(Scope::GrokWeb, "default"), 2);
        c.release();
        b.release();
    }

    #[tokio::test]
    async fn unsupported_scope_rejected() {
        let m = manager();
        let r = m
            .acquire(Scope::GrokWebAsset, "g".into(), Duration::from_secs(2))
            .await;
        assert!(matches!(r, Err(Error::ScopeUnsupported(_))));
    }

    #[tokio::test]
    async fn distinct_gates_do_not_share_slots() {
        let m = manager();
        let a = m
            .acquire(Scope::GrokWeb, "g1".into(), Duration::from_secs(5))
            .await
            .expect("g1");
        let b = m
            .acquire(Scope::GrokWeb, "g2".into(), Duration::from_secs(5))
            .await
            .expect("g2");
        assert_eq!(m.active(Scope::GrokWeb, "g1"), 1);
        assert_eq!(m.active(Scope::GrokWeb, "g2"), 1);
        a.release();
        b.release();
    }
}
