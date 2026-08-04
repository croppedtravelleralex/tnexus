//! Chrome 票池领域类型，对应 Go `domain/chrometicket` 与 `grok_chrome_tickets` 表。

use serde::{Deserialize, Serialize};

/// 票状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Available,
    Leased,
    Expired,
}

/// Chrome 票据（骨架）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromeTicket {
    pub id: i64,
    pub account_id: i64,
    pub status: TicketStatus,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
