//! Lease 抽象与并发闸门 trait。
//!
//! 语义对齐 Go `infra/egress/manager.go` 的 per-scope 有界闸门：
//! - `acquire` 成功即占用 scope 的一个并发槽位；
//! - `release` 释放该槽位，让后续等待者继续；
//! - 槽位数 = scope 并发上限（Go 默认 web=1、asset=4、expand=2；G1 web=1）。
//!
//! G1 为单实例内存实现（记忆/内存闸门）。Redis ZSET `concurrency` 多实例 lease
//! （docs/39b §5）属 G3；`gate` 参数已为 Redis `{scope}:{gate}` key 维度预留，
//! 当前内存实现按其值分组（同一 gate 内计数）。

use std::time::Duration;

use grok_domain::egress::Scope;

/// 并发闸门标识：Redis key 维度 `{scope}:{gate}`（docs/39b §5 `concurrency`）。
/// G1 内存实现将其作为分组键；多实例租约透传到 G3。
pub type GateId = String;

/// Lease 错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// scope 未启用（G1 仅 grok_web）。
    #[error("egress scope {0} not enabled in this phase")]
    ScopeUnsupported(String),
    /// 在等待槽位期间超时（耗时超过 lease duration）。
    #[error("failed to acquire lease within {0:?}")]
    Timeout(Duration),
    /// 在等待槽位期间上下文取消。
    #[error("acquisition canceled")]
    Canceled,
}

/// 一次成功获得的出口并发租约。
///
/// 持有期间独占 scope 内的一个并发槽位；`release(self)` 释放。
/// 采用 RAII：`Lease` 被 drop 时自动释放底层 permit，避免泄漏。
#[derive(Debug)]
pub struct Lease {
    scope: Scope,
    gate: GateId,
    /// 底层信号量 permit；drop 时自动归还。
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Lease {
    pub(crate) fn new(
        scope: Scope,
        gate: GateId,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Self {
        Self {
            scope,
            gate,
            _permit: Some(permit),
        }
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    pub fn gate(&self) -> &str {
        &self.gate
    }

    /// 显式释放槽位；重复调用与 drop 后调用均为 no-op（permmit 已消费）。
    pub fn release(mut self) {
        self._permit.take();
    }
}

/// 出口并发闸门。
#[async_trait::async_trait]
pub trait LeaseManager: Send + Sync {
    /// 获取指定 scope + gate 的一次并发槽位。
    ///
    /// - 槽位立即可用 → 立即返回 `Ok(Lease)`；
    /// - 槽位已满 → 等待至空闲，等待期受 `lease`（或 ctx）约束；
    /// - scope 未启用 → `Error::ScopeUnsupported`。
    async fn acquire(&self, scope: Scope, gate: GateId, lease: Duration) -> Result<Lease, Error>;

    /// 释放槽位（等价的显式释放；`Lease::release` 与 drop 也可）。
    /// 未 trace 到持有中的 lease 时静默忽略。
    fn release(&self, lease: Lease);

    /// 当前已占用的槽位数（scope×gate）。
    fn active(&self, scope: Scope, gate: &str) -> usize;
}
