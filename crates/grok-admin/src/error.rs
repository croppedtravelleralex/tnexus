//! 认证错误（对齐 Go `adminauth` 包的错误变量）。
use thiserror::Error;

/// Admin 认证错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdminError {
    /// 管理员账号或密码错误。
    #[error("管理员账号或密码错误")]
    InvalidCredentials,
    /// 管理员会话无效。
    #[error("管理员会话无效")]
    InvalidSession,
    /// 首次启动需要设置管理员账号和密码。
    #[error("首次启动需要设置管理员账号和密码")]
    BootstrapRequired,
    /// 新密码至少需要 8 个字符。
    #[error("新密码至少需要 8 个字符")]
    InvalidPassword,
    /// 管理员登录尝试过于频繁。
    #[error("管理员登录尝试过于频繁")]
    LoginRateLimited,
    /// 管理员认证运行态暂不可用。
    #[error("管理员认证运行态暂不可用: {0}")]
    RuntimeUnavailable(String),
    /// JWT 签发 / 校验内部错误。
    #[error("令牌服务错误: {0}")]
    Token(String),
    /// 密码哈希内部错误。
    #[error("密码哈希错误: {0}")]
    Password(String),
    /// 令牌随机数生成失败。
    #[error("随机令牌生成失败: {0}")]
    Random(String),
    /// 资源不存在（账号 / 额度窗口）。
    #[error("资源不存在: {0}")]
    NotFound(String),
    /// 列表筛选参数无效。
    #[error("筛选参数无效: {0}")]
    InvalidFilter(String),
    /// 请求参数无效。
    #[error("请求参数无效: {0}")]
    InvalidRequest(String),
}

pub type AdminResult<T> = Result<T, AdminError>;