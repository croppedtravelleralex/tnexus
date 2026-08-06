//! grok-egress — Egress 出口 Scope / lease 基础件（G1）。
//!
//! 对应 Go `backend/internal/domain/egress` 的 Scope 与 lease 抽象，以及
//! `backend/internal/infra/egress/manager.go` 的并发闸门语义。
//!
//! **G1 范围**：仅 lease 基础 + `grok_web` scope 的并发闸门（单实例内存实现）。
//! 完整 manager（node selection / asset affinity / traffic hops / Redis ZSET
//! `concurrency` lease）属 G2+ 或 G3（Redis runtime）。

pub mod lease;
pub mod memory;
pub mod redis;

pub use lease::{Error, GateId, Lease, LeaseManager};
pub use memory::InMemoryLeaseManager;
pub use redis::RedisLeaseManager;

/// G1 仅启用 `grok_web` scope；其余 scope 在后续 Phase 逐步开放。
/// `grok_web_expand` 不入库（仅运行时问），由上游回退到 grok_web（见 39 主文档 §2.2）。
pub const G1_ENABLED_SCOPES: &[grok_domain::egress::Scope] = &[grok_domain::egress::Scope::GrokWeb];
