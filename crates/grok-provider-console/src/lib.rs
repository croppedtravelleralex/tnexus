//! grok-provider-console — Grok Console provider（G5-A2/A3）。
//!
//! 最小闭环：**chat completions 流式往返**——`POST /v1/chat/completions`
//! （`stream:true`），增量解析上游 SSE（`data:` 行 + `[DONE]`），归一化为 OpenAI
//! 兼容分片（`choices[0].delta.content` / role / finish_reason；responses 风格
//! `response.output_text.delta` 映射到 content）。
//!
//! 边界（后续）：/v1/responses 协议（Go 侧 console 主协议）、egress 租约与节点选择、
//! 凭据加密/导入、模型目录与别名、gzip（reqwest 未开 gzip feature）。令牌明文传入。

pub mod adapter;
pub mod error;
pub mod normalize;
pub mod sse;

pub use adapter::{Config, ConsoleAdapter};
pub use error::{ProviderError, UpstreamError};
pub use normalize::build_chat_request;
pub use sse::{ChatDelta, SseEvent, SseParser, parse_chat_delta};

/// 默认上游基地址（Go 配置默认 `https://console.x.ai`）。
pub fn default_base_url() -> String {
    std::env::var("GROK2API_CONSOLE_BASE_URL")
        .unwrap_or_else(|_| "https://console.x.ai".to_string())
}
