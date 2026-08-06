//! 错误类型（对齐 Python bridge 的 400/502 语义）。

use std::fmt;

/// bridge 内部错误 → HTTP 502（客户端协议错误 → 400 在 handler 侧直接返回）。
#[derive(Debug)]
pub enum BridgeError {
    /// 上游浏览器操作失败（超时 / JS 抛错 / 会话关闭）。
    Upstream(String),
    /// 请求参数不合法（400）。
    Invalid(String),
    /// 内部初始化失败（Chrome 拉起 / CDP 连接）。
    Internal(String),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Upstream(s) | BridgeError::Invalid(s) | BridgeError::Internal(s) => {
                write!(f, "{s}")
            }
        }
    }
}

impl std::error::Error for BridgeError {}

impl BridgeError {
    pub fn upstream(msg: impl Into<String>) -> Self {
        BridgeError::Upstream(msg.into())
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        BridgeError::Invalid(msg.into())
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        BridgeError::Internal(msg.into())
    }
}
