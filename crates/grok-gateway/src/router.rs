//! grok-gateway 路由与状态（axum）。
//!
//! G1 最小集：`GET /v1/models`、`POST /v1/chat/completions`。路由挂
//! [`AppState`]，内含按需构建的 [`ChatEngine`]。
//!
//! [`build_app`] 接受已组装 engine 便于测试注入 mock（tower `ServiceExt::oneshot`）。

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use grok_provider_web::ChatEngine;

use crate::handlers::{chat_completions, models};

/// 应用共享状态。
#[derive(Clone)]
pub struct AppState {
    /// 推理引擎。生产在 `with_engine` 注入；默认 None（未配置时推理请求 500）。
    pub engine: Option<Arc<ChatEngine>>,
}

impl AppState {
    /// 空状态（engine 未配置）。
    pub fn empty() -> Self {
        Self { engine: None }
    }
}

/// 构建带 engine 的应用状态。
pub fn with_engine(engine: ChatEngine) -> AppState {
    AppState {
        engine: Some(Arc::new(engine)),
    }
}

/// 构建 router（G1 端点）。
pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(Arc::new(state))
}

/// 测试便利：直接构一个空 state 的 app（无 engine，仅测路由形状）。
pub fn build_app_empty() -> Router {
    build_app(AppState::empty())
}
