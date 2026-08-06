//! SSE 流解析与 OpenAI 兼容分片规整（G5-A3）。
//!
//! [`SseParser`]：增量逐行解析 `data:` 事件（可跨 TCP 分片累积，`feed` 幂等）；
//! [`parse_chat_delta`]：把单条 `data:` 负载规整为 OpenAI 兼容分片
//! （`choices[0].delta.content` 直通；上游 responses 风格
//! `{"type":"response.output_text.delta","delta":"..."}` 映射到 content）。

use crate::error::ProviderError;

/// 单个 SSE 事件（仅保留 `data:` 负载；`[DONE]` 亦为一条事件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub data: String,
}

/// 增量 SSE 解析器：累积跨块字节，产出完整事件。
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一块原始字节，返回累积出的完整事件。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.find('\n') {
            let line: String = self.buffer[..newline].to_string();
            self.buffer.drain(..=newline);
            if let Some(event) = parse_line(&line) {
                events.push(event);
            }
        }
        events
    }

    /// 流结束：把剩余未换行的尾行作为一条事件产出。
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let tail = std::mem::take(&mut self.buffer);
        let tail = tail.trim_end_matches('\r');
        if tail.is_empty() {
            return Vec::new();
        }
        parse_line(tail).into_iter().collect()
    }
}

/// 解析一行 SSE 文本；非 `data:` 行（注释 / event / 空行）返回 None。
fn parse_line(line: &str) -> Option<SseEvent> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.trim_start()
        .strip_prefix("data:")
        .map(|data| SseEvent {
            data: data.trim_start().to_string(),
        })
}

/// 归一化的 OpenAI 兼容聊天分片（`choices[0].delta` 子集）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatDelta {
    pub index: usize,
    pub role: Option<String>,
    pub content: Option<String>,
    pub finish_reason: Option<String>,
}

/// 把单条 SSE `data:` 负载规整为分片。
///
/// - `[DONE]` / 无内容负载（usage、keep-alive 等）→ `Ok(None)`（跳过）
/// - chat 风格 `choices[0].delta.{role,content}` + `choices[0].finish_reason`
/// - responses 风格 `{"type":"response.output_text.delta","delta":"..."}` → content
/// - 非法 JSON 或非对象 → [`ProviderError::InvalidRequest`]
pub fn parse_chat_delta(data: &str) -> Result<Option<ChatDelta>, ProviderError> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| ProviderError::InvalidRequest(format!("解析 SSE data 负载: {e}")))?;
    let Some(payload) = value.as_object() else {
        return Err(ProviderError::InvalidRequest(
            "SSE data 负载必须是 JSON 对象".into(),
        ));
    };

    // responses 风格：response.output_text.delta
    if let Some(type_name) = payload.get("type").and_then(|v| v.as_str()) {
        if type_name == "response.output_text.delta" {
            let delta = payload
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            return Ok(Some(ChatDelta {
                content: Some(delta.to_string()),
                ..Default::default()
            }));
        }
        // response.completed 等无文本事件 → 跳过
        if type_name.starts_with("response.") {
            return Ok(None);
        }
    }

    // chat 风格：choices[0].delta / finish_reason
    let Some(choice) = payload
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.as_object())
    else {
        return Ok(None);
    };
    let index = choice.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let delta = choice.get("delta").and_then(|d| d.as_object());
    let role = delta
        .and_then(|d| d.get("role"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let content = delta
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if role.is_none() && content.is_none() && finish_reason.is_none() {
        return Ok(None);
    }
    Ok(Some(ChatDelta {
        index,
        role,
        content,
        finish_reason,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_accumulates_across_chunks() {
        let mut parser = SseParser::new();
        // 第一块被截断在行中间
        let first = parser.feed(b"data: {\"choices\":[{\"delta\":{\"con");
        assert!(first.is_empty(), "partial line held back");
        let second = parser.feed(b"tent\":\"Hel\"}}]}\n\ndata: [DONE]\n");
        assert_eq!(second.len(), 2, "both events after line completes");
        assert!(second[0].data.contains("\"content\":\"Hel\""));
        assert_eq!(second[1].data, "[DONE]");
        assert_eq!(parser.finish(), Vec::new(), "no trailing partial");
    }

    #[test]
    fn sse_parser_finish_emits_trailing_line() {
        let mut parser = SseParser::new();
        assert!(parser.feed(b"data: {\"id\":1}").is_empty());
        let tail = parser.finish();
        assert_eq!(tail.len(), 1);
        assert!(tail[0].data.contains("\"id\":1"));
    }

    #[test]
    fn parses_chat_delta_content_role_and_finish_reason() {
        let delta = parse_chat_delta(
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"},"finish_reason":null}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(delta.index, 0);
        assert_eq!(delta.role.as_deref(), Some("assistant"));
        assert_eq!(delta.content.as_deref(), Some("Hel"));
        assert_eq!(delta.finish_reason, None);

        let delta =
            parse_chat_delta(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#)
                .unwrap()
                .unwrap();
        assert_eq!(delta.content, None);
        assert_eq!(delta.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn maps_responses_style_delta() {
        let delta = parse_chat_delta(r#"{"type":"response.output_text.delta","delta":"Hello"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(delta.content.as_deref(), Some("Hello"));
        assert_eq!(delta.role, None);

        // response.completed / usage 等无文本事件 → 跳过
        assert!(parse_chat_delta(r#"{"type":"response.completed"}"#)
            .unwrap()
            .is_none());
    }

    #[test]
    fn skips_terminator_and_empty() {
        assert!(parse_chat_delta("[DONE]").unwrap().is_none());
        assert!(parse_chat_delta("").unwrap().is_none());
        assert!(parse_chat_delta("   ").unwrap().is_none());
        // 无内容负载（usage 等）→ 跳过
        assert!(parse_chat_delta(r#"{"usage":{"total_tokens":5}}"#)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(
            parse_chat_delta("not json"),
            Err(ProviderError::InvalidRequest(_))
        ));
    }
}
