//! 只读仓储 trait 与 PG 实现（G0）。

use thiserror::Error;

/// 仓储错误类型（G0 最小集；G1+ 按 Go errors.go 细化）。
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("row decode error: {0}")]
    Decode(String),
    #[error("credential decrypt error: {0}")]
    Decrypt(String),
}
