//! Chrome 票据管理域（对齐 Go `transport/http` 的 chrome-tickets 三端点）。
//!
//! 底层 `grok-chrome-ticket` crate 已有 pool；本模块抽象 [`ChromeTicketStore`]，
//! 让 grok-admin 不引入对该 crate 的编译依赖（由接线层把 pool 包装成 store；TODO）。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AdminResult;

/// 单票视图（对齐 Go `chromeTicketResponse` 子集）。
#[derive(Debug, Clone, Serialize)]
pub struct ChromeTicketView {
    pub account_id: i64,
    pub name: String,
    /// 票内容摘要（ID 末段，避免整票）。
    pub ticket_id_preview: String,
    /// 借出时刻（空闲为 None）。
    pub borrowed_at: Option<DateTime<Utc>>,
    /// 到期时刻（无效票有）。
    pub expires_at: Option<DateTime<Utc>>,
}

/// 票池统计。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChromeTicketStats {
    pub total: i64,
    pub available: i64,
    pub borrowed: i64,
    pub expired: i64,
}

/// 票据存储抽象（桥接 grok-chrome-ticket pool）。
#[async_trait]
pub trait ChromeTicketStore: Send + Sync {
    async fn list(&self) -> AdminResult<Vec<ChromeTicketView>>;
    async fn stats(&self) -> AdminResult<ChromeTicketStats>;
    /// 清理到期/失效票据，返回清理条数。
    async fn sweep(&self) -> AdminResult<i64>;
}

/// 票据域服务。
pub struct ChromeTicketService {
    store: std::sync::Arc<dyn ChromeTicketStore>,
}

impl ChromeTicketService {
    pub fn new(store: std::sync::Arc<dyn ChromeTicketStore>) -> Self {
        Self { store }
    }

    pub async fn list(&self) -> AdminResult<Vec<ChromeTicketView>> {
        self.store.list().await
    }

    pub async fn stats(&self) -> AdminResult<ChromeTicketStats> {
        self.store.stats().await
    }

    pub async fn sweep(&self) -> AdminResult<i64> {
        self.store.sweep().await
    }
}