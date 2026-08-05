//! grok-provider-console 错误模型（对齐 grok-provider-build::error 层级约定 + Go console 错误规整）。

use thiserror::Error;

/// Provider Console 错误。
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

    /// 请求超时（对齐 Go context deadline + egress lease 超时语义）。
    #[error("request timed out after {0:?}")]
    Timeout(std::time::Duration),
}

/// 上游错误信封（对齐 Go `normalizeConversationError` 的解析产物）。
#[derive(Debug, Clone)]
pub struct UpstreamError {
    pub status: u16,
    /// 兼容 OpenAI 的 error.type（rate_limit_error / authentication_error 等）。
    pub error_type: String,
    pub message: String,
    /// 解析到的 Retry-After 秒数（无则 0；对齐 Go `consoleRetryAfter`）。
    pub retry_after_secs: i64,
}

impl UpstreamError {
    /// 从非 2xx 响应体解析错误（对齐 Go `normalizeConversationError` + `consoleRetryAfter`）。
    ///
    /// 优先级：`error.type` / `error.message` → 顶层 `message` → 原始文本（截断 4096）。
    /// error.type 缺失时按状态码推断（Go `conversationErrorType`）。
    pub fn parse(status: u16, body: &str) -> Self {
        let trimmed = body.trim();
        let (mut error_type, mut message) = (String::new(), String::new());
        if !trimmed.is_empty() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                let error = value.get("error");
                error_type = error
                    .and_then(|e| e.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                message = error
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if message.is_empty() {
                    message = value
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }
        }
        if error_type.is_empty() {
            error_type = conversation_error_type(status);
        }
        if message.is_empty() {
            message = trimmed.to_string();
        }
        if message.is_empty() {
            message = http_status_text(status).to_string();
        }
        if message.len() > 4096 {
            message.truncate(4096);
        }
        let retry_after_secs = console_retry_after(trimmed).as_secs() as i64;
        Self {
            status,
            error_type,
            message,
            retry_after_secs,
        }
    }
}

/// 按状态码推断兼容 error.type（对齐 Go `conversationErrorType`）。
pub fn conversation_error_type(status: u16) -> String {
    let value = match status {
        400 | 422 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        503 => "overloaded_error",
        _ => "server_error",
    };
    value.to_string()
}

/// 解析 "Resets in: 1h 2m 3s" 复合时长（对齐 Go `consoleRetryAfter`）。
pub fn console_retry_after(text: &str) -> std::time::Duration {
    let lower = text.to_ascii_lowercase();
    let Some(index) = lower.find("resets in:") else {
        return std::time::Duration::ZERO;
    };
    let rest = &lower[index + "resets in:".len()..];
    let mut total = 0u64;
    let mut chars = rest.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut number = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    number.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            let unit = chars.next().unwrap_or(' ');
            let value: u64 = number.parse().unwrap_or(0);
            total += match unit {
                'd' => value * 24 * 3600,
                'h' => value * 3600,
                'm' => value * 60,
                's' => value,
                _ => 0,
            };
        } else {
            chars.next();
        }
    }
    std::time::Duration::from_secs(total)
}

fn http_status_text(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Upstream Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_parses_compound_duration() {
        // Go TestConsoleRetryAfterParsesCompoundDuration
        assert_eq!(
            console_retry_after("Rate limit reached. Resets in: 1h 2m 3s"),
            std::time::Duration::from_secs(3723)
        );
        assert_eq!(console_retry_after("ordinary error"), std::time::Duration::ZERO);
    }

    #[test]
    fn parses_error_envelope_and_falls_back_to_status() {
        let err = UpstreamError::parse(
            429,
            r#"{"error":{"type":"rate_limit_error","message":"Rate limit reached"}}"#,
        );
        assert_eq!(err.status, 429);
        assert_eq!(err.error_type, "rate_limit_error");
        assert_eq!(err.message, "Rate limit reached");

        // 无 error.type → 按状态推断
        let err = UpstreamError::parse(403, r#"{"message":"denied"}"#);
        assert_eq!(err.error_type, "permission_error");
        assert_eq!(err.message, "denied");

        // 非 JSON → 原始文本
        let err = UpstreamError::parse(502, "plain text");
        assert_eq!(err.error_type, "server_error");
        assert_eq!(err.message, "plain text");
    }

    #[test]
    fn parses_retry_after_from_body() {
        let err = UpstreamError::parse(429, "Rate limit reached. Resets in: 1h 2m 3s");
        assert_eq!(err.retry_after_secs, 3723);
    }
}
