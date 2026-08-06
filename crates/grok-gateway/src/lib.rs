//! grok-gateway — Grok 推理网关 HTTP 层（docs/39d §2.1，docs/39 主文档 §6.1 P0/P1）。
//!
//! G1 面向 OCR + chat 最小闭环：
//! - [`handlers`]：`POST /v1/chat/completions`（含识图）、`GET /v1/models`
//! - [`router`]：axum `Router` + [`AppState`]，便于测试注入 mock engine
//! - [`error`]：[`GatewayError`] → OpenAI 风格错误 + 状态码映射
//!
//! 依赖：上层组装（`grok2api-rs`）把 [`grok_provider_web::ChatEngine`] 以
//! `engine` 注入状态。本 crate **不**依赖 `grok2api-rs`，避免循环（39d §1 顶层入口
//! 归 `grok2api-rs`，gateway 只做 handler/router，不含 main/config）。

pub mod backends;
pub mod error;
pub mod handlers;
pub mod protocol;
pub mod router;
pub mod video;

pub use backends::{default_protocol_backends, BuildResponsesBackend, ConsoleMessagesBackend};
pub use handlers::ProtocolBackend;
pub use error::GatewayError;
pub use router::{
    build_app, with_default_protocol_backends, with_engine, with_engines, with_engines_and_media,
    with_protocol_backend, with_protocol_backends, with_video_backend, AppState,
};
