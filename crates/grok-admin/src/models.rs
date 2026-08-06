//! 模型路由域（对齐 Go `transport/http/model`，数据源 `grok_model_routes` +
//! `grok_model_route_accounts`，012 migration）。
//!
//! 内存 fake 可测；SQL 实现由 grok-storage 提供（TODO 注记）。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AdminError, AdminResult};

/// 模型路由（对齐 Go `modelRouteResponse` 子集）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    pub id: i64,
    /// 上游 provider（grok_build / grok_web / grok_console）。
    pub provider: String,
    /// 上游模型名。
    pub upstream_model: String,
    /// 公开别名（可多个，逗号分隔）。
    pub aliases: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 模型创建/更新输入。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelRouteInput {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub upstream_model: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// 模型↔账号绑定视图（对齐 Go `/models/accounts`）。
#[derive(Debug, Clone, Serialize)]
pub struct ModelBindingView {
    pub model_route_id: i64,
    pub upstream_model: String,
    pub account_ids: Vec<i64>,
}

/// 模型存储抽象。
#[async_trait]
pub trait ModelStore: Send + Sync {
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ModelRoute>>;
    async fn get(&self, id: i64) -> AdminResult<Option<ModelRoute>>;
    async fn create(&self, input: &ModelRouteInput) -> AdminResult<ModelRoute>;
    async fn update(&self, id: i64, input: &ModelRouteInput) -> AdminResult<Option<ModelRoute>>;
    async fn delete(&self, id: i64) -> AdminResult<bool>;
    /// 模型↔账号绑定（对齐 Go `GetModelRouteAccounts`）。
    async fn bindings(&self) -> AdminResult<Vec<ModelBindingView>>;
}

/// 模型域服务。
pub struct ModelAdminService {
    store: std::sync::Arc<dyn ModelStore>,
}

impl ModelAdminService {
    pub fn new(store: std::sync::Arc<dyn ModelStore>) -> Self {
        Self { store }
    }

    pub async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ModelRoute>> {
        self.store.list(page.max(1), page_size.clamp(1, 100)).await
    }

    pub async fn create(&self, input: &ModelRouteInput) -> AdminResult<ModelRoute> {
        validate_route_input(input)?;
        self.store.create(input).await
    }

    pub async fn update(&self, id: i64, input: &ModelRouteInput) -> AdminResult<ModelRoute> {
        // PATCH 部分更新：仅校验 create 语义，update 允许只带 enabled 等字段。
        self.store
            .update(id, input)
            .await?
            .ok_or_else(|| AdminError::NotFound(format!("model route {id}")))
    }

    pub async fn delete(&self, id: i64) -> AdminResult<()> {
        let deleted = self.store.delete(id).await?;
        if !deleted {
            return Err(AdminError::NotFound(format!("model route {id}")));
        }
        Ok(())
    }

    pub async fn bindings(&self) -> AdminResult<Vec<ModelBindingView>> {
        self.store.bindings().await
    }
}

fn validate_route_input(input: &ModelRouteInput) -> AdminResult<()> {
    if input.provider.trim().is_empty() || input.upstream_model.trim().is_empty() {
        return Err(AdminError::InvalidRequest(
            "provider 与 upstream_model 不能为空".into(),
        ));
    }
    Ok(())
}