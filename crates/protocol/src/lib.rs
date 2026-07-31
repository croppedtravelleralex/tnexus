//! OpenAI-compatible request/response shapes for MVP text + image.

mod error_class;
mod image_contract;

pub use error_class::{classify_fault, ErrorClass};
pub use image_contract::{
    assert_json_matches_except, build_client_contextual_info, build_estuary_download_headers,
    build_image_prepare_body, build_image_prepare_body_opts, build_image_start_body,
    build_image_start_body_opts, build_image_start_body_with_refs,
    build_image_start_body_with_refs_opts, build_prepare_contextual_info,
    build_pure_http_image_contextual_info, picture_v2_prompt, validate_estuary_headers,
    validate_resource_put_headers, ContractOptions, ImageEditRequest, ImageRef,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionRequest {
    #[serde(default = "default_chat_model")]
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
}

fn default_chat_model() -> String {
    "gpt-4o-mini".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
}

impl ChatMessage {
    pub fn text(&self) -> String {
        match &self.content {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| {
                    v.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| v.as_str().map(|s| s.to_string()))
                })
                .collect::<Vec<_>>()
                .join("\n"),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageGenerationRequest {
    #[serde(default = "default_image_model")]
    pub model: String,
    pub prompt: String,
    #[serde(default = "default_n")]
    pub n: u32,
    #[serde(default = "default_size")]
    pub size: String,
    #[serde(default = "default_response_format")]
    pub response_format: String,
}

fn default_image_model() -> String {
    "gpt-image-2".into()
}
fn default_n() -> u32 {
    1
}
fn default_size() -> String {
    "1024x1024".into()
}
fn default_response_format() -> String {
    "b64_json".into()
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<String>,
}

pub fn openai_error(
    message: impl Into<String>,
    code: impl Into<String>,
    fault: Option<&str>,
) -> Value {
    json!({
        "error": {
            "message": message.into(),
            "type": "gateway_error",
            "code": code.into(),
            "fault": fault,
        }
    })
}

pub fn chat_completion_response(model: &str, content: &str) -> Value {
    let id = format!("chatcmpl-{}", Uuid::new_v4());
    json!({
        "id": id,
        "object": "chat.completion",
        "created": chrono_secs(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
    })
}

pub fn image_generation_response(b64: &str) -> Value {
    json!({
        "created": chrono_secs(),
        "data": [{ "b64_json": b64 }]
    })
}

pub fn image_generation_url_response(url: &str) -> Value {
    json!({
        "created": chrono_secs(),
        "data": [{ "url": url }]
    })
}

fn chrono_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build ChatGPT web conversation body (SPA-aligned text chat).
pub fn build_text_conversation_body(prompt: &str, model_slug: &str) -> Value {
    build_text_conversation_body_opts(prompt, model_slug, &ContractOptions::default())
}

pub fn build_text_conversation_body_opts(
    prompt: &str,
    model_slug: &str,
    opts: &ContractOptions,
) -> Value {
    let message_id = opts
        .fixed_message_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let seed = if opts.contextual_seed.is_empty() {
        opts.parent_message_id.clone()
    } else {
        opts.contextual_seed.clone()
    };
    json!({
        "action": "next",
        "messages": [{
            "id": message_id,
            "author": { "role": "user" },
            "content": { "content_type": "text", "parts": [prompt] },
            "metadata": {}
        }],
        "parent_message_id": opts.parent_message_id,
        "model": model_slug,
        "conversation_mode": { "kind": "primary_assistant" },
        "client_prepare_state": "none",
        "enable_message_followups": true,
        "supports_buffering": true,
        "supported_encodings": ["v1"],
        "system_hints": [],
        "timezone": opts.timezone,
        "timezone_offset_min": opts.timezone_offset_min,
        "paragen_cot_summary_display_override": "allow",
        "force_parallel_switch": "auto",
        "client_contextual_info": build_client_contextual_info(
            &seed,
            opts.contextual_jitter,
            "chatgpt.com",
        ),
        "history_and_training_disabled": true,
    })
}
