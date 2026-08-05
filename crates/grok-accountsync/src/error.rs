//! 统一错误类型。

use thiserror::Error;

/// grok-accountsync 错误。
#[derive(Debug, Error)]
pub enum Error {
    /// 后端 IO 或上游失败（Go `errors.Join` 合并的三路同步错误文本）。
    #[error("{0}")]
    Backend(String),
    /// 单次上游操作超过 `operationTimeout`（Go `context.WithTimeout` 触发）。
    #[error("operation timed out")]
    Timeout,
}