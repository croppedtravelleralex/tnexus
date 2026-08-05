//! 推理请求审计记录类型。
//!
//! 对齐 Go `domain/audit.Record` 与 `grok_request_audits` 表列
//! （migrations/013_grok_inference.sql）。G1 只覆盖推理写路径所需字段；
//! 聚合/查询（`domain/audit` 的 `Summary`）属 G4 admin，不做。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 审计操作类型，对应 `grok_request_audits.operation` CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Responses,
    Chat,
    Messages,
    Image,
    ImageEdit,
    Video,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Responses => "responses",
            Operation::Chat => "chat",
            Operation::Messages => "messages",
            Operation::Image => "image",
            Operation::ImageEdit => "image_edit",
            Operation::Video => "video",
        }
    }
}

/// 用量来源，对应 `grok_request_audits.usage_source` CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Upstream,
    Estimated,
    None,
}

impl UsageSource {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageSource::Upstream => "upstream",
            UsageSource::Estimated => "estimated",
            UsageSource::None => "none",
        }
    }
}

/// 推理请求审计记录（不含提示/响应正文）。
///
/// 字段与 `grok_request_audits` 列一一对应。`Default` 提供满足 DB CHECK
/// 的安全默认值（status_code=200、provider=grok_web、operation=chat、
/// usage_source=none），调用方只需覆盖实际字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAudit {
    /// 事件去重 ID。空字符串表示不写去重键（`idx_grok_audits_event_id` partial
    /// unique 只在非空时生效）；非空须 16–64 长度。
    pub event_id: String,
    /// 请求 ID（1–64），必填。
    pub request_id: String,
    pub client_key_id: i64,
    pub client_key_name: Option<String>,
    pub model_route_id: i64,
    pub model_public_id: Option<String>,
    pub model_upstream_model: Option<String>,
    pub provider: String,
    pub operation: Operation,
    pub usage_source: UsageSource,
    pub account_id: Option<i64>,
    pub account_name: Option<String>,
    pub status_code: u32,
    pub streaming: bool,
    pub media_input_images: i64,
    pub media_output_images: i64,
    pub media_output_seconds: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub cost_in_usd_ticks: i64,
    pub estimated_cost_in_usd_ticks: i64,
    pub pricing_model: Option<String>,
    pub pricing_version: Option<String>,
    pub num_sources_used: i64,
    pub num_server_side_tools_used: i64,
    pub context_input_tokens: i64,
    pub context_output_tokens: i64,
    pub duration_ms: i64,
    pub error_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Default for CreateAudit {
    fn default() -> Self {
        Self {
            event_id: String::new(),
            request_id: String::new(),
            client_key_id: 1,
            client_key_name: None,
            model_route_id: 1,
            model_public_id: None,
            model_upstream_model: None,
            provider: "grok_web".to_string(),
            operation: Operation::Chat,
            usage_source: UsageSource::None,
            account_id: None,
            account_name: None,
            status_code: 200,
            streaming: false,
            media_input_images: 0,
            media_output_images: 0,
            media_output_seconds: 0,
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 0,
            cost_in_usd_ticks: 0,
            estimated_cost_in_usd_ticks: 0,
            pricing_model: None,
            pricing_version: None,
            num_sources_used: 0,
            num_server_side_tools_used: 0,
            context_input_tokens: 0,
            context_output_tokens: 0,
            duration_ms: 0,
            error_code: None,
            created_at: Utc::now(),
        }
    }
}

impl CreateAudit {
    /// 生成一个满足长度约束的 event_id（uuid v4，36 字符，落在 16–64 区间）。
    pub fn new_event_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
