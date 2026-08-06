//! Stored response 解析（对齐 Go Responses `output` 结构，G5-P1 非流式路径）。

use serde::Deserialize;

use crate::error::ProviderError;

/// 上游 `/responses` 存储响应（`stream:false` 单次往返）。
#[derive(Debug, Clone, Deserialize)]
pub struct StoredResponse {
    pub id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub output: Vec<Output>,
}

/// 输出项（未知类型经 `#[serde(other)]` 容忍）。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    Message {
        #[serde(default)]
        content: Vec<Content>,
    },
    Reasoning {
        #[serde(default)]
        summary: Vec<serde_json::Value>,
    },
    #[serde(other)]
    Other,
}

/// 消息内容项。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    OutputText {
        text: String,
    },
    InputText {
        text: String,
    },
    #[serde(other)]
    Other,
}

impl StoredResponse {
    /// 从 JSON 解析；上游响应必须含 `id`。
    pub fn from_json(value: &serde_json::Value) -> Result<Self, ProviderError> {
        serde_json::from_value(value.clone())
            .map_err(|e| ProviderError::Upstream(format!("解析 Responses 响应: {e}")))
    }

    /// 拼接全部 `output_text`（消息内容的文本输出）。
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        for item in &self.output {
            if let Output::Message { content } = item {
                for part in content {
                    if let Content::OutputText { text } = part {
                        parts.push(text.as_str());
                    }
                }
            }
        }
        parts.join("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_stored_response_and_extracts_text() {
        let value = json!({
            "id": "resp_1",
            "model": "grok-4.5",
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "thought"}]},
                {"type": "message", "content": [{"type": "output_text", "text": "hello "}, {"type": "output_text", "text": "world"}]},
                {"type": "function_call", "call_id": "call_1", "name": "f", "arguments": "{}"}
            ]
        });
        let response = StoredResponse::from_json(&value).unwrap();
        assert_eq!(response.id, "resp_1");
        assert_eq!(response.model.as_deref(), Some("grok-4.5"));
        assert_eq!(response.text(), "hello world");
    }

    #[test]
    fn empty_output_text_is_empty() {
        let value = json!({"id": "resp_0", "output": []});
        let response = StoredResponse::from_json(&value).unwrap();
        assert_eq!(response.text(), "");
    }
}
