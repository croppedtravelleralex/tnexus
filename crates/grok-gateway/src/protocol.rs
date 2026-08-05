//! G5-P3 协议转换层：`/v1/responses`（OpenAI Responses）与 `/v1/messages`
//! （Anthropic Messages）→ grok 内部对话表示，以及响应组装。
//!
//! 纯函数，无 IO。归一化复用 `grok_conversation::normalize_chat_input`：
//! OpenAI Responses 的 `input` 消息 content part（`input_text` / `input_image` /
//! `input_file`）与 Anthropic 的 `text` / `image` 块在送入前映射为 conversation
//! 可识别的 part 形态。
//!
//! 对齐 Go `provider/web/chat.go` 的 `normalizeOpenAIInput(operation="responses")`
//! 与 `provider/conversation/request.go` 的 `convertMessagesRequest`（Anthropic
//! system 合并 / 缺省值），仅保留文本 + 图片主路径（G5 最小集）。

use serde::Deserialize;
use serde_json::{json, Value};

use grok_conversation::{normalize_chat_input, ChatMessage, NormalizedChatInput};

use crate::error::GatewayError;

/// OpenAI `POST /v1/responses` 请求（G5 子集：model / input / instructions / stream）。
#[derive(Debug, Deserialize)]
pub struct ResponsesRequest {
    /// 对外模型名。
    pub model: String,
    /// `input`：纯字符串，或 `{type:"message",role,content:[...]}` 消息数组
    /// （Responses API 与 chat 不同，用 `input` 而非 `messages`）。
    pub input: Value,
    /// 顶层指令 → 前缀 system 消息（Go `normalizeOpenAIInput` responses 分支）。
    #[serde(default)]
    pub instructions: Option<String>,
    /// true → SSE 流式。
    #[serde(default)]
    pub stream: bool,
}

/// Anthropic `POST /v1/messages` 请求（G5 子集：model / system / messages / max_tokens / stream）。
#[derive(Debug, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    /// system：纯字符串或 `[{type:"text",text:...}]` 块数组。
    #[serde(default)]
    pub system: Option<Value>,
    pub messages: Vec<AnthropicMessage>,
    /// 缺失时使用上游默认（G5 最小集不强制，仅校验 > 0 时合法）。
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub stream: bool,
}

/// Anthropic message：role + content（字符串或 text/image 块数组）。
#[derive(Debug, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Value,
}

fn system_message(text: &str) -> ChatMessage {
    ChatMessage {
        role: "system".into(),
        content: json!(text),
        type_name: String::new(),
    }
}

/// 归一化 OpenAI Responses `input`（对齐 Go `normalizeOpenAIInput(..., "responses")`）。
///
/// - `input` 为字符串 → 单条 user 消息。
/// - `input` 为数组 → 逐项取 `type=="message"`（role/content）；其它项类型（如
///   `function_call`、`computer_call`）G5 最小集明确拒绝。
/// - `instructions` 非空 → 前缀 system 消息。
pub fn normalize_responses_input(
    req: &ResponsesRequest,
) -> Result<NormalizedChatInput, GatewayError> {
    let mut messages = Vec::new();
    if let Some(instructions) = req.instructions.as_deref() {
        if !instructions.trim().is_empty() {
            messages.push(system_message(instructions));
        }
    }
    match req.input.as_str() {
        Some(text) => {
            messages.push(ChatMessage {
                role: "user".into(),
                content: json!(text),
                type_name: String::new(),
            });
        }
        None => {
            let items = req.input.as_array().ok_or_else(|| {
                GatewayError::InvalidRequest("input 必须是字符串或消息数组".into())
            })?;
            if items.is_empty() {
                return Err(GatewayError::InvalidRequest("input 不能为空".into()));
            }
            for item in items {
                let type_name = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if type_name != "message" {
                    return Err(GatewayError::InvalidRequest(format!(
                        "不支持的 input item 类型: {type_name}"
                    )));
                }
                let role = item
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .to_string();
                let content = item.get("content").cloned().unwrap_or(Value::Null);
                messages.push(ChatMessage {
                    role,
                    content,
                    type_name: String::new(),
                });
            }
        }
    }
    normalize_chat_input(messages).map_err(GatewayError::from)
}

/// 归一化 Anthropic Messages 请求（对齐 Go `convertMessagesRequest` 的 system 合并）。
///
/// - `system`（字符串或 text 块）→ 首条 system 消息。
/// - `messages` → text / image 块映射为 conversation part（image 的 base64 source
///   转 data URI）；`tool_use` / `tool_result` G5 最小集明确拒绝。
pub fn normalize_messages_input(
    req: &MessagesRequest,
) -> Result<NormalizedChatInput, GatewayError> {
    if let Some(max_tokens) = req.max_tokens {
        if max_tokens < 1 {
            return Err(GatewayError::InvalidRequest(
                "max_tokens 必须大于 0".into(),
            ));
        }
    }
    let mut messages = Vec::new();
    if let Some(system) = &req.system {
        let text = anthropic_system_text(system)?;
        if !text.trim().is_empty() {
            messages.push(system_message(&text));
        }
    }
    for m in &req.messages {
        let content = anthropic_content_to_compat(&m.content)?;
        messages.push(ChatMessage {
            role: m.role.clone(),
            content,
            type_name: String::new(),
        });
    }
    normalize_chat_input(messages).map_err(GatewayError::from)
}

/// system 字段 → 纯文本（字符串原样；块数组取 `type=text` 拼接）。
fn anthropic_system_text(value: &Value) -> Result<String, GatewayError> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    let blocks = value
        .as_array()
        .ok_or_else(|| GatewayError::InvalidRequest("system 必须是字符串或块数组".into()))?;
    let mut out = Vec::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                out.push(text.to_string());
            }
        }
    }
    Ok(out.join("\n"))
}

/// Anthropic content（字符串或块数组）→ conversation 兼容 content。
fn anthropic_content_to_compat(value: &Value) -> Result<Value, GatewayError> {
    if let Some(text) = value.as_str() {
        return Ok(json!(text));
    }
    let blocks = value
        .as_array()
        .ok_or_else(|| GatewayError::InvalidRequest("content 必须是字符串或块数组".into()))?;
    let mut parts = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text" => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or_default();
                parts.push(json!({"type": "text", "text": text}));
            }
            "image" => {
                let url = anthropic_image_data_uri(block)?;
                parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
            }
            other => {
                return Err(GatewayError::InvalidRequest(format!(
                    "不支持的 Anthropic content 块: {other}"
                )));
            }
        }
    }
    Ok(Value::Array(parts))
}

/// Anthropic image 块 `{type:image, source:{type:base64, media_type, data}}`
/// → data URI（G5 仅支持 base64 内联；URL source 留后续）。
fn anthropic_image_data_uri(block: &Value) -> Result<String, GatewayError> {
    let source = block
        .get("source")
        .ok_or_else(|| GatewayError::InvalidRequest("image 块缺少 source".into()))?;
    let source_type = source.get("type").and_then(Value::as_str).unwrap_or_default();
    if source_type != "base64" {
        return Err(GatewayError::InvalidRequest(format!(
            "不支持的 image source 类型: {source_type}"
        )));
    }
    let media_type = source
        .get("media_type")
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    let data = source
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::InvalidRequest("image source 缺少 data".into()))?;
    Ok(format!("data:{media_type};base64,{data}"))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// OpenAI Responses 非流式响应（`object: response`，output 数组）。
pub fn responses_json(id: &str, model: &str, text: &str) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created": now_secs(),
        "model": model,
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }],
        }],
        "usage": null,
    })
}

/// Anthropic Messages 非流式响应（content block 数组 + stop_reason）。
pub fn messages_json(id: &str, model: &str, text: &str) -> Value {
    json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{ "type": "text", "text": text }],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": null,
    })
}

/// OpenAI Responses SSE 帧（response.created → output_text.delta → response.completed）。
pub fn responses_stream_events(id: &str, model: &str, text: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "response.created",
            "response": { "id": id, "object": "response", "model": model, "status": "in_progress" },
        }),
        json!({
            "type": "response.output_text.delta",
            "item_id": id,
            "output_index": 0,
            "delta": text,
        }),
        json!({
            "type": "response.completed",
            "response": { "id": id, "object": "response", "model": model, "status": "completed" },
        }),
    ]
}

/// Anthropic Messages SSE 帧（message_start → content_block_* → message_delta → message_stop）。
pub fn messages_stream_events(id: &str, model: &str, text: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "message_start",
            "message": {
                "id": id, "type": "message", "role": "assistant", "model": model,
                "content": [], "stop_reason": null, "stop_sequence": null, "usage": null,
            },
        }),
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "text", "text": "" },
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": text },
        }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": null }),
        json!({ "type": "message_stop" }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_conversation::MAX_CHAT_IMAGE_ATTACHMENTS;

    const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn responses(input: Value) -> ResponsesRequest {
        ResponsesRequest {
            model: "grok-4.5".into(),
            input,
            instructions: None,
            stream: false,
        }
    }

    #[test]
    fn responses_input_text_and_image_roundtrip() {
        // 对齐 Go TestNormalizeResponsesInputImage
        let req = responses(json!([{
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_text", "text": "what is this"},
                {"type": "input_image", "image_url": DATA_URI},
            ],
        }]));
        let out = normalize_responses_input(&req).unwrap();
        assert_eq!(out.prompt, "[user]\nwhat is this");
        assert_eq!(out.images, vec![DATA_URI.to_string()]);
    }

    #[test]
    fn responses_input_string_becomes_user_message() {
        let out = normalize_responses_input(&responses(json!("hello"))).unwrap();
        assert_eq!(out.prompt, "[user]\nhello");
        assert!(out.images.is_empty());
    }

    #[test]
    fn responses_instructions_prepend_system() {
        let req = ResponsesRequest {
            input: json!("draw a cat"),
            instructions: Some("be concise".into()),
            ..responses(json!("draw a cat"))
        };
        let out = normalize_responses_input(&req).unwrap();
        assert_eq!(out.prompt, "[system]\nbe concise\n\n[user]\ndraw a cat");
    }

    #[test]
    fn responses_input_file_rejected_explicitly() {
        // 对齐 Go TestNormalizeResponsesInputFileFailsExplicitly
        let req = responses(json!([{
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_file", "file_url": "https://example.com/a.pdf"},
            ],
        }]));
        let err = normalize_responses_input(&req).unwrap_err();
        assert!(err.to_string().contains("input_file"));
    }

    #[test]
    fn responses_unsupported_item_type_rejected() {
        let req = responses(json!([{ "type": "function_call", "name": "f", "arguments": "{}" }]));
        let err = normalize_responses_input(&req).unwrap_err();
        assert!(err.to_string().contains("function_call"));
    }

    #[test]
    fn responses_input_must_be_string_or_array() {
        let err = normalize_responses_input(&responses(json!(42))).unwrap_err();
        assert!(err.to_string().contains("字符串或消息数组"));
    }

    #[test]
    fn responses_nine_images_rejected() {
        let parts: Vec<Value> = (0..9)
            .map(|_| json!({"type": "input_image", "image_url": DATA_URI}))
            .collect();
        let req = responses(json!([{ "type": "message", "role": "user", "content": parts }]));
        let err = normalize_responses_input(&req).unwrap_err();
        assert!(err.to_string().contains(&MAX_CHAT_IMAGE_ATTACHMENTS.to_string()));
    }

    #[test]
    fn messages_system_string_and_text_content() {
        let req = MessagesRequest {
            model: "grok-4.5".into(),
            system: Some(json!("你是一名助手")),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: json!("你好"),
            }],
            max_tokens: Some(100),
            stream: false,
        };
        let out = normalize_messages_input(&req).unwrap();
        assert_eq!(out.prompt, "[system]\n你是一名助手\n\n[user]\n你好");
        assert!(out.images.is_empty());
    }

    #[test]
    fn messages_system_block_array_concat() {
        let req = MessagesRequest {
            model: "grok-4.5".into(),
            system: Some(json!([{ "type": "text", "text": "第一段" }, { "type": "text", "text": "第二段" }])),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: json!("hi"),
            }],
            max_tokens: None,
            stream: false,
        };
        let out = normalize_messages_input(&req).unwrap();
        assert_eq!(out.prompt, "[system]\n第一段\n第二段\n\n[user]\nhi");
    }

    #[test]
    fn messages_image_source_to_data_uri() {
        let req = MessagesRequest {
            model: "grok-4.5".into(),
            system: None,
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: json!([{
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": "aW1n" },
                }]),
            }],
            max_tokens: None,
            stream: false,
        };
        let out = normalize_messages_input(&req).unwrap();
        assert_eq!(out.images, vec!["data:image/png;base64,aW1n".to_string()]);
    }

    #[test]
    fn messages_max_tokens_must_be_positive() {
        let req = MessagesRequest {
            model: "grok-4.5".into(),
            system: None,
            messages: vec![AnthropicMessage { role: "user".into(), content: json!("hi") }],
            max_tokens: Some(0),
            stream: false,
        };
        let err = normalize_messages_input(&req).unwrap_err();
        assert!(err.to_string().contains("max_tokens"));
    }

    #[test]
    fn response_shapes() {
        let r = responses_json("resp_1", "grok-4.5", "hello");
        assert_eq!(r["object"], "response");
        assert_eq!(r["output"][0]["content"][0]["type"], "output_text");
        assert_eq!(r["output"][0]["content"][0]["text"], "hello");

        let m = messages_json("msg_1", "grok-4.5", "hi");
        assert_eq!(m["type"], "message");
        assert_eq!(m["content"][0]["type"], "text");
        assert_eq!(m["content"][0]["text"], "hi");
        assert_eq!(m["stop_reason"], "end_turn");

        let s = responses_stream_events("r1", "grok-4.5", "t");
        assert_eq!(s[0]["type"], "response.created");
        assert_eq!(s[1]["type"], "response.output_text.delta");
        assert_eq!(s[1]["delta"], "t");
        assert_eq!(s[2]["type"], "response.completed");

        let ms = messages_stream_events("m1", "grok-4.5", "t");
        assert_eq!(ms[0]["type"], "message_start");
        assert_eq!(ms[2]["type"], "content_block_delta");
        assert_eq!(ms[2]["delta"]["text"], "t");
        assert_eq!(ms[5]["type"], "message_stop");
    }
}
