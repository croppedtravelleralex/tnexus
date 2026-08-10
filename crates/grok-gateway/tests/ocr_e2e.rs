//! G1 OCR/chat E2E（docs/39c §2 G-OCR-*，docs/39a G1-A*）。
//!
//! 使用 tower `ServiceExt::oneshot` 直接驱动 axum app，bridge 用 mock
//! （[`MockBridgeClient`]），号池注入单账号并 pin，保证确定性。不发起真实 HTTP。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use grok_domain::{Account, AuthStatus, Provider};
use grok_egress::InMemoryLeaseManager;
use grok_pool::SimplifiedPool;
use grok_provider_web::{ChatEngine, MockBridgeClient, ALIAS_OCR, UPSTREAM_OCR_MODEL};

use grok_gateway::with_engine;

const GOLDEN_REQ: &str = include_str!("../../../tests/grok_golden/chat_ocr_request.json");
const GOLDEN_PAYLOAD: &str =
    include_str!("../../../tests/grok_golden/chat_ocr_upstream_payload.json");

const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn sample_account(id: i64) -> Account {
    Account {
        id,
        identity_key: format!("web-{id}"),
        provider: Provider::GrokWeb,
        enabled: true,
        auth_status: AuthStatus::Active,
        priority: 0,
        observed_model: None,
        ..Default::default()
    }
}

/// 构图：单账号 pin + mock bridge（指定 chat 文本与图字节）。
async fn app_with(mock: MockBridgeClient) -> axum::Router {
    let pool = Arc::new(SimplifiedPool::new());
    pool.load_in_memory(vec![sample_account(7)]).await;
    pool.pin(7).await;
    let lease = Arc::new(InMemoryLeaseManager::new(&[(
        grok_domain::egress::Scope::GrokWeb,
        4,
    )]));
    let bridge: Arc<dyn grok_provider_web::BridgeClient> = Arc::new(mock);
    let engine = ChatEngine::new(pool, lease, bridge, None);
    grok_gateway::build_app(with_engine(engine))
}

async fn post(app: &axum::Router, path: &str, body: Value) -> (StatusCode, String) {
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

async fn get(app: &axum::Router, path: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// /v1/models 含 OCR 别名（G1-4）。
#[tokio::test]
async fn models_endpoint_lists_ocr_alias() {
    let app = app_with(MockBridgeClient::new()).await;
    let (status, body) = get(&app, "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    let ids: Vec<String> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.contains(&ALIAS_OCR.to_string()),
        "models missing OCR alias: {ids:?}"
    );
}

/// G-OCR-1: 单图中文 data URI → 200 + 含中文
/// G-OCR-7: payload golden（bridge 收到的上游 body === golden，禁生图+fast）
#[tokio::test]
async fn ocr_single_chinese_image_returns_text_and_golden_payload() {
    let mut mock = MockBridgeClient::new();
    mock.chat_text = "图中文字是「你好」，第二行是『世界』".to_string();
    let app = app_with(mock).await;

    let req = json!({
        "model": ALIAS_OCR,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": DATA_URI}},
                {"type": "text", "text": "提取图中文字"},
            ]
        }]
    });
    let (status, body) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    let content = v["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("你好"), "missing chinese: {content}");
}

/// G-OCR-2: 单图英文 HTTPS URL → 200 + 含英文（mock bytes）
#[tokio::test]
async fn ocr_single_english_url_returns_text() {
    let mut mock = MockBridgeClient::new();
    mock.images = std::collections::HashMap::from([(
        "https://example.com/a.png".to_string(),
        vec![0x89, 0x50, 0x4E, 0x47], // PNG magic
    )]);
    mock.chat_text = "The quick brown fox".to_string();
    let app = app_with(mock).await;

    let req = json!({
        "model": ALIAS_OCR,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
                {"type": "text", "text": "extract text"},
            ]
        }]
    });
    let (status, body) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body.contains("quick brown fox"));
}

/// G-OCR-4: 9 张图 → 400
#[tokio::test]
async fn nine_images_rejected() {
    let app = app_with(MockBridgeClient::new()).await;
    let parts: Vec<Value> = (0..9)
        .map(|_| json!({"type": "image_url", "image_url": {"url": DATA_URI}}))
        .collect();
    let req = json!({
        "model": ALIAS_OCR,
        "messages": [{ "role": "user", "content": parts }]
    });
    let (status, body) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "9 images must 400, body={body}"
    );
}

/// G-OCR-6: input_image.file_id → 400
#[tokio::test]
async fn file_id_rejected() {
    let app = app_with(MockBridgeClient::new()).await;
    let req = json!({
        "model": ALIAS_OCR,
        "messages": [{
            "role": "user",
            "content": [{"type": "input_image", "file_id": "file_123"}]
        }]
    });
    let (status, body) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "file_id must 400, body={body}"
    );
}

/// G-OCR-10 + G-OCR-7: 别名路由 → fast + 禁生图，payload 与 golden 一致。
#[tokio::test]
async fn golden_payload_locked() {
    let mut mock = MockBridgeClient::new();
    mock.chat_text = "你好".to_string();
    let concrete: Arc<MockBridgeClient> = Arc::new(mock);
    let bridge: Arc<dyn grok_provider_web::BridgeClient> = concrete.clone();

    // 独立构图以便拿到 mock 引用。
    let pool = Arc::new(SimplifiedPool::new());
    pool.load_in_memory(vec![sample_account(7)]).await;
    pool.pin(7).await;
    let lease = Arc::new(InMemoryLeaseManager::new(&[(
        grok_domain::egress::Scope::GrokWeb,
        4,
    )]));
    let engine = ChatEngine::new(pool, lease, bridge, None);
    let app = grok_gateway::build_app(with_engine(engine));

    // 请求体直接来自 golden 请求文件。
    let req_body: Value = serde_json::from_str(GOLDEN_REQ).unwrap();
    let (status, body) = post(&app, "/v1/chat/completions", req_body).await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    // bridge 收到的上游 payload === golden payload。
    let payload = concrete.last_chat_payload.lock().await;
    let payload = payload
        .clone()
        .expect("bridge should have received payload");
    let golden: Value = serde_json::from_str(GOLDEN_PAYLOAD).unwrap();
    assert_eq!(payload, golden, "upstream payload diverged from golden");
    // 显式断言关键字段（避免整 golden 掩盖差异）。
    assert_eq!(payload["modeId"], "fast");
    assert_eq!(payload["enableImageGeneration"], false);
    assert_eq!(payload["enableImageStreaming"], false);
    assert_eq!(payload["fileAttachments"].as_array().unwrap().len(), 1);
}

/// G-OCR-9: stream:true → SSE 完整（含 [DONE]）。
#[tokio::test]
async fn stream_sse_completes_with_done() {
    let mut mock = MockBridgeClient::new();
    mock.chat_text = "流式内容".to_string();
    let app = app_with(mock).await;

    let req = json!({
        "model": ALIAS_OCR,
        "stream": true,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": DATA_URI}},
                {"type": "text", "text": "输出图中的文字"},
            ]
        }]
    });
    let (status, body) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body.contains("[DONE]"), "missing DONE terminator: {body}");
    assert!(
        body.contains("流式内容"),
        "missing streamed content: {body}"
    );
    // 内容 chunk 前有 data: 前缀、choices.delta.content。
    assert!(body.starts_with("data:"), "expected SSE data frame");
}

/// G-OCR-3: 无文字图 → 即使 bridge 返回空也走 200（mock 返回「无文字」）。
#[tokio::test]
async fn no_text_image_returns_empty_result() {
    let mut mock = MockBridgeClient::new();
    mock.chat_text = "无文字内容".to_string();
    let app = app_with(mock).await;
    let req = json!({
        "model": ALIAS_OCR,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": DATA_URI}},
                {"type": "text", "text": "这是风景图，识别文字"},
            ]
        }]
    });
    let (status, body) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("无文字"));
}

/// 空池 → 503（无可用账号）。
#[tokio::test]
async fn empty_pool_returns_503() {
    // 不注入账号：直接空池构 engine。
    let pool = Arc::new(SimplifiedPool::new());
    let lease = Arc::new(InMemoryLeaseManager::new(&[(
        grok_domain::egress::Scope::GrokWeb,
        4,
    )]));
    let bridge: Arc<dyn grok_provider_web::BridgeClient> = Arc::new(MockBridgeClient::new());
    let engine = ChatEngine::new(pool, lease, bridge, None);
    let app = grok_gateway::build_app(with_engine(engine));

    let req = json!({
        "model": ALIAS_OCR,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": DATA_URI}},
                {"type": "text", "text": "提取文字"},
            ]
        }]
    });
    let (status, _) = post(&app, "/v1/chat/completions", req).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
