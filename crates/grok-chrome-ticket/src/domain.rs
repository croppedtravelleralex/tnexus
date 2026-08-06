//! Chrome 票领域类型（对齐 Go `domain/chrometicket/ticket.go`）。

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

/// 可用票状态（Go `StatusAvailable`）。
pub const STATUS_AVAILABLE: &str = "available";
/// 已消费票状态（Go `StatusConsumed`）。
pub const STATUS_CONSUMED: &str = "consumed";
/// 已过期票状态（Go `StatusExpired`）。
pub const STATUS_EXPIRED: &str = "expired";

/// 默认票有效期（Go `defaultTicketTTL`）。
pub const DEFAULT_TICKET_TTL: Duration = Duration::hours(12);

/// 本机 Chrome 预捕获的长效 statsig_meta 资产（Go `Ticket`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub id: String,
    pub account_id: i64,
    pub statsig_meta: String,
    pub device_cookie: String,
    pub user_agent: String,
    pub sign_source: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub status: String,
}

/// 入池请求载荷（Go `PushInput`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushInput {
    pub account_id: i64,
    pub statsig_meta: String,
    pub device_cookie: String,
    pub user_agent: String,
    pub sign_source: String,
    /// <= 0 时由 Pool 按默认 TTL（12h）兜底。
    pub ttl: Duration,
}

/// 票池状态汇总（Go `Stats`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub by_status: HashMap<String, i64>,
    pub available_by_account: Vec<AccountCount>,
    pub available_tickets: Vec<TicketSummary>,
    pub ttl_distribution: HashMap<String, i64>,
    pub earliest_expires_at: Option<DateTime<Utc>>,
    pub earliest_expires_in_sec: i64,
}

/// 单个账号的可用票数（Go `AccountCount`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountCount {
    pub account_id: i64,
    pub count: i64,
}

/// 管理端展示的可用票摘要（不含 meta 正文，Go `TicketSummary`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketSummary {
    pub id: String,
    pub account_id: i64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub ttl_remaining_seconds: i64,
    pub sign_source: String,
}

/// 剩余 TTL 分桶（Go `ttlBucket`）。
pub fn ttl_bucket(remaining_seconds: i64) -> &'static str {
    match remaining_seconds {
        ..3600 => "<1h",
        3600..10800 => "1-3h",
        10800..21600 => "3-6h",
        21600..43200 => "6-12h",
        _ => ">12h",
    }
}
