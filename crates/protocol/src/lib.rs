//! OpenAI-compatible request/response shapes for MVP text + image.

mod error_class;
mod image_contract;

pub use error_class::{
    classify_fault, default_rate_limit_wait_secs, openai_error_type_for_class, ErrorClass,
};
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
    /// When true, treat the last user message as an image generation prompt (plain text).
    #[serde(default)]
    pub image_mode: bool,
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

/// Fold OpenAI-style messages into one upstream text prompt (multi-turn context).
pub fn fold_chat_messages_for_upstream(messages: &[ChatMessage]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    if messages.len() == 1 {
        return messages[0].text();
    }
    let mut lines = Vec::new();
    for m in messages {
        let full = m.text();
        let text = full.trim();
        if text.is_empty() {
            continue;
        }
        let label = match m.role.as_str() {
            "assistant" => "Assistant",
            "system" => "System",
            _ => "User",
        };
        lines.push(format!("{label}: {text}"));
    }
    lines.join("\n\n")
}

/// User explicitly requested inline image generation in chat.
pub fn chat_message_requests_image(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("@create image") || t.starts_with("@Create image") {
        return true;
    }
    if lower.starts_with("/image") || lower.starts_with("/img") {
        return true;
    }
    // Natural-language image intents (Chinese)
    const KEYWORDS: [&str; 11] = [
        "画一张",
        "画一幅",
        "画个",
        "帮我画",
        "生成图片",
        "生成一张图",
        "生成图像",
        "生图",
        "绘制",
        "出一张图",
        "画出来",
    ];
    KEYWORDS.iter().any(|k| t.contains(k))
}

/// Obvious text / Q&A intents — skip image path when `image_mode` is default-on.
pub fn chat_message_prefers_text(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    if t.contains('?') || t.contains('？') {
        return true;
    }
    const TEXT_HINTS: [&str; 14] = [
        "什么",
        "怎么",
        "如何",
        "为什么",
        "为何",
        "是不是",
        "能不能",
        "可以吗",
        "介绍",
        "解释",
        "翻译",
        "代码",
        "帮我写",
        "写一个",
    ];
    if TEXT_HINTS.iter().any(|k| t.contains(k)) {
        return true;
    }
    let lower = t.to_lowercase();
    if lower == "hi" || lower == "hello" || t == "你好" || t == "嗨" {
        return true;
    }
    if lower.starts_with("你好") || lower.starts_with("hello") || lower.starts_with("hi ") {
        return true;
    }
    false
}

/// Model slugs that can only produce images (`gpt-image-2`, `dall-e-3`, …).
pub fn model_is_image_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.contains("image") || m.contains("dall-e") || m.contains("dalle")
}

/// Whether chat should route to inline image generation.
///
/// `model` is authoritative: an image-only slug must never fall through to the
/// text path. Keyword/`image_mode` heuristics alone let `gpt-image-2` requests
/// reach `fold_chat_messages_for_upstream`, which folds the whole history into
/// one message and trips upstream's 413 `message_length_exceeds_limit`.
pub fn chat_should_use_image_path(model: &str, text: &str, image_mode: bool) -> bool {
    if model_is_image_model(model) {
        return true;
    }
    if chat_message_requests_image(text) {
        return true;
    }
    image_mode && !chat_message_prefers_text(text)
}

/// Extract image prompt from chat text (`@Create image`, `/image …`, or plain prompt).
pub fn extract_chat_image_prompt(text: &str) -> String {
    let t = text.trim();
    let lower = t.to_lowercase();
    if lower.starts_with("/image") {
        let rest = t.chars().skip_while(|c| *c == '/').collect::<String>();
        let rest = rest
            .trim_start_matches("image")
            .trim_start_matches("IMAGE")
            .trim_start();
        return if rest.is_empty() {
            t.to_string()
        } else {
            rest.to_string()
        };
    }
    if lower.starts_with("/img") {
        let rest = t
            .chars()
            .skip(4)
            .collect::<String>()
            .trim_start()
            .to_string();
        return if rest.is_empty() { t.to_string() } else { rest };
    }
    if let Some(rest) = t
        .strip_prefix("/image ")
        .or_else(|| t.strip_prefix("/img "))
        .or_else(|| t.strip_prefix("/IMAGE "))
    {
        return rest.trim().to_string();
    }
    if lower.starts_with("@create image") {
        let rest = t
            .chars()
            .skip("@create image".len())
            .collect::<String>()
            .trim_start_matches([' ', '\u{00a0}'])
            .trim()
            .to_string();
        return if rest.is_empty() {
            "@Create image".to_string()
        } else {
            rest
        };
    }
    if t.starts_with("@Create image") {
        let rest = t
            .chars()
            .skip("@Create image".len())
            .collect::<String>()
            .trim_start_matches([' ', '\u{00a0}'])
            .trim()
            .to_string();
        return if rest.is_empty() {
            "@Create image".to_string()
        } else {
            rest
        };
    }
    // Strip common Chinese command prefixes for natural-language image requests
    for prefix in [
        "画一张",
        "画一幅",
        "画个",
        "帮我画",
        "生成图片",
        "生成一张图",
        "生成图像",
        "生图",
        "绘制",
        "出一张图",
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let cleaned = rest.trim_start_matches(['：', ':', '，', ',', ' ']);
            if !cleaned.is_empty() {
                return cleaned.to_string();
            }
        }
    }
    t.to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageGenerationRequest {
    #[serde(default = "default_image_model")]
    pub model: String,
    pub prompt: String,
    #[serde(default = "default_n")]
    pub n: u32,
    #[serde(default = "default_size")]
    pub size: String,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub asset_ids: Vec<String>,
    #[serde(default = "default_response_format")]
    pub response_format: String,
    /// gptimage async tunnel: enqueue and return task id immediately.
    #[serde(default)]
    pub panda_async: bool,
    /// Poll an existing async image task (gptimage tunnel).
    #[serde(default)]
    pub panda_task_id: Option<String>,
}

/// Parsed gptimage prompt tunnel prefixes.
#[derive(Debug, Clone)]
pub enum ImagePromptTunnel {
    Normal(String),
    AsyncGenerate(String),
    StatusPoll(String),
}

pub fn parse_image_prompt_tunnel(prompt: &str) -> ImagePromptTunnel {
    let trimmed = prompt.trim();
    if let Some(rest) = trimmed.strip_prefix("panda-status ") {
        return ImagePromptTunnel::StatusPoll(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("panda-async:") {
        return ImagePromptTunnel::AsyncGenerate(rest.trim().to_string());
    }
    ImagePromptTunnel::Normal(trimmed.to_string())
}

pub fn image_task_queued_response(task_id: &str) -> Value {
    json!({
        "id": task_id,
        "object": "image.task",
        "status": "queued",
        "created": chrono_secs(),
    })
}

pub fn image_task_status_response(
    task_id: &str,
    status: &str,
    result: Option<Value>,
    error: Option<&str>,
) -> Value {
    let mut body = json!({
        "id": task_id,
        "object": "image.task",
        "status": status,
        "created": chrono_secs(),
    });
    if let Some(obj) = body.as_object_mut() {
        if let Some(r) = result {
            obj.insert("result".into(), r);
        }
        if let Some(e) = error {
            obj.insert("error".into(), json!(e));
        }
    }
    body
}

impl ImageGenerationRequest {
    pub fn transparent_bg(&self) -> bool {
        self.background
            .as_deref()
            .map(|b| b.eq_ignore_ascii_case("transparent"))
            .unwrap_or(false)
    }
}

fn default_image_model() -> String {
    "gpt-image-2".into()
}
/// OpenAI-compatible max batch size for `n` on image endpoints.
pub const MAX_IMAGE_BATCH_N: u32 = 4;

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
    let message = message.into();
    let code = code.into();
    let class = classify_fault(fault, Some(&message));
    let error_type = openai_error_type_for_class(class, &code);
    let mut error = json!({
        "message": message,
        "type": error_type,
        "code": code,
        "fault": fault,
    });
    if let Some(wait) = default_rate_limit_wait_secs(class) {
        error["estimated_wait_secs"] = json!(wait);
    }
    json!({ "error": error })
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

pub fn chat_completion_response_with_image_b64(model: &str, b64: &str) -> Value {
    let id = format!("chatcmpl-{}", Uuid::new_v4());
    json!({
        "id": id,
        "object": "chat.completion",
        "created": chrono_secs(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tnexus_image_b64": b64,
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
    })
}

pub fn estimate_image_input_tokens(prompt: &str) -> i64 {
    let chars = prompt.chars().count();
    // Rough OpenAI-style estimate; varies with short/medium/long test prompts.
    ((chars as f64 / 3.5).ceil() as i64).clamp(2, 8192)
}

fn image_generation_usage_value(prompt: &str) -> Value {
    let text_tokens = estimate_image_input_tokens(prompt);
    let output_tokens = 1650_i64;
    json!({
        "input_tokens": text_tokens,
        "output_tokens": output_tokens,
        "total_tokens": text_tokens + output_tokens,
        "input_tokens_details": {
            "text_tokens": text_tokens,
            "image_tokens": 0,
            "cached_tokens": 0
        },
        "output_tokens_details": {
            "text_tokens": 0,
            "image_tokens": output_tokens,
            "reasoning_tokens": 0
        }
    })
}

pub fn image_generation_response(b64: &str, prompt: &str) -> Value {
    json!({
        "created": chrono_secs(),
        "data": [{ "b64_json": b64 }],
        "usage": image_generation_usage_value(prompt),
    })
}

pub fn image_generation_url_response(url: &str, prompt: &str) -> Value {
    json!({
        "created": chrono_secs(),
        "data": [{ "url": url }],
        "usage": image_generation_usage_value(prompt),
    })
}

/// OpenAI-compatible image response with TNexus pipeline telemetry extension.
pub fn image_generation_response_with_pipeline(
    data: Value,
    pipeline: Value,
    prompt: &str,
) -> Value {
    json!({
        "created": chrono_secs(),
        "data": data,
        "usage": image_generation_usage_value(prompt),
        "_tnexus_pipeline": pipeline,
    })
}

pub fn image_generation_url_response_with_pipeline(
    url: &str,
    pipeline: Value,
    prompt: &str,
) -> Value {
    image_generation_response_with_pipeline(json!([{ "url": url }]), pipeline, prompt)
}

pub fn image_generation_b64_response_with_pipeline(
    b64: &str,
    pipeline: Value,
    prompt: &str,
) -> Value {
    image_generation_response_with_pipeline(json!([{ "b64_json": b64 }]), pipeline, prompt)
}

pub fn image_generation_b64_multi_response(b64s: &[String], prompt: &str) -> Value {
    let data: Vec<Value> = b64s.iter().map(|b64| json!({ "b64_json": b64 })).collect();
    json!({
        "created": chrono_secs(),
        "data": data,
        "usage": image_generation_usage_value(prompt),
    })
}

pub fn image_generation_url_multi_response(urls: &[String], prompt: &str) -> Value {
    let data: Vec<Value> = urls.iter().map(|url| json!({ "url": url })).collect();
    json!({
        "created": chrono_secs(),
        "data": data,
        "usage": image_generation_usage_value(prompt),
    })
}

pub fn image_generation_b64_multi_response_with_pipeline(
    b64s: &[String],
    pipeline: Value,
    prompt: &str,
) -> Value {
    let data: Vec<Value> = b64s.iter().map(|b64| json!({ "b64_json": b64 })).collect();
    image_generation_response_with_pipeline(json!(data), pipeline, prompt)
}

pub fn image_generation_url_multi_response_with_pipeline(
    urls: &[String],
    pipeline: Value,
    prompt: &str,
) -> Value {
    let data: Vec<Value> = urls.iter().map(|url| json!({ "url": url })).collect();
    image_generation_response_with_pipeline(json!(data), pipeline, prompt)
}

pub fn chrono_secs() -> u64 {
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
