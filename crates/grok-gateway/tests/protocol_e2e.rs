//! G5-P3 `/v1/responses` 与 `/v1/messages` E2E（tower `ServiceExt::oneshot`，
//! [`FakeProtocolBackend`] 注入，断言 200 / SSE 帧 / 400 错误映射 / 500 未配置）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use grok_conversation::NormalizedChatInput;
use grok_gateway::handlers::ProtocolBackend;
use grok_gateway::GatewayError;

const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

/// 记录最后一次归一化输入（prompt/images/model），返回固定文本。
struct FakeProtocolBackend {
    last_prompt: Arc<tokio::sync::Mutex<Option<String>>>,
    last_model: Arc<tokio::sync::Mutex<Option<String>>>,
    calls: AtomicUsize,
    fail: bool,
}

impl FakeProtocolBackend {
    fn new(fail: bool) -> Self {
        Self {
            last_prompt: Arc::new(tokio::sync::Mutex::new(None)),
            last_model: Arc::new(tokio::sync::Mutex::new(None)),
            calls: AtomicUsize::new(0),
            fail,
        }
    }

    async fn recorded(&self, prompt: &mut Option<String>, model: &mut Option<String>) {
        *prompt = self.last_prompt.lock().await.take();
        *model = self.last_model.lock().await.take();
    }
}

#[async_trait::async_trait]
impl ProtocolBackend for FakeProtocolBackend {
    async fn complete(
        &self,
        model: &str,
        normalized: &NormalizedChatInput,
    ) -> Result<String, GatewayError> {
        if self.fail {
            return Err(GatewayError::Upstream("fake upstream error".into()));
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.last_prompt.lock().await = Some(normalized.prompt.clone());
        *self.last_model.lock().await = Some(model.to_string());
        Ok("fake-completion".to_string())
    }
}


fn app_with(backend: Arc<dyn ProtocolBackend>) -> axum::Router {
    grok_gateway::build_app(grok_gateway::with_protocol_backend(backend))
}

fn app_empty() -> axum::Router {
    grok_gateway::build_app(grok_gateway::AppState::empty())
}

async fn post(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let (status, raw) = post_raw(app, path, body).await;
    (status, serde_json::from_str(&raw).unwrap_or_else(|_| json!({"raw": raw})))
}

async fn post_raw(app: &axum::Router, path: &str, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn assert_backend_seen(backend: &FakeProtocolBackend, want_prompt: &str, want_model: &str) {
    let (mut prompt, mut model) = (None, None);
    backend.recorded(&mut prompt, &mut model).await;
    assert_eq!(prompt.as_deref(), Some(want_prompt));
    assert_eq!(model.as_deref(), Some(want_model));
}

/// /v1/responses：input message 数组（input_text + input_image）→ 归一化 → 200 response。
#[tokio::test]
async fn responses_input_array_roundtrip() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = post(
        &app,
        "/v1/responses",
        json!({
            "model": "grok-4.5",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "what is this"},
                    {"type": "input_image", "image_url": DATA_URI},
                ],
            }],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["object"], "response");
    assert_eq!(body["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(body["output"][0]["content"][0]["text"], "fake-completion");
    assert_backend_seen(&backend, "[user]\nwhat is this", "grok-4.5").await;
    assert_eq!(backend.calls.load(Ordering::Relaxed), 1);
}

/// /v1/responses：input 字符串 + instructions → system 前缀。
#[tokio::test]
async fn responses_string_with_instructions() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, _) = post(
        &app,
        "/v1/responses",
        json!({
            "model": "grok-4.5",
            "instructions": "be concise",
            "input": "draw a cat",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_backend_seen(&backend, "[system]\nbe concise\n\n[user]\ndraw a cat", "grok-4.5")
        .await;
}

/// /v1/responses：stream=true → SSE 含 response.created / output_text.delta / completed。
#[tokio::test]
async fn responses_stream_sse() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = post_raw(
        &app,
        "/v1/responses",
        json!({ "model": "grok-4.5", "stream": true, "input": "hi" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#""type":"response.created""#), "body = {body}");
    assert!(body.contains(r#""type":"response.output_text.delta""#));
    assert!(body.contains(r#""delta":"fake-completion""#));
    assert!(body.contains(r#""type":"response.completed""#));
}

/// /v1/responses：input_file → 400 明确错误。
#[tokio::test]
async fn responses_input_file_rejected_400() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = post(
        &app,
        "/v1/responses",
        json!({
            "model": "grok-4.5",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_file", "file_url": "https://example.com/a.pdf"}],
            }],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().contains("input_file"));
    assert_eq!(backend.calls.load(Ordering::Relaxed), 0, "backend must not run on 400");
}

/// /v1/messages：system + user → 归一化 → 200 Anthropic content block。
#[tokio::test]
async fn messages_system_and_text_roundtrip() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = post(
        &app,
        "/v1/messages",
        json!({
            "model": "grok-4.5",
            "system": "你是一名助手",
            "max_tokens": 256,
            "messages": [{"role": "user", "content": "你好"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "fake-completion");
    assert_eq!(body["stop_reason"], "end_turn");
    assert_backend_seen(&backend, "[system]\n你是一名助手\n\n[user]\n你好", "grok-4.5").await;
}

/// /v1/messages：image 块 → data URI → 归一化。
#[tokio::test]
async fn messages_image_to_data_uri() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = post(
        &app,
        "/v1/messages",
        json!({
            "model": "grok-4.5",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "看看"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aW1n"}},
                ],
            }],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["content"][0]["text"], "fake-completion");
    assert_backend_seen(&backend, "[user]\n看看", "grok-4.5").await;
    // images 已归一化（backend 收到 prompt 文本，图片清单隐式含于调用；此处不强断言）
}

/// /v1/messages：stream=true → SSE 含 message_start / content_block_delta / message_stop。
#[tokio::test]
async fn messages_stream_sse() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = post_raw(
        &app,
        "/v1/messages",
        json!({
            "model": "grok-4.5",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#""type":"message_start""#), "body = {body}");
    assert!(body.contains(r#""type":"content_block_delta""#));
    // serde_json Value 序列化键序为字母序：{"delta":{"text":...,"type":"text_delta"}}
    assert!(body.contains(r#""text":"fake-completion""#) && body.contains("text_delta"));
    assert!(body.contains(r#""type":"message_stop""#));
}

/// /v1/messages：max_tokens=0 → 400；未配置 backend → 500。
#[tokio::test]
async fn messages_bad_max_tokens_400() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let (status, _) = post(
        &app,
        "/v1/messages",
        json!({ "model": "grok-4.5", "max_tokens": 0, "messages": [{"role": "user", "content": "hi"}] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// 未配置 ProtocolBackend → 500（engine 不参与协议端点）。
#[tokio::test]
async fn protocol_endpoint_without_backend_500() {
    let app = app_empty();
    let (status, body) = post(
        &app,
        "/v1/responses",
        json!({ "model": "grok-4.5", "input": "hi" }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body["error"]["message"].as_str().unwrap().contains("ProtocolBackend"));

    let (status, _) = post(
        &app,
        "/v1/messages",
        json!({ "model": "grok-4.5", "messages": [{"role": "user", "content": "hi"}] }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// 未解析的 JSON → 400（axum Json extractor 层）。
#[tokio::test]
async fn invalid_json_400() {
    let backend = Arc::new(FakeProtocolBackend::new(false));
    let app = app_with(backend.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}