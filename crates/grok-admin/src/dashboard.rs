//! 仪表盘聚合端点（对齐 Go `transport/http/dashboard`，G6 首页必需）。
//!
//! 真净值由 grok-storage 聚合；本模块抽象 [`DashboardStore`] + 序列化视图，
//! 测试注入内存 fake。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AdminResult;

/// 仪表盘聚合视图（对齐 Go `dashboardResponse`）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct DashboardView {
    /// 账号总数。
    pub total_accounts: i64,
    /// 可用（enabled 且 Active）。
    pub available_accounts: i64,
    /// 冷却中。
    pub cooldown_accounts: i64,
    /// 需重登。
    pub reauth_accounts: i64,
    /// 额度已耗尽的账号（窗口 remaining<=0 且 total>0）。
    pub quota_exhausted_accounts: i64,
    /// 近 24h 请求数。
    pub requests_24h: i64,
    /// 近 24h 请求成功率（0.0–1.0）。
    pub success_rate_24h: f64,
    /// 模型路由数。
    pub model_routes: i64,
    /// 活跃客户端密钥数。
    pub active_client_keys: i64,
    /// 最近一次请求时间（无则 None）。
    pub last_request_at: Option<DateTime<Utc>>,
}

/// 仪表盘数据源。
#[async_trait]
pub trait DashboardStore: Send + Sync {
    async fn view(&self) -> AdminResult<DashboardView>;
}

/// 仪表盘端点服务（校验 + 组装）。
pub struct DashboardService {
    store: std::sync::Arc<dyn DashboardStore>,
}

impl DashboardService {
    pub fn new(store: std::sync::Arc<dyn DashboardStore>) -> Self {
        Self { store }
    }

    pub async fn view(&self) -> AdminResult<DashboardView> {
        self.store.view().await
    }
}

/// 便捷当前时间（测试注入 / 序列化用）。
pub fn utc_now() -> DateTime<Utc> {
    Utc::now()
}