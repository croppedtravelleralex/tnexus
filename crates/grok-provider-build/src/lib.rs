//! grok-provider-build — Grok Build/CLI provider（G5-P1，Go `infra/provider/cli` 移植）。
//!
//! 最小闭环：**stored response 往返**（G5-A1）——构造 `POST /responses`
//! （`store:false, stream:false`），注入 Grok Build 协议头（x-grok-*），规整请求体
//! （模型覆盖 / response_format → text 映射 / prompt_cache_key 保留），解析存储响应
//! 文本。
//!
//! 边界（G5-P1 不做）：流式 SSE（G5-P3）、tools 兼容映射（namespace/shell/MCP）、
//! OAuth/设备授权、Billing、凭据导入、gzip 解压（reqwest 未开 gzip feature）。
//! 令牌为明文传入（Rust 侧暂无 Cipher 层）。

pub mod adapter;
pub mod error;
pub mod normalize;
pub mod response;

pub use adapter::{default_timeout, BuildAdapter, Config, ForwardRequest, ForwardResponse};
pub use error::ProviderError;
pub use normalize::{ensure_prompt_cache_key, normalize_responses_request};
pub use response::StoredResponse;

/// 默认上游基地址（Go 配置默认 `https://cli-chat-proxy.grok.com/v1`）。
pub fn default_base_url() -> String {
    std::env::var("GROK2API_BUILD_BASE_URL")
        .unwrap_or_else(|_| "https://cli-chat-proxy.grok.com/v1".to_string())
}
