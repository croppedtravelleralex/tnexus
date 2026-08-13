//! Admin 非账号域的内存实现（G4-A1 接线补齐）。
//!
//! [`AdminDomains`] 的 8 个域 service 在组装处全部挂上，端点从 503「域未接线」变 200：
//! - models / client-keys：内存可写（列表/增删改真可用；重启丢失）
//! - settings：内存键值（PUT 递增版本号）
//! - audits / dashboard / media / chrome-tickets：空数据 + 默认视图（真实数据源接 grok-storage/PG，TODO）
//! - system：纯本地（环形日志 + 配置视图），无 store
//!
//! ponytail: 内存持久化，域数据落 PG（grok_model_routes/grok_client_keys 等表）在
//! 写路径接入后替换；当前先保证管理台全端点可用。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use grok_admin::{
    AdminDomains, AdminResult, AuditAdminService, AuditEntryView, AuditStore, AuditSummaryView,
    ChromeTicketService, ChromeTicketStats, ChromeTicketStore, ChromeTicketView,
    ClientKeyAdminService, ClientKeyInput, ClientKeyStore, ClientKeyView, DashboardService,
    DashboardStore, DashboardView, ImageTimelineEntry, MediaImageView, MediaService,
    MediaSizeSummaryView, MediaStatsView, MediaStore, ModelAdminService, ModelAliasView,
    ModelBindingView, ModelRoute, ModelRouteInput, ModelStore, ModelSyncStateView, SettingsService,
    SettingsStore, SettingsView, SystemService,
};

// ── models 域 ───────────────────────────────────────────────────

/// 内存模型路由表。
#[derive(Default)]
pub struct InMemoryModelStore {
    routes: Mutex<Vec<ModelRoute>>,
    next_id: AtomicI64,
}

#[async_trait::async_trait]
impl ModelStore for InMemoryModelStore {
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ModelRoute>> {
        let routes = self.routes.lock().unwrap();
        let offset = ((page - 1) * page_size).max(0) as usize;
        Ok(routes
            .iter()
            .skip(offset)
            .take(page_size as usize)
            .cloned()
            .collect())
    }
    async fn get(&self, id: i64) -> AdminResult<Option<ModelRoute>> {
        Ok(self
            .routes
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned())
    }
    async fn create(&self, input: &ModelRouteInput) -> AdminResult<ModelRoute> {
        let mut routes = self.routes.lock().unwrap();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let now = Utc::now();
        let route = ModelRoute {
            id,
            provider: input.provider.clone(),
            upstream_model: input.upstream_model.clone(),
            aliases: input.aliases.clone(),
            enabled: input.enabled.unwrap_or(true),
            created_at: now,
            updated_at: now,
        };
        routes.push(route.clone());
        Ok(route)
    }
    async fn update(&self, id: i64, input: &ModelRouteInput) -> AdminResult<Option<ModelRoute>> {
        let mut routes = self.routes.lock().unwrap();
        let Some(route) = routes.iter_mut().find(|r| r.id == id) else {
            return Ok(None);
        };
        if !input.provider.is_empty() {
            route.provider = input.provider.clone();
        }
        if !input.upstream_model.is_empty() {
            route.upstream_model = input.upstream_model.clone();
        }
        if !input.aliases.is_empty() {
            route.aliases = input.aliases.clone();
        }
        if let Some(enabled) = input.enabled {
            route.enabled = enabled;
        }
        route.updated_at = Utc::now();
        Ok(Some(route.clone()))
    }
    async fn delete(&self, id: i64) -> AdminResult<bool> {
        let mut routes = self.routes.lock().unwrap();
        let before = routes.len();
        routes.retain(|r| r.id != id);
        Ok(routes.len() < before)
    }
    async fn bindings(&self) -> AdminResult<Vec<ModelBindingView>> {
        Ok(self
            .routes
            .lock()
            .unwrap()
            .iter()
            .map(|r| ModelBindingView {
                model_route_id: r.id,
                upstream_model: r.upstream_model.clone(),
                account_ids: vec![],
            })
            .collect())
    }
    async fn aliases(&self) -> AdminResult<Vec<ModelAliasView>> {
        Ok(self
            .routes
            .lock()
            .unwrap()
            .iter()
            .map(|r| ModelAliasView {
                upstream_model: r.upstream_model.clone(),
                aliases: r.aliases.clone(),
                enabled: r.enabled,
            })
            .collect())
    }
    async fn sync_states(&self) -> AdminResult<Vec<ModelSyncStateView>> {
        Ok(self
            .routes
            .lock()
            .unwrap()
            .iter()
            .map(|r| ModelSyncStateView {
                upstream_model: r.upstream_model.clone(),
                account_count: 0,
                sync_state: "unknown".into(),
            })
            .collect())
    }
}

// ── client-keys 域 ─────────────────────────────────────────────

/// 内存客户端密钥表（create 返回一次性明文 secret）。
#[derive(Default)]
pub struct InMemoryKeyStore {
    keys: Mutex<Vec<ClientKeyView>>,
    next_id: AtomicI64,
}

fn random_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[async_trait::async_trait]
impl ClientKeyStore for InMemoryKeyStore {
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ClientKeyView>> {
        let keys = self.keys.lock().unwrap();
        let offset = ((page - 1) * page_size).max(0) as usize;
        Ok(keys
            .iter()
            .skip(offset)
            .take(page_size as usize)
            .cloned()
            .collect())
    }
    async fn create(&self, input: &ClientKeyInput) -> AdminResult<(ClientKeyView, String)> {
        let mut keys = self.keys.lock().unwrap();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let secret = random_secret();
        let view = ClientKeyView {
            id,
            name: input.name.clone(),
            prefix: secret[..8].to_string(),
            enabled: input.enabled.unwrap_or(true),
            created_at: Utc::now(),
            last_used_at: None,
        };
        keys.push(view.clone());
        Ok((view, secret))
    }
    async fn update(&self, id: i64, input: &ClientKeyInput) -> AdminResult<Option<ClientKeyView>> {
        let mut keys = self.keys.lock().unwrap();
        let Some(key) = keys.iter_mut().find(|k| k.id == id) else {
            return Ok(None);
        };
        if !input.name.is_empty() {
            key.name = input.name.clone();
        }
        if let Some(enabled) = input.enabled {
            key.enabled = enabled;
        }
        Ok(Some(key.clone()))
    }
    async fn delete(&self, id: i64) -> AdminResult<bool> {
        let mut keys = self.keys.lock().unwrap();
        let before = keys.len();
        keys.retain(|k| k.id != id);
        Ok(keys.len() < before)
    }
}

// ── audits 域 ─────────────────────────────────────────────────

/// 内存审计（空；真实数据在 grok_request_audits，PG store TODO）。
#[derive(Default)]
pub struct InMemoryAuditStore;

#[async_trait::async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn list(
        &self,
        _page: i64,
        _page_size: i64,
    ) -> AdminResult<(Vec<AuditEntryView>, i64)> {
        Ok((vec![], 0))
    }
    async fn summary(&self) -> AdminResult<AuditSummaryView> {
        Ok(AuditSummaryView::default())
    }
}

// ── dashboard 域 ───────────────────────────────────────────────

/// 内存仪表盘（默认全 0；PG 聚合 TODO）。
#[derive(Default)]
pub struct InMemoryDashboardStore;

#[async_trait::async_trait]
impl DashboardStore for InMemoryDashboardStore {
    async fn view(&self) -> AdminResult<DashboardView> {
        Ok(DashboardView::default())
    }
}

// ── settings 域 ────────────────────────────────────────────────

/// 内存全局设置（PUT 递增版本号）。
#[derive(Default)]
pub struct InMemorySettingsStore {
    inner: Mutex<SettingsView>,
}

#[async_trait::async_trait]
impl SettingsStore for InMemorySettingsStore {
    async fn get(&self) -> AdminResult<SettingsView> {
        Ok(self.inner.lock().unwrap().clone())
    }
    async fn put(&self, values: BTreeMap<String, String>) -> AdminResult<SettingsView> {
        let mut inner = self.inner.lock().unwrap();
        inner.values = values;
        inner.version += 1;
        inner.updated_at = Utc::now();
        Ok(inner.clone())
    }
}

// ── chrome-tickets 域 ──────────────────────────────────────────

/// 内存票据（空；仅维持端点形状）。
#[derive(Default)]
pub struct InMemoryTicketStore;

#[async_trait::async_trait]
impl ChromeTicketStore for InMemoryTicketStore {
    async fn list(&self) -> AdminResult<Vec<ChromeTicketView>> {
        Ok(vec![])
    }
    async fn stats(&self) -> AdminResult<ChromeTicketStats> {
        Ok(ChromeTicketStats::default())
    }
    async fn sweep(&self) -> AdminResult<i64> {
        Ok(0)
    }
}

// ── media 域 ───────────────────────────────────────────────────

/// 内存媒体（空；真实数据在 grok_media_assets，PG store TODO）。
#[derive(Default)]
pub struct InMemoryMediaStore;

#[async_trait::async_trait]
impl MediaStore for InMemoryMediaStore {
    async fn list_images(&self, _page: i64, _page_size: i64) -> AdminResult<Vec<MediaImageView>> {
        Ok(vec![])
    }
    async fn media_stats(&self) -> AdminResult<MediaStatsView> {
        Ok(MediaStatsView::default())
    }
    async fn timeline(&self, _limit: usize) -> AdminResult<Vec<ImageTimelineEntry>> {
        Ok(vec![])
    }
    async fn get_image(&self, _asset_id: &str) -> AdminResult<Option<MediaImageView>> {
        Ok(None)
    }
    async fn size_summary(&self) -> AdminResult<MediaSizeSummaryView> {
        Ok(MediaSizeSummaryView {
            total_images: 0,
            total_bytes: 0,
            buckets: vec![],
        })
    }
}

/// 组装全部非账号域（models/client-keys 可写；只读域空数据；system 本地）。
pub fn build_admin_domains() -> AdminDomains {
    AdminDomains {
        models: Some(ModelAdminService::new(Arc::new(
            InMemoryModelStore::default(),
        ))),
        client_keys: Some(ClientKeyAdminService::new(Arc::new(
            InMemoryKeyStore::default(),
        ))),
        audits: Some(AuditAdminService::new(Arc::new(InMemoryAuditStore))),
        dashboard: Some(DashboardService::new(Arc::new(InMemoryDashboardStore))),
        settings: Some(SettingsService::new(Arc::new(
            InMemorySettingsStore::default(),
        ))),
        chrome_tickets: Some(ChromeTicketService::new(Arc::new(InMemoryTicketStore))),
        media: Some(MediaService::new(Arc::new(InMemoryMediaStore))),
        system: Some(SystemService::new()),
    }
}
