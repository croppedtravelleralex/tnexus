//! 请求构造（G5-A3：model / messages / stream 字段，对齐 Go console 对话请求形态）。

use serde_json::{json, Value};

use crate::error::ProviderError;

/// 构造 chat completions 流式请求体。
///
/// - `model`：上游模型名（必填非空）
/// - `messages`：OpenAI 消息数组（非数组 → InvalidRequest）
/// - `stream`：恒按调用方意图写入（适配器流式路径传 true）
pub fn build_chat_request(
    model: &str,
    messages: &Value,
    stream: bool,
) -> Result<Value, ProviderError> {
    let model = model.trim();
    if model.is_empty() {
        return Err(ProviderError::InvalidRequest("model 不能为空".into()));
    }
    if !messages.is_array() {
        return Err(ProviderError::InvalidRequest("messages 必须是数组".into()));
    }
    Ok(json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_chat_request_shape() {
        let body = build_chat_request(
            "grok-4.3",
            &json!([{"role": "user", "content": "hello"}]),
            true,
        )
        .unwrap();
        assert_eq!(body["model"], "grok-4.3");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn rejects_missing_model_or_non_array_messages() {
        assert!(build_chat_request("", &json!([]), true).is_err());
        assert!(build_chat_request("grok-4.3", &json!({"role": "user"}), true).is_err());
    }
}
