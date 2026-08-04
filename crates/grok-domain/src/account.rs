//! 账号 / Provider / 额度领域类型（骨架）。
//! 对照 Go `domain/account`；字段后续按 docs/39b 补齐。

use serde::{Deserialize, Serialize};

/// Provider 类型，对应 `grok_accounts.provider` CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    GrokBuild,
    GrokWeb,
    GrokConsole,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::GrokBuild => "grok_build",
            Provider::GrokWeb => "grok_web",
            Provider::GrokConsole => "grok_console",
        }
    }
}

/// 账号认证状态，对应 `grok_accounts.auth_status`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    Unknown,
    Active,
    Restricted,
    Banned,
}

/// 账号主表领域模型（G0 最小字段，见 docs/39b §3 表 3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub identity_key: String,
    pub provider: Provider,
    pub enabled: bool,
    pub auth_status: AuthStatus,
    pub priority: i32,
    pub observed_model: Option<String>,
}

/// 账号额度窗口（fast/auto/imagine），对应 `grok_quota_windows`（骨架）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub account_id: i64,
    pub remaining: i64,
    pub reset_at: chrono::DateTime<chrono::Utc>,
}

/// 配额扣减结果（G1 fast 额度验收用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaDeduction {
    pub account_id: i64,
    pub remaining_after: i64,
}
