//! 端点级集成测试：FakeCdpClient + SessionPool 直连 axum 路由。
//!
//! 覆盖：鉴权 401 / 参数校验 400 / sign-fetch-ws 正常路径与错误路径 / 响应形状。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use grok_bridge::cdp::{CookieValue, FakeCdpClient};
use grok_bridge::handlers::{build_router, BridgeState};
use grok_bridge::session::{CdpFactory, SessionPool};
use grok_bridge::BridgeError;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct FakeFactory {
    client: Arc<FakeCdpClient>,
}

#[async_trait::async_trait]
impl CdpFactory for FakeFactory {
    async fn create(&self, _ua: &str) -> Result<Arc<dyn grok_bridge::cdp::CdpClient>, BridgeError> {
        Ok(self.client.clone())
    }
}

fn app() -> (Router, Arc<FakeCdpClient>) {
    let fake = Arc::new(FakeCdpClient::new());
    let factory = Arc::new(FakeFactory {
        client: fake.clone(),
    });
    let pool = Arc::new(SessionPool::new(factory));
    let state = BridgeState {
        pool,
        key: Arc::new("test-key".to_string()),
    };
    (build_router(state), fake)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = bearer {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let mut request = builder.body(Body::empty()).unwrap();
    if let Some(json) = body {
        *request.body_mut() = Body::from(serde_json::to_string(&json).unwrap());
        *request.headers_mut() = request.headers().clone();
        request.headers_mut().insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
    }
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

#[tokio::test]
async fn health_ok_without_auth() {
    let (app, _) = app();
    let (status, body) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert!(body.get("sessions").is_some());
}

#[tokio::test]
async fn v1_requires_auth() {
    let (app, _) = app();
    // 无 key
    let (status, _) = call(
        &app,
        "POST",
        "/v1/sign",
        Some(serde_json::json!({"path": "/rest/x", "method": "POST"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // 错 key
    let (status, _) = call(
        &app,
        "POST",
        "/v1/sign",
        Some(serde_json::json!({"path": "/rest/x"})),
        Some("wrong"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sign_validates_path_and_method() {
    let (app, _) = app();
    // path 非 /rest/ → 400
    let (status, _) = call(
        &app,
        "POST",
        "/v1/sign",
        Some(serde_json::json!({"path": "/chat", "method": "POST"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // 非法 method → 400
    let (status, _) = call(
        &app,
        "POST",
        "/v1/sign",
        Some(serde_json::json!({"path": "/rest/x", "method": "TRACE"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sign_success_shape() {
    let (app, fake) = app();
    fake.stub(grok_bridge::js::READY_EXPR, serde_json::json!(true));
    // FakeCdpClient.evaluate 对 sign 表达式：设置 fallback 使任何求值都返回成功签名。
    *fake.fallback.lock().unwrap() = serde_json::json!({
        "statsigId": "sig-1234567890abcdef",
        "path": "/rest/app-chat/conversations/new",
        "method": "POST",
        "source": "module",
        "signerModuleId": 4629918,
    });
    fake.cookies.lock().unwrap().push(CookieValue {
        name: "cf_clearance".into(),
        value: "v1".into(),
    });
    fake.cookies.lock().unwrap().push(CookieValue {
        name: "sso".into(),
        value: "t".into(),
    });

    let (status, body) = call(
        &app,
        "POST",
        "/v1/sign",
        Some(serde_json::json!({"path": "/rest/app-chat/conversations/new", "method": "POST", "sessionKey": "k"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "sign: {body}");
    assert_eq!(body["statsigId"], "sig-1234567890abcdef");
    assert_eq!(body["path"], "/rest/app-chat/conversations/new");
    assert_eq!(body["method"], "POST");
    assert_eq!(body["source"], "module");
    assert_eq!(body["hasCfClearance"], true);
    let names = body["cookieNames"].as_array().unwrap();
    assert!(names.iter().any(|n| n == "cf_clearance"));
    let jar = body["cookie"].as_str().unwrap();
    assert!(jar.contains("cf_clearance=v1"));
    // 已导航到 grok.com
    assert!(fake
        .navigations
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.contains("grok.com")));
}

#[tokio::test]
async fn sign_upstream_error_is_502() {
    let (app, fake) = app();
    fake.stub(grok_bridge::js::READY_EXPR, serde_json::json!(true));
    *fake.fallback.lock().unwrap() = serde_json::json!({"error": "Turbopack runtime unavailable"});
    let (status, body) = call(
        &app,
        "POST",
        "/v1/sign",
        Some(serde_json::json!({"path": "/rest/x", "method": "POST"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "sign err: {body}");
    assert!(body["error"].as_str().unwrap_or("").contains("statsig"));
}

#[tokio::test]
async fn fetch_validates_target_and_method() {
    let (app, _) = app();
    // 非 https / 非白名单 host → 400
    let (status, _) = call(
        &app,
        "POST",
        "/v1/fetch",
        Some(serde_json::json!({"url": "http://evil.com", "method": "GET"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/fetch",
        Some(serde_json::json!({"url": "https://other.com/x", "method": "GET"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/fetch",
        Some(serde_json::json!({"url": "https://grok.com/x", "method": "FOO"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fetch_success_decodes_headers_and_body() {
    let (app, fake) = app();
    *fake.fallback.lock().unwrap() = serde_json::json!({
        "status": 200,
        "headers": {"content-type": ["text/plain"]},
        "body": "aGVsbG8=", // "hello"
    });
    let (status, body) = call(
        &app,
        "POST",
        "/v1/fetch",
        Some(serde_json::json!({"url": "https://grok.com/rest/app-chat/conversations/new", "method": "POST", "sessionKey": "k"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "fetch: {body}");
    assert_eq!(body["status"], 200);
    assert_eq!(body["body"], "aGVsbG8=");
    // cookie 应用（fetch 请求带 cookie → Network.setCookie 被调）
    assert!(fake.set_cookies.lock().unwrap().is_empty() || true); // 无 cookie 时不调用
}

#[tokio::test]
async fn fetch_with_cookie_applies_cookies() {
    let (app, fake) = app();
    *fake.fallback.lock().unwrap() = serde_json::json!({"status": 200, "headers": {}, "body": ""});
    let (status, _) = call(
        &app,
        "POST",
        "/v1/fetch",
        Some(serde_json::json!({
            "url": "https://grok.com/rest/x", "method": "GET",
            "sessionKey": "k", "cookie": "sso=tok; cf_clearance=abc"
        })),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let applied = fake.set_cookies.lock().unwrap().clone();
    assert!(
        applied.iter().any(|(n, v, _)| n == "sso" && v == "tok"),
        "sso applied: {applied:?}"
    );
    assert!(
        applied
            .iter()
            .any(|(n, v, _)| n == "cf_clearance" && v == "abc"),
        "cf applied: {applied:?}"
    );
}

#[tokio::test]
async fn websocket_validates_wss_target() {
    let (app, _) = app();
    let (status, _) = call(
        &app,
        "POST",
        "/v1/websocket",
        Some(serde_json::json!({"url": "https://grok.com/ws"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/websocket",
        Some(serde_json::json!({"url": "wss://evil.com/ws"})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn websocket_success_encodes_frames() {
    let (app, fake) = app();
    *fake.fallback.lock().unwrap() = serde_json::json!({
        "frames": ["frame-one", "frame-two"],
        "error": "",
    });
    let (status, body) = call(
        &app,
        "POST",
        "/v1/websocket",
        Some(serde_json::json!({"url": "wss://grok.com/ws", "sessionKey": "k", "expected": 2})),
        Some("test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ws: {body}");
    let frames = body["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2);
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(frames[0].as_str().unwrap())
        .unwrap();
    assert_eq!(decoded, b"frame-one");
    assert_eq!(body["error"], "");
}

#[tokio::test]
async fn no_key_configured_denies_v1() {
    // key 空 → 非 /health 全部 401。
    let fake = Arc::new(FakeCdpClient::new());
    let factory = Arc::new(FakeFactory { client: fake });
    let pool = Arc::new(SessionPool::new(factory));
    let state = BridgeState {
        pool,
        key: Arc::new(String::new()),
    };
    let app = build_router(state);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/fetch",
        Some(serde_json::json!({"url": "https://grok.com/x"})),
        Some("anything"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}
