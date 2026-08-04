//! grok-storage — Grok 子系统 PG 只读仓储（G0 最小集）。
//!
//! 模块映射见 docs/39d-grok-go-rust-map.md §6/§7：
//! 账号 / 凭据 / 额度网关 只读 SELECT 路径属于 G0（39e G0-P2）。
//! 写路径（导入、配额扣减、probe 更新等）留待 G1+，对应 Go
//! `persistence/relational/account_repository.go`、`credential`、`quota`。
//!
//! 表名为 `grok_*`（grok_accounts / grok_credentials / grok_quota_windows），
//! 取自 migrations/010_grok_core.sql 与 011_grok_quota_models.sql。

pub mod error;
pub mod repo;

pub use error::StorageError;
