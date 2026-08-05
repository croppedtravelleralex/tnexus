//! grok-provider-web 错误模型。
//!
//! G1 层级约定（docs/39c §1）：协议校验错误（conversation）→ HTTP 400，
//! pool 选号失败/lease 超时 → 上游或资源错误（gateway 侧转 502/429），
//! bridge 调用失败 → 上游错误。本 crate 只表达错误种类，HTTP 映射归 gateway。

use thiserror::Error;

/// Provider Web 错误。
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
    Lease(grok_egress::Error),

    /// 调用 browser-bridge 失败（下载图 / chat fetch）。
    #[error("browser-bridge error: {0}")]
    Bridge(String),

    /// 上游 chat 返回非成功或不可解析。
    #[error("upstream chat error: {0}")]
    Upstream(String),
}

impl From<grok_egress::Error> for ProviderError {
    fn from(e: grok_egress::Error) -> Self {
        ProviderError::Lease(e)
    }
}
