//! grok-provider-web — Grok Web 推理（chat / OCR）Provider（G1）。
//!
//! 端口自 Go `infra/provider/web/*.go`（docs/39d §4.1）。G1 覆盖 OCR / chat 主路径：
//! - [`bridge`]：browser-bridge 客户端（下载图 / chat 转发）
//! - [`attachments`]：`prepareChatAttachments`（图 → FileAttachment）
//! - [`chat`]：payload 构造（OCR 别名 `grok-vision-ocr` → `grok-chat-fast` + 禁生图）
//! - [`engine`]：ChatEngine 编排放行（pool→lease→payload→bridge→文本）
//!
//! **不做**：text-to-image（G2）、quota 刷额度（G3）、statsig（G3 开关留 TODO）。
//! 完整 SSE 流式与多模型命名空间见 G3/G5。

pub mod attachments;
pub mod bridge;
pub mod chat;
pub mod direct;
pub mod engine;
pub mod expand;
pub mod image;
pub mod proxy;
pub mod statsig;

pub use attachments::{prepare_file_attachments, FileAttachment};
pub use bridge::{default_bridge_url, BridgeClient, HttpBridgeClient, MockBridgeClient};
pub use chat::{
    build_web_chat_payload, public_models, ALIAS_OCR, DEFAULT_OCR_SYSTEM_PROMPT, UPSTREAM_OCR_MODEL,
};
pub use direct::{DirectConfig, HttpDirectClient};
pub use engine::ChatEngine;
pub use expand::expand_prompt;
pub use image::ImageEngine;
pub use statsig::{validate_signer_url, StatsigSigner};
// 契约类型/端口在 grok-domain（跨 crate 共享），此处 re-export 保持旧调用路径。
pub use grok_domain::{
    ChatBackend, ChatRequest, ImageBackend, ImagineRequest, ImagineResult, ProviderError,
};
