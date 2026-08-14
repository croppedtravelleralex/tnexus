//! grok-gateway HTTP handlers（docs/39d §2.1 推理端点，docs/39 主文档 §6.1 P0/P1）。
//!
//! G1 端点：`GET /v1/models`、`POST /v1/chat/completions`（含识图 OCR）。
//!
//! OCR 判定（39 主文档 §4.2 + §4.1 治理）：
//! 请求 `model == grok-vision-ocr` **或** 消息含 image_url → 走 OCR 路径
//! （`grok-chat-fast` + `enableImageGeneration=false`）。这符合 §4.1「普通带图
//! 请求 enableImageGeneration=true 可能触发上游生图副作用，移植时需显式治理」——
//! 在 gateway 边界把带图请求统一送 OCR/识图路径（禁生图），避免意外生图。

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::{stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use grok_conversation::{normalize_chat_input, strip_grok_markup, ChatMessage, NormalizedChatInput};
use grok_domain::{
    public_models, ChatBackend, ChatRequest as ProviderChatRequest, ChatStreamEvent, ImagineRequest,
    ProviderError, ALIAS_OCR,
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

/// `POST /v1/chat/completions`（含 OCR）。stream=true 走上游增量 SSE。
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatCompletionsRequest>,
) -> Result<axum::response::Response, GatewayError> {
    let normalized = normalize_chat_input(req.messages).map_err(GatewayError::from)?;
    let ocr = is_ocr_request(&req.model, &normalized.images);
    let provider_req = ProviderChatRequest {
        prompt: normalized.prompt,
        images: normalized.images,
        ocr,
        system_prompt: None,
        request_id: new_request_id(),
        event_sink: None,
    };

    let engine = state
        .engine
        .clone()
        .ok_or_else(|| GatewayError::Internal("ChatEngine not configured".into()))?;

    let model_out = if ocr { ALIAS_OCR } else { "grok-chat" };
    if req.stream {
        return stream_chat_completions(engine, provider_req, model_out).await;
    }

    let outcome = engine.chat_outcome(&provider_req).await?;
    let text = strip_grok_markup(&outcome.text);
    let mut response = Json(chat_completion_json(
        provider_req.request_id,
        model_out,
        text,
        outcome.account_id,
    ))
    .into_response();
    set_account_header(&mut response, outcome.account_id);
    Ok(response)
}

enum StreamMsg {
    Account(i64),
    Delta(String),
    End,
    Failed(ProviderError),
}

async fn stream_chat_completions(
    engine: Arc<dyn ChatBackend>,
    mut provider_req: ProviderChatRequest,
    model_out: &'static str,
) -> Result<Response, GatewayError> {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamMsg>();
    let sink = {
        let tx = tx.clone();
        Arc::new(move |ev: ChatStreamEvent| {
            let msg = match ev {
                ChatStreamEvent::Account(id) => StreamMsg::Account(id),
                ChatStreamEvent::Token(t) => StreamMsg::Delta(t),
            };
            let _ = tx.send(msg);
        })
    };
    provider_req.event_sink = Some(sink);
    let request_id = provider_req.request_id.clone();
    tokio::spawn(async move {
        match engine.chat_outcome(&provider_req).await {
            Ok(outcome) => {
                if let Some(id) = outcome.account_id {
                    let _ = tx.send(StreamMsg::Account(id));
                }
                let _ = tx.send(StreamMsg::End);
            }
            Err(e) => {
                let _ = tx.send(StreamMsg::Failed(e));
            }
        }
    });

    let mut account_id = None;
    let first = loop {
        match rx.recv().await {
            Some(StreamMsg::Account(id)) => {
                account_id = Some(id);
                break StreamMsg::Account(id);
            }
            Some(StreamMsg::Failed(e)) => return Err(e.into()),
            Some(other) => break other,
            None => {
                return Err(GatewayError::Internal("chat stream closed".into()));
            }
        }
    };

    let mut response =
        live_stream_response(request_id, model_out.to_string(), first, rx).into_response();
    set_account_header(&mut response, account_id);
    Ok(response)
}

fn set_account_header(response: &mut Response, account_id: Option<i64>) {
    if let Some(id) = account_id {
        response.headers_mut().insert(
            header::HeaderName::from_static("x-grok-account-id"),
            header::HeaderValue::from_str(&id.to_string())
                .unwrap_or(header::HeaderValue::from_static("0")),
        );
    }
}

fn sse_delta_event(id: &str, model: &str, content: &str, finish: bool) -> Event {
    let chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": content },
            "finish_reason": if finish { Value::from("stop") } else { Value::Null },
        }],
    });
    Event::default().data(chunk.to_string())
}

fn live_stream_response(
    id: String,
    model: String,
    first: StreamMsg,
    rx: mpsc::UnboundedReceiver<StreamMsg>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let prelude: Vec<Result<Event, Infallible>> = match &first {
        StreamMsg::Delta(text) => vec![Ok(sse_delta_event(&id, &model, text, false))],
        StreamMsg::End => vec![
            Ok(sse_delta_event(&id, &model, "", true)),
            Ok(Event::default().data("[DONE]")),
        ],
        StreamMsg::Account(_) | StreamMsg::Failed(_) => vec![],
    };
    let already_done = matches!(first, StreamMsg::End);
    let rest = stream::unfold(
        (rx, id, model, already_done),
        |(mut rx, id, model, done)| async move {
            if done {
                return None;
            }
            loop {
                match rx.recv().await {
                    Some(StreamMsg::Account(_)) => continue,
                    Some(StreamMsg::Delta(text)) => {
                        return Some((
                            Ok(sse_delta_event(&id, &model, &text, false)),
                            (rx, id, model, false),
                        ));
                    }
                    Some(StreamMsg::Failed(e)) => {
                        return Some((
                            Ok(sse_delta_event(&id, &model, &e.to_string(), true)),
                            (rx, id, model, false),
                        ));
                    }
                    Some(StreamMsg::End) | None => {
                        return Some((
                            Ok(Event::default().data("[DONE]")),
                            (rx, id, model, true),
                        ));
                    }
                }
            }
        },
    );
    Sse::new(stream::iter(prelude).chain(rest))
}

/// 非流式 OpenAI 兼容 `chat.completion`。
fn chat_completion_json(
    id: String,
    model: &str,
    content: String,
    account_id: Option<i64>,
) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_delta_event_omits_finish_until_stop() {
        let chunk = json!({
            "id": "id-1",
            "object": "chat.completion.chunk",
            "model": "grok-chat",
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant", "content": "**hi**" },
                "finish_reason": Value::Null,
            }],
        });
        assert_eq!(chunk["choices"][0]["finish_reason"], Value::Null);
        assert_eq!(chunk["choices"][0]["delta"]["content"], "**hi**");
        let ev = sse_delta_event("id-1", "grok-chat", "**hi**", false);
        let _ = ev;
    }

    #[test]
    fn non_stream_json_strips_via_helper() {
        let text = strip_grok_markup("**是的**<grok:render>x</grok:render>");
        let body = chat_completion_json("x".into(), "grok-chat", text, Some(7));
        assert_eq!(body["choices"][0]["message"]["content"], "**是的**");
        assert_eq!(body["account_id"], 7);
        assert!(!body.to_string().contains("grok:render"));
    }
}
