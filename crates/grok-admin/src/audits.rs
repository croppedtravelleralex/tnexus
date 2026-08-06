//! 请求审计域（对齐 Go `transport/http/audit`；数据源 `grok_request_audits`，
//! grok-audit 已写、缺读接口）。
//!
//! 内存 fake 可测；SQL 读实现由 grok-storage 提供（TODO）。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AdminResult;

/// 单条审计记录（对齐 Go `requestAuditResponse` 子集）。
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntryView {
    pub id: i64,
    pub account_id: Option<i64>,
    pub provider: Option<String>,
    pub upstream_model: Option<String>,
    pub status: i16,
    /// 本次请求状态（success / error）。
    pub outcome: String,
    /// 耗时毫秒。
    pub latency_ms: i64,
    pub created_at: DateTime<Utc>,
}

/// 审计汇总（对齐 Go `request-audits/summary`）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct AuditSummaryView {
    pub total: i64,
    pub requests_24h: i64,
    pub succeeded_24h: i64,
    pub failed_24h: i64,
    /// 近 24h 成功率（0.0–1.0）。
    pub success_rate_24h: f64,
}

/// 审计存储抽象（读侧）。
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// 分页列表（按时间倒序）。
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<AuditEntryView>>;
    async fn summary(&self) -> AdminResult<AuditSummaryView>;
}

/// 审计域服务。
pub struct AuditAdminService {
    store: std::sync::Arc<dyn AuditStore>,
}

impl AuditAdminService {
    pub fn new(store: std::sync::Arc<dyn AuditStore>) -> Self {
        Self { store }
    }

    pub async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<AuditEntryView>> {
        self.store.list(page.max(1), page_size.clamp(1, 100)).await
    }

    pub async fn summary(&self) -> AdminResult<AuditSummaryView> {
        self.store.summary().await
    }
}

// UTC 便捷（跨模块一致注入点）。
pub use crate::dashboard::utc_now;