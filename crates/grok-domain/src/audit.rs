//! 审计领域类型，对应 Go `domain/audit` 与 `grok_request_audits` 表。

use serde::{Deserialize, Serialize};

/// 请求审计事件类型（G1 起异步写入 `grok_request_audits`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    ChatCompletion,
    ImageGeneration,
    AdminAction,
}

/// 请求审计记录（骨架）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: i64,
    pub client_key: String,
    pub model: String,
    pub event: AuditEvent,
    pub status_code: u16,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
