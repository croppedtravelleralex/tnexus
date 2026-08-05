//! G2 生图 E2E（docs/39a G2-A*，docs/39c §3 生图矩阵）。
//!
//! 用 tower `ServiceExt::oneshot` 驱动 axum app，bridge 用 mock（`MockBridgeClient` 的
//! `imagine_response`），号池注入单账号。不发起真实 HTTP。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use grok_domain::{Account, AuthStatus, Provider, egress::Scope};
use grok_egress::InMemoryLeaseManager;
use grok_image_pipeline::{ImagePipeline, InMemoryTraceRepository, SlotManager};
use grok_pool::SimplifiedPool;
use grok_provider_web::{ImageEngine, MockBridgeClient};

use grok_gateway::{build_app, with_engines_and_media};

use grok_gateway::handlers::MediaFetcher;

/// 内存媒体取回 fake：按 id 返回预置 PNG 字节。
struct FakeMedia {
    bytes: Vec<u8>,
}

#[async_trait::async_trait]
impl MediaFetcher for FakeMedia {
    async fn fetch_bytes(&self, _id: &str) -> Result<(Vec<u8>, String), grok_gateway::GatewayError> {
        Ok((self.bytes.clone(), "image/png".to_string()))
    }
}

fn sample_account(id: i64) -> Account {
    Account {
        id,
        identity_key: format!("web-{id}"),
        provider: Provider::GrokWeb,
        enabled: true,
        auth_status: AuthStatus::Active,
        priority: 0,
        observed_model: None,
    }
}

fn test_pipeline() -> ImagePipeline {
    ImagePipeline::new(
        SlotManager::new(&[("ps", 2), ("ss", 1)]),
        Arc::new(InMemoryTraceRepository::new()),
    )
}

type SharedPool = Arc<SimplifiedPool>;

/// 构图：单账号 + mock bridge（imagine_response）+ 内存 pipeline + 媒体 fake。
async fn app_with_media(mock: MockBridgeClient) -> axum::Router {
    let pool: SharedPool = Arc::new(SimplifiedPool::new());
    pool.load_in_memory(vec![sample_account(11)]).await;
    let lease: Arc<dyn grok_egress::LeaseManager> =
        Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
    let bridge: Arc<dyn grok_provider_web::BridgeClient> = Arc::new(mock);
    let image = ImageEngine::new(pool, lease, bridge, None, test_pipeline());

    let media: Arc<dyn MediaFetcher> = Arc::new(FakeMedia {
        bytes: vec![0x89, 0x50, 0x4E, 0x47], // PNG magic
    });

    build_app(with_engines_and_media(build_chat_engine_for_test(), image, media))
}

fn build_chat_engine_for_test() -> grok_provider_web::ChatEngine {
    use grok_egress::LeaseManager;
    let pool: SharedPool = Arc::new(SimplifiedPool::new());
    let lease: Arc<dyn LeaseManager> =
        Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
    let bridge: Arc<dyn grok_provider_web::BridgeClient> = Arc::new(MockBridgeClient::new());
    grok_provider_web::ChatEngine::new(pool, lease, bridge, None)
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

/// G2-A*：POST /v1/images/generations → 200 + data[0].url。
#[tokio::test]
async fn image_generations_returns_url() {
    let mut mock = MockBridgeClient::new();
    mock.imagine_response = serde_json::json!({ "data": [ {"url": "https://cdn/img.png"} ] });
    let app = app_with_media(mock).await;

    let req = json!({ "prompt": "a red fox", "n": 1, "response_format": "url" });
    let (status, body) = post(&app, "/v1/images/generations", req).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["data"][0]["url"], "https://cdn/img.png");
}

/// n=0 → 400。
#[tokio::test]
async fn image_generations_rejects_zero_n() {
    let app = app_with_media(MockBridgeClient::new()).await;
    let req = json!({ "prompt": "x", "n": 0 });
    let (status, body) = post(&app, "/v1/images/generations", req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

/// engine 未配置 → 503/500（build_app_empty 无 image engine）。
#[tokio::test]
async fn image_generations_needs_engine() {
    let app = grok_gateway::build_app(grok_gateway::AppState::empty());
    let req = json!({ "prompt": "x", "n": 1 });
    let (status, _) = post(&app, "/v1/images/generations", req).await;
    // Internal → 500。
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// G2-A4：GET /v1/media/images/{id} → 200（注入媒体 fake）。
#[tokio::test]
async fn media_images_returns_200_with_bytes() {
    let app = app_with_media(MockBridgeClient::new()).await;
    let req = Request::builder()
        .method("GET")
        .uri("/v1/media/images/img-1")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert_eq!(ct, "image/png");
}
