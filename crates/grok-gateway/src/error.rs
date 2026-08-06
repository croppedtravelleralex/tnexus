//! grok-gateway 错误模型 + HTTP 映射（docs/39c §1 层级约定）。
//!
//! 错误 → 状态码：
//! - [`grok_conversation::ConversationError`]（协议校验）→ 400
//! - [`grok_provider_web::ProviderError::InvalidRequest`] → 400
//! - `NoAvailableAccount` → 503（空池/全冷却）
//! - `Lease`（egress 超时）→ 429
//! - `Bridge` / `Upstream`（上游） → 502
//!
//! 其余（内部失真/注入错误）→ 500。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use grok_conversation::ConversationError;
use grok_domain::ProviderError;
use serde_json::json;

/// 推理端点错误，携带 HTTP 状态码（OAI 风格 `{error:{message,type,param}}`）。
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// 请求协议层无效（conversation 校验 / provider InvalidRequest）。
    #[error("{0}")]
    InvalidRequest(String),
    /// 号池没有可用账号。
    #[error("no available grok_web account")]
    NoAvailableAccount,
    /// egress lease 超时 / scope 未启用。
    #[error("{0}")]
    Lease(String),
    /// 上游 / bridge 调用失败。
    #[error("{0}")]
    Upstream(String),
    /// 上游未配置（缺 token / base_url）→ 503，绝不带空凭据外呼真实上游。
    #[error("upstream not configured for this endpoint")]
    NotConfigured,
    /// 内部失真（engine 未注入 / 其它工具错误）。
    #[error("internal: {0}")]
    Internal(String),
    /// 资源不存在（如视频任务 id 未命中）。
    #[error("{0}")]
    NotFound(String),
}

impl GatewayError {
    fn status(&self) -> StatusCode {
        match self {
            GatewayError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            GatewayError::NoAvailableAccount => StatusCode::SERVICE_UNAVAILABLE,
            GatewayError::NotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            GatewayError::Lease(_) => StatusCode::TOO_MANY_REQUESTS,
            GatewayError::Upstream(_) => StatusCode::BAD_GATEWAY,
            GatewayError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            GatewayError::NotFound(_) => StatusCode::NOT_FOUND,
        }
    }

    fn oai_error(&self) -> String {
        let (msg, ty) = match self {
            GatewayError::InvalidRequest(_)
            | GatewayError::NoAvailableAccount
            | GatewayError::NotConfigured
            | GatewayError::Internal(_) => (self.to_string(), "invalid_request_error"),
            GatewayError::Lease(_) => (self.to_string(), "rate_limit_error"),
            GatewayError::Upstream(_) => (self.to_string(), "upstream_error"),
            GatewayError::NotFound(_) => (self.to_string(), "not_found"),
        };
        json!({
            "error": { "message": msg, "type": ty, "param": null, "code": null },
        })
        .to_string()
    }
}

impl From<ConversationError> for GatewayError {
    fn from(e: ConversationError) -> Self {
        GatewayError::InvalidRequest(e.to_string())
    }
}

impl From<ProviderError> for GatewayError {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::InvalidRequest(msg) => GatewayError::InvalidRequest(msg),
            ProviderError::NoAvailableAccount => GatewayError::NoAvailableAccount,
            ProviderError::Lease(inner) => GatewayError::Lease(inner.to_string()),
            ProviderError::Bridge(msg) => GatewayError::Upstream(format!("bridge: {msg}")),
            ProviderError::Upstream(msg) => GatewayError::Upstream(msg),
        }
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let code = self.status();
        let body = self.oai_error();
        (
            code,
            Json(serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default()),
        )
            .into_response()
    }
}
