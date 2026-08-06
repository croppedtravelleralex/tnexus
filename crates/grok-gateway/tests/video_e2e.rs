//! G5-P4 `/v1/videos` E2E（tower `ServiceExt::oneshot`，[`FakeVideoBackend`] 注入）。
//!
//! 覆盖（迁移 Go `video_test.go` 核心用例）：
//! - 创建成功：POST 返回 200 + `status=queued/processing` + id；prompt 透传
//! - 轮询成功：首次 processing → 之后 completed（含 url）
//! - 失败状态映射：backend 报 failed → job.error 携带 code/message
//! - 非法 id：GET 未命中 → 404；空 id → 404
//! - 校验：缺 prompt 且无参考图 → 400

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use grok_gateway::video::{VideoBackend, VideoJob, VideoRequest, VideoStatus};
use grok_gateway::GatewayError;

/// 记录创建参数，轮询走「processing → completed」或「failed」剧本。
struct FakeVideoBackend {
    created_prompt: Arc<Mutex<Option<String>>>,
    poll_calls: AtomicUsize,
    /// 非空 → 轮询第 1 次返回该失败；空 → 第 2 次完成。
    fail_first: bool,
}

impl FakeVideoBackend {
    fn new(fail_first: bool) -> Self {
        Self {
            created_prompt: Arc::new(Mutex::new(None)),
            poll_calls: AtomicUsize::new(0),
            fail_first,
        }
    }

    fn completed_job(&self) -> VideoJob {
        VideoJob {
            id: "video_1".into(),
            object: "video",
            status: VideoStatus::Completed,
            created: 1_800_000_000,
            prompt: Some("moving camera".into()),
            duration: 8,
            aspect_ratio: "16:9".into(),
            resolution: "720p".into(),
            reference_urls: Vec::new(),
            progress: 100,
            url: Some("https://cdn.example.com/video_1.mp4".into()),
            content_type: Some("video/mp4".into()),
            error: None,
        }
    }

    fn processing_job(&self) -> VideoJob {
        let mut job = self.completed_job();
        job.status = VideoStatus::Processing;
        job.progress = 30;
        job.url = None;
        job.content_type = None;
        job
    }
}

#[async_trait::async_trait]
impl VideoBackend for FakeVideoBackend {
    async fn create_video(&self, input: VideoRequest) -> Result<VideoJob, GatewayError> {
        *self.created_prompt.lock().unwrap() = input.prompt.clone();
        Ok(self.processing_job())
    }

    async fn poll_video(&self, id: &str) -> Result<VideoJob, GatewayError> {
        if id != "video_1" {
            return Err(GatewayError::NotFound(format!("video {id} not found")));
        }
        let calls = self.poll_calls.fetch_add(1, Ordering::Relaxed);
        if self.fail_first {
            if calls == 0 {
                let mut job = self.completed_job();
                job.status = VideoStatus::Failed;
                job.error = Some(grok_gateway::video::VideoError {
                    code: "generation_failed".into(),
                    message: "上游生成失败".into(),
                });
                return Ok(job);
            }
            return Ok(self.completed_job());
        }
        // 剧本：首次 processing，之后 completed
        if calls == 0 {
            Ok(self.processing_job())
        } else {
            Ok(self.completed_job())
        }
    }
}

fn app_with(backend: Arc<FakeVideoBackend>) -> axum::Router {
    grok_gateway::build_app(grok_gateway::with_video_backend(backend))
}

fn app_empty() -> axum::Router {
    grok_gateway::build_app(grok_gateway::AppState::empty())
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(path);
    let (builder, body) = if let Some(body) = body {
        (
            builder.header("content-type", "application/json"),
            Some(Body::from(body.to_string())),
        )
    } else {
        (builder, None)
    };
    let req = builder.body(body.unwrap_or_else(Body::empty)).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let raw = String::from_utf8_lossy(&bytes).to_string();
    (
        status,
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({"raw": raw})),
    )
}

/// POST /v1/videos：创建成功 → 201 + 任务快照（status=processing，prompt 透传）。
#[tokio::test]
async fn create_video_returns_job_describing_progress() {
    let backend = Arc::new(FakeVideoBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = send(
        &app,
        "POST",
        "/v1/videos",
        Some(json!({
            "model": "grok-video",
            "prompt": "一只猫在跳",
            "duration": 8,
            "aspect_ratio": "16:9",
            "resolution": "720p",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body = {body}");
    assert_eq!(body["id"], "video_1");
    assert_eq!(body["object"], "video");
    assert_eq!(body["status"], "processing");
    assert_eq!(body["duration"], 8);
    assert!(
        body.get("reference_urls").is_none(),
        "空 reference_urls 不应序列化"
    );
    assert_eq!(
        *backend.created_prompt.lock().unwrap(),
        Some("一只猫在跳".into()),
        "prompt 原样透传"
    );
}

/// 轮询剧本：首次 GET processing，再次 GET completed（含 url / content_type / progress=100）。
#[tokio::test]
async fn poll_video_reaches_completed_after_processing() {
    let backend = Arc::new(FakeVideoBackend::new(false));
    let app = app_with(backend.clone());

    let (status, body) = send(&app, "GET", "/v1/videos/video_1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "processing");
    assert!(body.get("url").is_none());

    let (status, body) = send(&app, "GET", "/v1/videos/video_1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "completed");
    assert_eq!(body["progress"], 100);
    assert_eq!(body["url"], "https://cdn.example.com/video_1.mp4");
    assert_eq!(body["content_type"], "video/mp4");
}

/// 失败状态映射：backend 首次轮询返回 failed → job.error 携带 code/message。
#[tokio::test]
async fn poll_video_maps_failed_status_with_error() {
    let backend = Arc::new(FakeVideoBackend::new(true));
    let app = app_with(backend.clone());
    let (status, body) = send(&app, "GET", "/v1/videos/video_1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "failed");
    assert_eq!(body["error"]["code"], "generation_failed");
    assert_eq!(body["error"]["message"], "上游生成失败");
}

/// 非法/未知 id → 404。
#[tokio::test]
async fn poll_unknown_video_404() {
    let backend = Arc::new(FakeVideoBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = send(&app, "GET", "/v1/videos/video_missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

/// 校验：缺 prompt 且无参考图 → 400；未配置 video_backend → 500。
#[tokio::test]
async fn video_request_validation_and_unconfigured() {
    let backend = Arc::new(FakeVideoBackend::new(false));
    let app = app_with(backend.clone());
    let (status, body) = send(
        &app,
        "POST",
        "/v1/videos",
        Some(json!({ "model": "grok-video", "duration": 8 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("prompt"));
    assert_eq!(
        backend.poll_calls.load(Ordering::Relaxed),
        0,
        "backend 不应在 400 时执行"
    );

    // duration <= 0 → 400
    let (status, _) = send(
        &app,
        "POST",
        "/v1/videos",
        Some(json!({ "model": "grok-video", "prompt": "x", "duration": 0 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 图片生视频可省略 prompt（reference_urls 提供）→ 200
    let (status, body) = send(
        &app,
        "POST",
        "/v1/videos",
        Some(json!({
            "model": "grok-video",
            "reference_urls": ["https://example.com/i.png"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body = {body}");
}

/// 未配置 VideoBackend → 500。
#[tokio::test]
async fn video_endpoint_without_backend_500() {
    let app = app_empty();
    let (status, body) = send(
        &app,
        "POST",
        "/v1/videos",
        Some(json!({ "model": "grok-video", "prompt": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("VideoBackend"));

    let (status, _) = send(&app, "GET", "/v1/videos/video_1", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// 未解析 JSON → 400。
#[tokio::test]
async fn invalid_json_400() {
    let backend = Arc::new(FakeVideoBackend::new(false));
    let app = app_with(backend.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/videos")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
