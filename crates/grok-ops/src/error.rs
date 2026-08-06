//! 统一错误类型。

use thiserror::Error;

/// grok-ops 错误类型。
#[derive(Debug, Error)]
pub enum OpsError {
    #[error("storage error: {0}")]
    Storage(#[from] grok_storage::StorageError),
    #[error("pin repository error: {0}")]
    Pin(String),
    #[error("quota repository error: {0}")]
    Quota(String),
    #[error("probe backend error: {0}")]
    Probe(String),
    #[error("account not found: {0}")]
    NotFound(i64),
}

pub type OpsResult<T> = Result<T, OpsError>;
