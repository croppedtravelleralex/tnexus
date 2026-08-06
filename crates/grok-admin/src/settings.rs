//! 全局设置域（对齐 Go `transport/http/settings`；GET/PUT 版本化）。
//!
//! 写路径触发 settings_change_listener（grok-ops `SettingsWatcher` 已备，接线留 TODO）。
//! 内存 fake 可测。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::AdminResult;

/// 全局设置（对齐 Go `settingsResponse`；值统一字符串键值对）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SettingsView {
    /// 当前版本号（每次 PUT 递增）。
    pub version: i64,
    pub updated_at: DateTime<Utc>,
    pub values: BTreeMap<String, String>,
}

/// 设置写入输入（对齐 Go `updateSettingsRequest`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingsInput {
    pub values: BTreeMap<String, String>,
}

/// 设置存储抽象。
#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get(&self) -> AdminResult<SettingsView>;
    async fn put(&self, values: BTreeMap<String, String>) -> AdminResult<SettingsView>;
}

/// 设置域服务。
pub struct SettingsService {
    store: std::sync::Arc<dyn SettingsStore>,
}

impl SettingsService {
    pub fn new(store: std::sync::Arc<dyn SettingsStore>) -> Self {
        Self { store }
    }

    pub async fn get(&self) -> AdminResult<SettingsView> {
        self.store.get().await
    }

    /// 写回并递增版本（触发变更回调由调用方接线）。
    pub async fn put(&self, input: &SettingsInput) -> AdminResult<SettingsView> {
        self.store.put(input.values.clone()).await
    }
}