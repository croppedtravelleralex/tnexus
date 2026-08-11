//! Provider 契约层：对话/生图请求-响应数据、上游错误模型与引擎端口。
//!
//! 本模块是 grok 子系统「端口」（Ports）所在：`ChatBackend` / `ImageBackend`
//! 定义网关对推理引擎的全部依赖，类型均为纯数据（无 crate 级依赖），
//! 使 `grok-gateway` 不再反向依赖 `grok-provider-web` 的具体引擎实现
//! （低耦合高内聚：依赖方向 domain ← provider / gateway，无反向边）。

use serde::Serialize;
use thiserror::Error;

/// OCR 对外别名（`grok-vision-ocr` → 上游 `grok-chat-fast` + 禁生图）。
pub const ALIAS_OCR: &str = "grok-vision-ocr";
/// 别名内部映射到的上游模型（web/catalog.go grok-chat-fast）。
pub const UPSTREAM_OCR_MODEL: &str = "grok-chat-fast";
/// OCR 默认 system prompt（可配置，39 主文档 §4.2）。
pub const DEFAULT_OCR_SYSTEM_PROMPT: &str =
    "提取图中全部可见文字，保持版面顺序；无文字则回复「无文字内容」。";

/// 公开的对外模型路由（含 G1 OCR 别名）。
pub fn public_models() -> Vec<(&'static str, &'static str)> {
    vec![("grok-chat", "grok-chat"), (ALIAS_OCR, UPSTREAM_OCR_MODEL)]
}

/// 对话请求（网关归一化后的输入，`grok-provider-web::engine::ChatEngine` 消费）。
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// 归一化 prompt（`[role]\ntext` 段落拼接）。
    pub prompt: String,
    /// 归一化图片清单（HTTPS URL 或 data URI）。
    pub images: Vec<String>,
    /// OCR 路径（`grok-vision-ocr`）→ 强制禁生图 + fast 模型。
    pub ocr: bool,
    /// OCR system prompt 覆盖；None 用默认。
    pub system_prompt: Option<String>,
    /// 审计请求 ID。
    pub request_id: String,
}

/// 生图请求（`grok-provider-web::image::ImageEngine` 消费）。
#[derive(Debug, Clone)]
pub struct ImagineRequest {
    /// 提示词。
    pub prompt: String,
    /// 生图数量（默认 1）。
    pub n: usize,
    /// 输出格式：`url` 或 `b64_json`。
    pub response_format: String,
    /// 是否走 `imagine-lite`。
    pub lite: bool,
    /// 是否先扩写提示词（prompt_enhance）。
    pub enhance: bool,
    /// 审计请求 ID。
    pub request_id: String,
    /// 画幅比例（如 `1:1`、`16:9`）；空则默认 `1:1`。
    pub aspect_ratio: String,
}

impl Default for ImagineRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            n: 1,
            response_format: "url".to_string(),
            lite: false,
            enhance: false,
            request_id: String::new(),
            aspect_ratio: "1:1".to_string(),
        }
    }
}

/// 生图结果（上游数据）。
#[derive(Debug, Clone)]
pub struct ImagineResult {
    /// 每张图的 URL 或 b64。
    pub items: Vec<String>,
    /// 是否 b64 输出。
    pub b64: bool,
}

/// Provider Web 错误（跨 crate 契约；egress 错误以字符串内联，避免 domain 反向依赖）。
#[derive(Debug, Error)]
pub enum ProviderError {
    /// 请求在协议层无效（空消息 / 图片超限 / file_id 等），应映射 HTTP 400。
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// 号池没有可用账号（空池或全部冷却）。
    #[error("no available grok_web account in pool")]
    NoAvailableAccount,

    /// 未能在 lease 时限内获得 egress 并发槽位。gateway 应映射 429/502。
    #[error("failed to acquire egress lease: {0}")]
    Lease(String),

    /// 调用 browser-bridge 失败（下载图 / chat fetch）。
    #[error("browser-bridge error: {0}")]
    Bridge(String),

    /// 依赖未就绪（如本地签名 bundle 缺失）：应映射 503 且不外呼。
    #[error("not configured: {0}")]
    NotConfigured(String),

    /// 上游 chat 返回非成功或不可解析。
    #[error("upstream chat error: {0}")]
    Upstream(String),
}

/// 对话推理结果（含调度账号，供网关透传）。
#[derive(Debug, Clone)]
pub struct ChatOutcome {
    pub text: String,
    pub account_id: Option<i64>,
}

/// 对话推理端口：网关对 chat 引擎的全部依赖。
#[async_trait::async_trait]
pub trait ChatBackend: Send + Sync {
    /// 执行一次对话推理，返回最终文本。
    async fn chat(&self, req: &ChatRequest) -> Result<String, ProviderError> {
        Ok(self.chat_outcome(req).await?.text)
    }

    /// 执行一次对话推理，返回文本与调度账号 id。
    async fn chat_outcome(&self, req: &ChatRequest) -> Result<ChatOutcome, ProviderError>;
}

/// 生图端口：网关对 image 引擎的全部依赖。
#[async_trait::async_trait]
pub trait ImageBackend: Send + Sync {
    /// 执行一次生图，返回图片清单。
    async fn imagine(&self, req: &ImagineRequest) -> Result<ImagineResult, ProviderError>;
}

/// 账号 sso token 提供者（无 chrome 直连路径用）：按账号取解密后的 sso token。
/// 实现方负责凭据解密（grok-storage `PgSsoTokenProvider`）与密钥治理。
#[async_trait::async_trait]
pub trait SsoTokenProvider: Send + Sync {
    /// 返回账号 sso token（`Cookie: sso=<token>; sso-rw=<token>`）。
    async fn sso_token(&self, account_id: i64) -> Result<String, ProviderError>;
}

/// 便于 domain 用户直接引用序列化（如 `ImagineResult` 测试断言）。
#[allow(unused)]
fn _assert_serializable(v: &impl Serialize) {}
