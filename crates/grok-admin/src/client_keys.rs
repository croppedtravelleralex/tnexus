//! 客户端密钥域（对齐 Go `transport/http/clientkey`）。
//!
//! 数据源为 client key 表（SQL 实现由 grok-storage 提供，TODO）；内存 fake 可测。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AdminError, AdminResult};

/// 客户端密钥（对齐 Go `clientKeyResponse` 子集；不返回明文 secret）。
#[derive(Debug, Clone, Serialize)]
pub struct ClientKeyView {
    pub id: i64,
    pub name: String,
    /// 密钥前 8 位（识别用；不含完整 secret）。
    pub prefix: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// 客户端密钥创建/更新输入。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientKeyInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// 客户端密钥存储抽象。
#[async_trait]
pub trait ClientKeyStore: Send + Sync {
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ClientKeyView>>;
    /// 创建并返回视图；`created_secret` 为一次性明文（创建响应返回给调用方）。
    async fn create(&self, input: &ClientKeyInput) -> AdminResult<(ClientKeyView, String)>;
    async fn update(&self, id: i64, input: &ClientKeyInput) -> AdminResult<Option<ClientKeyView>>;
    async fn delete(&self, id: i64) -> AdminResult<bool>;
}

/// 客户端密钥服务。
pub struct ClientKeyAdminService {
    store: std::sync::Arc<dyn ClientKeyStore>,
}

impl ClientKeyAdminService {
    pub fn new(store: std::sync::Arc<dyn ClientKeyStore>) -> Self {
        Self { store }
    }

    pub async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ClientKeyView>> {
        self.store.list(page.max(1), page_size.clamp(1, 100)).await
    }

    pub async fn create(&self, input: &ClientKeyInput) -> AdminResult<(ClientKeyView, String)> {
        if input.name.trim().is_empty() {
            return Err(AdminError::InvalidRequest("name 不能为空".into()));
        }
        self.store.create(input).await
    }

    pub async fn update(&self, id: i64, input: &ClientKeyInput) -> AdminResult<ClientKeyView> {
        self.store
            .update(id, input)
            .await?
            .ok_or_else(|| AdminError::NotFound(format!("client key {id}")))
    }

    pub async fn delete(&self, id: i64) -> AdminResult<()> {
        let deleted = self.store.delete(id).await?;
        if !deleted {
            return Err(AdminError::NotFound(format!("client key {id}")));
        }
        Ok(())
    }
}
