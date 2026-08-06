//! grok-provider-build 错误模型（对齐 grok-provider-web::error 层级约定）。

use thiserror::Error;

/// Provider Build 错误。
#[derive(Debug, Error)]
pub enum ProviderError {
    /// 请求在协议层无效（解析失败 / 参数非法），应映射 HTTP 400。
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// 上游返回非成功或不可解析。
    #[error("upstream error: {0}")]
    Upstream(String),

    /// HTTP 传输层错误（连接失败等）。
    #[error("http error: {0}")]
    Http(String),

    /// 请求超时（对齐 console `Timeout` 层级约定）。
    #[error("request timed out after {0:?}")]
    Timeout(std::time::Duration),
}
