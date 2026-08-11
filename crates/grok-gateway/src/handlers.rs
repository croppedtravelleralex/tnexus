//! grok-gateway HTTP handlers（docs/39d §2.1 推理端点，docs/39 主文档 §6.1 P0/P1）。
//!
//! G1 端点：`GET /v1/models`、`POST /v1/chat/completions`（含识图 OCR）。
//!
//! OCR 判定（39 主文档 §4.2 + §4.1 治理）：
//! 请求 `model == grok-vision-ocr` **或** 消息含 image_url → 走 OCR 路径
//! （`grok-chat-fast` + `enableImageGeneration=false`）。这符合 §4.1「普通带图
//! 请求 enableImageGeneration=true 可能触发上游生图副作用，移植时需显式治理」——
//! 在 gateway 边界把带图请求统一送 OCR/识图路径（禁生图），避免意外生图。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream;
use serde::Deserialize;
use serde_json::{json, Value};

use grok_conversation::{normalize_chat_input, ChatMessage, NormalizedChatInput};
use grok_domain::{
    public_models, ChatBackend, ChatRequest as ProviderChatRequest, ImagineRequest, ALIAS_OCR,
};

use crate::error::GatewayError;
use crate::protocol::{
    messages_json, messages_stream_events, normalize_messages_input, normalize_responses_input,
    responses_json, responses_stream_events, MessagesRequest, ResponsesRequest,
};
use crate::router::AppState;

/// `POST /v1/chat/completions` 请求（G1 子集：model / messages / stream）。
#[derive(Debug, Deserialize)]
pub struct ChatCompletionsRequest {
    /// 对外模型名，可为 `grok-vision-ocr`（OCR 别名）或常规模型。
    pub model: String,
    /// 消息列表（含多模态 content）。
    pub messages: Vec<ChatMessage>,
    /// true → SSE 流式返回。
    #[serde(default)]
    pub stream: bool,
}

/// `POST /v1/images/generations` 请求（G2：prompt / n / response_format / size / quality）。
#[derive(Debug, Deserialize)]
pub struct ImageGenerationRequest {
    /// 生图提示词。
    pub prompt: String,
    /// 生图数量（默认 1，上限 10）。
    #[serde(default = "default_n")]
    pub n: usize,
    /// 输出格式：`url`（默认）或 `b64_json`。
    #[serde(default)]
    pub response_format: String,
    /// 尺寸（本阶段忽略具体尺寸，交给上游）。
    #[serde(default)]
    pub size: String,
    /// 质量（本阶段忽略）。
    #[serde(default)]
    pub quality: String,
}

fn default_n() -> usize {
    1
}

/// OpenAI `size`（如 `1792x1024`）或 `16:9` → grok `aspect_ratio`。
pub fn size_to_aspect_ratio(size: &str) -> String {
    let size = size.trim();
    if size.is_empty() || size == "1024x1024" || size.eq_ignore_ascii_case("1:1") {
        return "1:1".to_string();
    }
    if size.contains(':') && !size.contains('x') {
        return size.to_string();
    }
    if let Some((w, h)) = size.split_once('x') {
        if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
            if w > 0 && h > 0 {
                return simplify_ratio(w, h);
            }
        }
    }
    "1:1".to_string()
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

fn simplify_ratio(w: u32, h: u32) -> String {
    let g = gcd(w, h).max(1);
    format!("{}:{}", w / g, h / g)
}

/// `POST /v1/images/generations`（G2）。走 `ImageEngine.imagine`。
pub async fn image_generations(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImageGenerationRequest>,
) -> Result<Response, GatewayError> {
    if req.n == 0 || req.n > 10 {
        return Err(GatewayError::InvalidRequest(format!(
            "n must be in 1..=10, got {}",
            req.n
        )));
    }
    let image_engine = state
        .image_engine
        .as_ref()
        .ok_or_else(|| GatewayError::Internal("ImageEngine not configured".into()))?;

    let out_format = if req.response_format == "b64_json" {
        "b64_json".to_string()
    } else {
        "url".to_string()
    };
    let aspect_ratio = size_to_aspect_ratio(&req.size);
    let imagine_req = ImagineRequest {
        prompt: req.prompt.clone(),
        n: req.n,
        response_format: out_format.clone(),
        lite: false,
        enhance: false,
        request_id: new_request_id(),
        aspect_ratio,
    };
    let result = image_engine.imagine(&imagine_req).await?;

    let data: Vec<Value> = result
        .items
        .iter()
        .map(|item| {
            if result.b64 {
                json!({ "b64_json": item })
            } else {
                json!({ "url": item })
            }
        })
        .collect();

    Ok(Json(json!({
        "object": "list",
        "created": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "data": data,
    }))
    .into_response())
}

/// `GET /v1/media/images/{id}`（G2-A4）。Grok 生图返回的是上游 URL，
/// 这里通过 id 查媒体字节并回传（Content-Type 嗅探）。未配置 `media_fetcher`
/// 或 id 未命中 → 404。
pub async fn media_images(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.media_fetcher.as_ref() {
        Some(fetcher) => match fetcher.fetch_bytes(&id).await {
            Ok((bytes, content_type)) => {
                ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
            }
            Err(_) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"message": "media not found"}})),
            )
                .into_response(),
        },
        None => (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": {"message": "media fetcher not configured"}})),
        )
            .into_response(),
    }
}

use axum::http::StatusCode;

/// 媒体取回抽象（G2-A4 `GET /v1/media/images/{id}`）。按 id 返回字节与 Content-Type。
#[async_trait::async_trait]
pub trait MediaFetcher: Send + Sync {
    async fn fetch_bytes(&self, id: &str) -> Result<(Vec<u8>, String), GatewayError>;
}

/// G5-P3 协议后端抽象：`/v1/responses` 与 `/v1/messages` 的上游推理面。
///
/// 真实实现接 grok-provider-build / grok-provider-console（TODO(G5-P3): 接线
/// grok-provider-build 的 stored response / console 流式到 `complete`），
/// 测试注入 fake。`normalized` 已含 system 前缀（`[system]\n...`）与图片清单。
#[async_trait::async_trait]
pub trait ProtocolBackend: Send + Sync {
    /// 执行一次对话推理，返回最终文本。
    async fn complete(
        &self,
        model: &str,
        normalized: &NormalizedChatInput,
    ) -> Result<String, GatewayError>;
}

/// `POST /v1/responses`（OpenAI Responses，G5-P3）。
///
/// `stream=false` → 单次 stored response（`object: response`）；`stream=true` →
/// SSE（response.created → output_text.delta → response.completed）。
pub async fn responses_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ResponsesRequest>,
) -> Result<Response, GatewayError> {
    let normalized = normalize_responses_input(&req)?;
    let backend = state
        .responses_backend
        .as_ref()
        .ok_or(GatewayError::NotConfigured)?;
    let text = backend.complete(&req.model, &normalized).await?;
    let request_id = new_request_id();
    if req.stream {
        let events = responses_stream_events(&request_id, &req.model, &text);
        Ok(protocol_sse(events).into_response())
    } else {
        Ok(Json(responses_json(&request_id, &req.model, &text)).into_response())
    }
}

/// `POST /v1/messages`（Anthropic Messages，G5-P3）。
///
/// `stream=false` → content block 响应；`stream=true` → SSE
/// （message_start → content_block_* → message_delta → message_stop）。
pub async fn messages_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MessagesRequest>,
) -> Result<Response, GatewayError> {
    let normalized = normalize_messages_input(&req)?;
    let backend = state
        .messages_backend
        .as_ref()
        .ok_or(GatewayError::NotConfigured)?;
    let text = backend.complete(&req.model, &normalized).await?;
    let request_id = new_request_id();
    if req.stream {
        let events = messages_stream_events(&request_id, &req.model, &text);
        Ok(protocol_sse(events).into_response())
    } else {
        Ok(Json(messages_json(&request_id, &req.model, &text)).into_response())
    }
}

/// 协议 SSE：事件数组 → 逐帧 `data: {...}`（无 `[DONE]`，协议各自终止事件收尾）。
fn protocol_sse(
    events: Vec<serde_json::Value>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let s = stream::iter(
        events
            .into_iter()
            .map(|event| Ok(Event::default().data(event.to_string()))),
    );
    Sse::new(s)
}

/// `GET /v1/models` 返回的模型项。
#[derive(Debug, Clone, serde::Serialize)]
struct ModelEntry {
    id: String,
    object: &'static str,
    created: i64,
    owned_by: &'static str,
}

/// 生成唯一请求 ID（审计 / SSE id）。本阶段用时间戳 + 计数器，足够区分。
fn new_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    format!(
        "chatcmpl-{stamp}-{:06}",
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// `GET /v1/models`：对外模型路由（含 OCR 别名）。
pub async fn models(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let data: Vec<ModelEntry> = public_models()
        .into_iter()
        .map(|(alias, _upstream)| ModelEntry {
            id: alias.to_string(),
            object: "model",
            created: now,
            owned_by: "grok",
        })
        .collect();
    let _ = &state; // G1 无鉴权；保留抽取便于后续扩展。
    Json(json!({ "object": "list", "data": data })).into_response()
}

/// OCR 判定：别名或带图请求。
fn is_ocr_request(model: &str, normalized_images: &[String]) -> bool {
    model == ALIAS_OCR || !normalized_images.is_empty()
}

/// `POST /v1/chat/completions`（含 OCR）。stream=true 走 SSE。
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatCompletionsRequest>,
) -> Result<axum::response::Response, GatewayError> {
    // 1) 协议归一化（图片数/大小/file_id 校验，conversation 层）。
    let normalized = normalize_chat_input(req.messages).map_err(GatewayError::from)?;

    // 2) OCR 判定 + 组装引擎请求。
    let ocr = is_ocr_request(&req.model, &normalized.images);
    let provider_req = ProviderChatRequest {
        prompt: normalized.prompt,
        images: normalized.images,
        ocr,
        system_prompt: None,
        request_id: new_request_id(),
    };

    let engine: &dyn ChatBackend = state
        .engine
        .as_deref()
        .ok_or_else(|| GatewayError::Internal("ChatEngine not configured".into()))?;

    // 3) 执行推理（池 / lease / payload / bridge）。
    let outcome = engine.chat_outcome(&provider_req).await?;

    // 4) 组装 OpenAI 兼容响应 / SSE。
    let model_out = if ocr { ALIAS_OCR } else { "grok-chat" };
    let mut response = if req.stream {
        stream_response(provider_req.request_id, model_out, outcome.text).into_response()
    } else {
        Json(chat_completion_json(
            provider_req.request_id,
            model_out,
            outcome.text,
            outcome.account_id,
        ))
        .into_response()
    };
    if let Some(id) = outcome.account_id {
        response.headers_mut().insert(
            header::HeaderName::from_static("x-grok-account-id"),
            header::HeaderValue::from_str(&id.to_string()).unwrap_or(header::HeaderValue::from_static("0")),
        );
    }
    Ok(response)
}

/// 非流式 OpenAI 兼容 `chat.completion`。
fn chat_completion_json(id: String, model: &str, content: String, account_id: Option<i64>) -> Value {
    json!({
        "id": id,
        "object": "chat.completion",
        "created": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
        "usage": { "total_tokens": null },
        "account_id": account_id,
    })
}

/// 流式：完整文本以单个 chunk 的 delta 发出，然后 `[DONE]`（G-OCR-9）。
fn stream_response(
    id: String,
    model: &str,
    content: String,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
    });
    let s = stream::iter(vec![
        Ok(Event::default().data(chunk.to_string())),
        Ok(Event::default().data("[DONE]")),
    ]);
    Sse::new(s)
}
