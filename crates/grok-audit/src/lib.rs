//! grok-audit — Grok 推理请求审计（异步写 `grok_request_audits`）。
//!
//! 对应 docs/39e G1-P5、39a G1-7、39b 表 21。G1 只需写路径：
//! - [`audit::CreateAudit`]：领域记录（对齐 Go `domain/audit.Record`）。
//! - [`repo`]：`AuditRepository` trait + PG 批量实现 + 测试 fake。
//! - [`sink::AuditSink`]：有界 mpsc 缓冲 + 后台批量写；DB 不可达时计数丢弃，
//!   绝不阻塞推理路径。
//!
//! 聚合/查询（`domain/audit.Summary` / admin 端点）属 G4，不做。

pub mod audit;
pub mod repo;
pub mod sink;

pub use audit::{CreateAudit, Operation, UsageSource};
pub use repo::{AuditRepoError, AuditRepository, FakeAuditRepository, PgAuditRepository};
pub use sink::{AuditSink, AuditStats, SinkError};
