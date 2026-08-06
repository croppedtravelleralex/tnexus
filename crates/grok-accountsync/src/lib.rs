//! grok-accountsync — Grok 账号同步服务（G4-P5，Go `accountsync` 移植）。

pub mod error;
pub mod service;

pub use error::Error;
pub use service::{
    AccountSyncService, Provider, QuotaKind, SyncBackend, SyncResult, DEFAULT_WORKER_COUNT,
    OPERATION_TIMEOUT,
};
