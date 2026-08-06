//! grok-gateway 路由与状态（axum）。
//!
//! G1 最小集：`GET /v1/models`、`POST /v1/chat/completions`。路由挂
//! [`AppState`]，内含按需构建的 [`ChatEngine`]。
//!
//! [`build_app`] 接受已组装 engine 便于测试注入 mock（tower `ServiceExt::oneshot`）。
//! `/v1` 写操作（POST/PATCH/DELETE）可选鉴权：配置 `GATEWAY_AUTH_KEY` 后要求
//! `Authorization: Bearer <key>` 或 `X-API-Key: <key>`（见 [`require_gateway_auth`]）。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use grok_provider_web::{ChatEngine, ImageEngine};

use crate::handlers::{
    chat_completions, image_generations, media_images, messages_completions, models,
    responses_completions, MediaFetcher, ProtocolBackend,
};
use crate::video::{create_video, get_video, VideoBackend};

/// 应用共享状态。
#[derive(Clone)]
pub struct AppState {
    /// 推理引擎。生产在 `with_engine` 注入；默认 None（未配置时推理请求 500）。
    pub engine: Option<Arc<ChatEngine>>,
    /// 生图引擎（G2）。None 时 `/v1/images/generations` 返回 503。
    pub image_engine: Option<Arc<ImageEngine>>,
    /// 媒体取回器（G2-A4 `/v1/media/images/{id}`）。None 时返回 501。
    pub media_fetcher: Option<Arc<dyn MediaFetcher>>,
    /// `/v1/responses` 协议后端（Build provider，G5-P3）。None 时返回 500。
    pub responses_backend: Option<Arc<dyn ProtocolBackend>>,
    /// `/v1/messages` 协议后端（Console provider，G5-P3）。None 时返回 500。
    pub messages_backend: Option<Arc<dyn ProtocolBackend>>,
    /// G5-P4 视频后端（/v1/videos）。None 时返回 500。
    pub video_backend: Option<Arc<dyn VideoBackend>>,
    /// `/v1` 写操作（POST）的共享密钥（`GATEWAY_AUTH_KEY`）。
    /// `None` = 不校验（生产必须设置；启动时由 grok2api-rs 告警）。
    pub gateway_auth_key: Option<String>,
}

impl AppState {
    /// 空状态（engine 未配置）。
    pub fn empty() -> Self {
        Self {
            engine: None,
            image_engine: None,
            media_fetcher: None,
            responses_backend: None,
            messages_backend: None,
            video_backend: None,
            gateway_auth_key: None,
        }
    }

    /// 设置 `/v1` 写操作鉴权密钥（可选；空字符串视为未配置）。
    pub fn with_gateway_auth_key(mut self, key: Option<String>) -> Self {
        self.gateway_auth_key = key.filter(|k| !k.trim().is_empty());
        self
    }
}

/// 构建带 chat engine 的应用状态（image engine 未配置）。
pub fn with_engine(engine: ChatEngine) -> AppState {
    AppState {
        engine: Some(Arc::new(engine)),
        image_engine: None,
        media_fetcher: None,
        responses_backend: None,
        messages_backend: None,
        video_backend: None,
        gateway_auth_key: None,
    }
}

/// 构建带 chat + image 引擎的应用状态。
pub fn with_engines(engine: ChatEngine, image_engine: ImageEngine) -> AppState {
    AppState {
        engine: Some(Arc::new(engine)),
        image_engine: Some(Arc::new(image_engine)),
        media_fetcher: None,
        responses_backend: None,
        messages_backend: None,
        video_backend: None,
        gateway_auth_key: None,
    }
}

/// 构建带 chat + image 引擎 + 媒体取回器的应用状态（测试注入）。
pub fn with_engines_and_media(
    engine: ChatEngine,
    image_engine: ImageEngine,
    media_fetcher: Arc<dyn MediaFetcher>,
) -> AppState {
    AppState {
        engine: Some(Arc::new(engine)),
        image_engine: Some(Arc::new(image_engine)),
        media_fetcher: Some(media_fetcher),
        responses_backend: None,
        messages_backend: None,
        video_backend: None,
        gateway_auth_key: None,
    }
}

/// 构建带协议后端（G5-P3）的应用状态：同一后端同时服务 /v1/responses 与 /v1/messages。
/// 测试注入 fake；生产用 [`with_default_protocol_backends`]。
pub fn with_protocol_backend(backend: Arc<dyn ProtocolBackend>) -> AppState {
    AppState {
        engine: None,
        image_engine: None,
        media_fetcher: None,
        responses_backend: Some(backend.clone()),
        messages_backend: Some(backend),
        video_backend: None,
        gateway_auth_key: None,
    }
}

/// 构建带两个独立协议后端（/v1/responses → Build；/v1/messages → Console）的状态。
pub fn with_protocol_backends(
    responses: Option<Arc<dyn ProtocolBackend>>,
    messages: Option<Arc<dyn ProtocolBackend>>,
) -> AppState {
    AppState {
        engine: None,
        image_engine: None,
        media_fetcher: None,
        responses_backend: responses,
        messages_backend: messages,
        video_backend: None,
        gateway_auth_key: None,
    }
}

/// 默认真实协议后端：Build（/v1/responses）与 Console（/v1/messages），
/// 各用 `base_url` 可覆盖（测试指 mock server；None 走各自 default_base_url）。
pub fn with_default_protocol_backends(
    build_base_url: Option<String>,
    console_base_url: Option<String>,
) -> AppState {
    let (responses, messages) =
        crate::backends::default_protocol_backends(build_base_url, console_base_url);
    with_protocol_backends(responses, messages)
}

/// 构建带视频后端（G5-P4）的应用状态。
pub fn with_video_backend(backend: Arc<dyn VideoBackend>) -> AppState {
    AppState {
        engine: None,
        image_engine: None,
        media_fetcher: None,
        responses_backend: None,
        messages_backend: None,
        video_backend: Some(backend),
        gateway_auth_key: None,
    }
}

/// 构建 router（G1 端点 + G2 生图端点）。
pub fn build_app(state: AppState) -> Router {
    let shared = Arc::new(state);
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/images/generations", post(image_generations))
        .route("/v1/media/images/{id}", get(media_images))
        .route("/v1/responses", post(responses_completions))
        .route("/v1/messages", post(messages_completions))
        .route("/v1/videos", post(create_video))
        .route("/v1/videos/{id}", get(get_video))
        .layer(middleware::from_fn_with_state(
            shared.clone(),
            require_gateway_auth,
        ))
        .with_state(shared)
}

/// `/v1` 写操作鉴权中间件（对齐 :8014 `GATEWAY_AUTH_KEY` 语义）。
///
/// - `gateway_auth_key` 未配置（None）→ 放行（grok2api-rs 启动时告警）。
/// - 配置后：仅校验非 GET/HEAD/OPTIONS 请求（`/v1/models`、`/v1/media/images/{id}` 等
///   读接口保持开放，供 `<img>` 标签等无头场景使用）；通过 `Authorization: Bearer <key>`
///   或 `X-API-Key: <key>` 校验，失败返回结构化 401 JSON。
/// - 密钥比较为常数时间，避免时序侧信道。
async fn require_gateway_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = state.gateway_auth_key.as_deref() else {
        return next.run(req).await;
    };
    let method = req.method();
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return next.run(req).await;
    }
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| req.headers().get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    match provided {
        Some(token) if constant_time_eq(token, expected) => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response(),
    }
}

/// 常数时间字符串比较（对齐 :8014 `auth_routes.rs` 的 `constant_time_eq`）。
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// 测试便利：直接构一个空 state 的 app（无 engine，仅测路由形状）。
pub fn build_app_empty() -> Router {
    build_app(AppState::empty())
}
