//! G4-A1 新域端点集成测试（dashboard/models/client-keys/audits/settings/chrome-tickets/media/system）。
//!
//! 覆盖：Bearer guard（无/坏 token → 401）、各端点正常路径 + 校验错误（400/404）。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use grok_admin::repos::{AdminRepository, AdminSessionRepository};
use grok_admin::{
    Admin, AdminAuthService, AdminDomains, AdminResult, AdminRouter, AuditAdminService,
    AuditEntryView, AuditStore, AuditSummaryView, ChromeTicketService, ChromeTicketStats,
    ChromeTicketStore, ChromeTicketView, ClientKeyAdminService, ClientKeyInput, ClientKeyStore,
    ClientKeyView, DashboardService, DashboardStore, DashboardView, MediaImageView, MediaService,
    MediaStatsView, MediaStore, ModelAdminService, ModelBindingView, ModelRoute, ModelRouteInput,
    ModelStore, Session, SettingsService, SettingsStore, SettingsView, SystemService, TokenService,
};
use grok_domain::{ModelState, Provider, QuotaWindow};

fn secret() -> String {
    "12345678901234567890123456789012".to_string()
}

// ── auth fakes（复用 admin_accounts.rs 模式）──────────────────────

#[derive(Default)]
struct AuthStore {
    admins: Mutex<Vec<Admin>>,
    sessions: Mutex<Vec<Session>>,
    next_admin_id: AtomicI64,
    next_session_id: AtomicI64,
}
struct FakeAdminRepo(Arc<AuthStore>);
struct FakeSessionRepo(Arc<AuthStore>);

#[async_trait::async_trait]
impl AdminRepository for FakeAdminRepo {
    async fn count(&self) -> AdminResult<i64> {
        Ok(self.0.admins.lock().unwrap().len() as i64)
    }
    async fn create(&self, mut admin: Admin) -> AdminResult<Admin> {
        admin.id = self.0.next_admin_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.admins.lock().unwrap().push(admin.clone());
        Ok(admin)
    }
    async fn get_by_username(&self, username: &str) -> AdminResult<Option<Admin>> {
        Ok(self
            .0
            .admins
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.username == username)
            .cloned())
    }
    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Admin>> {
        Ok(self
            .0
            .admins
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .cloned())
    }
    async fn update_password_and_revoke_sessions(
        &self,
        admin_id: i64,
        password_hash: &str,
    ) -> AdminResult<()> {
        if let Some(a) = self
            .0
            .admins
            .lock()
            .unwrap()
            .iter_mut()
            .find(|a| a.id == admin_id)
        {
            a.password_hash = password_hash.to_string();
        }
        self.0
            .sessions
            .lock()
            .unwrap()
            .retain(|s| s.admin_id != admin_id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl AdminSessionRepository for FakeSessionRepo {
    async fn get_by_token_hash(&self, _h: &str) -> AdminResult<Option<Session>> {
        Ok(None)
    }
    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Session>> {
        Ok(self
            .0
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .cloned())
    }
    async fn create(&self, mut session: Session) -> AdminResult<Session> {
        session.id = self.0.next_session_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.sessions.lock().unwrap().push(session.clone());
        Ok(session)
    }
    async fn rotate(&self, _id: i64, _o: &str, _n: &str, _e: DateTime<Utc>) -> AdminResult<bool> {
        Ok(true)
    }
    async fn revoke(&self, id: i64) -> AdminResult<()> {
        self.0.sessions.lock().unwrap().retain(|s| s.id != id);
        Ok(())
    }
}

// ── 各域内存 fake store ───────────────────────────────────────────

#[derive(Default)]
struct ModelStoreFake {
    routes: Mutex<Vec<ModelRoute>>,
    next_id: AtomicI64,
}
impl ModelStoreFake {
    fn seed(&self, provider: &str, model: &str) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.routes.lock().unwrap().push(ModelRoute {
            id,
            provider: provider.into(),
            upstream_model: model.into(),
            aliases: vec![model.into()],
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        id
    }
}
#[async_trait::async_trait]
impl ModelStore for ModelStoreFake {
    async fn list(&self, _page: i64, _n: i64) -> AdminResult<Vec<ModelRoute>> {
        Ok(self.routes.lock().unwrap().clone())
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
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let route = ModelRoute {
            id,
            provider: input.provider.clone(),
            upstream_model: input.upstream_model.clone(),
            aliases: input.aliases.clone(),
            enabled: input.enabled.unwrap_or(true),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.routes.lock().unwrap().push(route.clone());
        Ok(route)
    }
    async fn update(&self, id: i64, input: &ModelRouteInput) -> AdminResult<Option<ModelRoute>> {
        let mut routes = self.routes.lock().unwrap();
        let Some(route) = routes.iter_mut().find(|r| r.id == id) else {
            return Ok(None);
        };
        if !input.provider.trim().is_empty() {
            route.provider = input.provider.clone();
        }
        if !input.upstream_model.trim().is_empty() {
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
        Ok(routes.len() != before)
    }
    async fn aliases(&self) -> AdminResult<Vec<grok_admin::ModelAliasView>> {
        Ok(self
            .routes
            .lock()
            .unwrap()
            .iter()
            .map(|r| grok_admin::ModelAliasView {
                upstream_model: r.upstream_model.clone(),
                aliases: r.aliases.clone(),
                enabled: r.enabled,
            })
            .collect())
    }
    async fn sync_states(&self) -> AdminResult<Vec<grok_admin::ModelSyncStateView>> {
        let routes = self.routes.lock().unwrap();
        Ok(routes
            .iter()
            .map(|r| grok_admin::ModelSyncStateView {
                upstream_model: r.upstream_model.clone(),
                account_count: 1,
                sync_state: if r.enabled { "synced" } else { "unknown" }.into(),
            })
            .collect())
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
                account_ids: vec![1],
            })
            .collect())
    }
}

#[derive(Default)]
struct KeyStoreFake {
    keys: Mutex<Vec<(ClientKeyView, String)>>,
    next_id: AtomicI64,
}
#[async_trait::async_trait]
impl ClientKeyStore for KeyStoreFake {
    async fn list(&self, _p: i64, _n: i64) -> AdminResult<Vec<ClientKeyView>> {
        Ok(self
            .keys
            .lock()
            .unwrap()
            .iter()
            .map(|(v, _)| v.clone())
            .collect())
    }
    async fn create(&self, input: &ClientKeyInput) -> AdminResult<(ClientKeyView, String)> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let secret = format!("sk-{}", id);
        let view = ClientKeyView {
            id,
            name: input.name.clone(),
            prefix: secret.chars().take(8).collect(),
            enabled: input.enabled.unwrap_or(true),
            created_at: Utc::now(),
            last_used_at: None,
        };
        self.keys
            .lock()
            .unwrap()
            .push((view.clone(), secret.clone()));
        Ok((view, secret))
    }
    async fn update(&self, id: i64, input: &ClientKeyInput) -> AdminResult<Option<ClientKeyView>> {
        let mut keys = self.keys.lock().unwrap();
        let Some((view, _)) = keys.iter_mut().find(|(v, _)| v.id == id) else {
            return Ok(None);
        };
        if !input.name.trim().is_empty() {
            view.name = input.name.clone();
        }
        if let Some(enabled) = input.enabled {
            view.enabled = enabled;
        }
        Ok(Some(view.clone()))
    }
    async fn delete(&self, id: i64) -> AdminResult<bool> {
        let mut keys = self.keys.lock().unwrap();
        let before = keys.len();
        keys.retain(|(v, _)| v.id != id);
        Ok(keys.len() != before)
    }
}

#[derive(Default)]
struct AuditStoreFake {
    entries: Mutex<Vec<AuditEntryView>>,
}
impl AuditStoreFake {
    fn seed(&self, outcome: &str, ms: i64) {
        // 先取 len 再 push，避免同线程对 std Mutex 重入死锁。
        let mut entries = self.entries.lock().unwrap();
        let id = entries.len() as i64 + 1;
        entries.push(AuditEntryView {
            id,
            account_id: Some(1),
            provider: Some("grok_web".into()),
            upstream_model: Some("grok-3".into()),
            status: 200,
            outcome: outcome.into(),
            latency_ms: ms,
            created_at: Utc::now(),
        });
    }
}
#[async_trait::async_trait]
impl AuditStore for AuditStoreFake {
    async fn list(&self, _p: i64, _n: i64) -> AdminResult<Vec<AuditEntryView>> {
        let mut entries = self.entries.lock().unwrap().clone();
        entries.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        Ok(entries)
    }
    async fn summary(&self) -> AdminResult<AuditSummaryView> {
        let entries = self.entries.lock().unwrap();
        Ok(AuditSummaryView {
            total: entries.len() as i64,
            requests_24h: entries.len() as i64,
            succeeded_24h: entries.iter().filter(|e| e.outcome == "success").count() as i64,
            failed_24h: entries.iter().filter(|e| e.outcome == "error").count() as i64,
            success_rate_24h: if entries.is_empty() {
                0.0
            } else {
                entries.iter().filter(|e| e.outcome == "success").count() as f64
                    / entries.len() as f64
            },
        })
    }
}

#[derive(Default)]
struct DashboardFake;
#[async_trait::async_trait]
impl DashboardStore for DashboardFake {
    async fn view(&self) -> AdminResult<DashboardView> {
        // 与 setup() 的 3 个 seed 账号一致（2 active + 1 reauth 禁用）。
        Ok(DashboardView {
            total_accounts: 3,
            available_accounts: 2,
            cooldown_accounts: 0,
            reauth_accounts: 1,
            quota_exhausted_accounts: 0,
            requests_24h: 120,
            success_rate_24h: 0.95,
            model_routes: 1,
            active_client_keys: 0,
            last_request_at: Some(Utc::now()),
        })
    }
}

#[derive(Default)]
struct SettingsFake {
    version: Mutex<i64>,
    values: Mutex<std::collections::BTreeMap<String, String>>,
}
#[async_trait::async_trait]
impl SettingsStore for SettingsFake {
    async fn get(&self) -> AdminResult<SettingsView> {
        Ok(SettingsView {
            version: *self.version.lock().unwrap(),
            updated_at: Utc::now(),
            values: self.values.lock().unwrap().clone(),
        })
    }
    async fn put(
        &self,
        values: std::collections::BTreeMap<String, String>,
    ) -> AdminResult<SettingsView> {
        *self.version.lock().unwrap() += 1;
        *self.values.lock().unwrap() = values;
        self.get().await
    }
}

#[derive(Default)]
struct TicketFake {
    swept: Mutex<i64>,
}
#[async_trait::async_trait]
impl ChromeTicketStore for TicketFake {
    async fn list(&self) -> AdminResult<Vec<ChromeTicketView>> {
        Ok(vec![ChromeTicketView {
            account_id: 4,
            name: "web-4".into(),
            ticket_id_preview: "tk_ab12".into(),
            borrowed_at: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
        }])
    }
    async fn stats(&self) -> AdminResult<ChromeTicketStats> {
        Ok(ChromeTicketStats {
            total: 1,
            available: 1,
            borrowed: 0,
            expired: 0,
        })
    }
    async fn sweep(&self) -> AdminResult<i64> {
        let mut swept = self.swept.lock().unwrap();
        *swept += 1;
        Ok(*swept)
    }
}

#[derive(Default)]
struct MediaFake {
    images: Mutex<Vec<MediaImageView>>,
}
impl MediaFake {
    fn seed(&self, asset: &str) {
        self.images.lock().unwrap().push(MediaImageView {
            asset_id: asset.into(),
            account_id: 1,
            provider: Some("grok_web".into()),
            width: Some(1024),
            height: Some(1024),
            size_bytes: Some(12345),
            created_at: Utc::now(),
        });
    }
}
#[async_trait::async_trait]
impl MediaStore for MediaFake {
    async fn list_images(&self, _p: i64, _n: i64) -> AdminResult<Vec<MediaImageView>> {
        Ok(self.images.lock().unwrap().clone())
    }
    async fn media_stats(&self) -> AdminResult<MediaStatsView> {
        let images = self.images.lock().unwrap();
        Ok(MediaStatsView {
            total_images: images.len() as i64,
            total_bytes: images.iter().map(|i| i.size_bytes.unwrap_or(0)).sum(),
            recent_24h: images.len() as i64,
        })
    }
    async fn get_image(&self, asset_id: &str) -> AdminResult<Option<MediaImageView>> {
        Ok(self
            .images
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.asset_id == asset_id)
            .cloned())
    }
    async fn size_summary(&self) -> AdminResult<grok_admin::MediaSizeSummaryView> {
        let images = self.images.lock().unwrap();
        Ok(grok_admin::MediaSizeSummaryView {
            total_images: images.len() as i64,
            total_bytes: images.iter().map(|i| i.size_bytes.unwrap_or(0)).sum(),
            buckets: vec![],
        })
    }
    async fn timeline(&self, limit: usize) -> AdminResult<Vec<grok_admin::ImageTimelineEntry>> {
        let n = self.images.lock().unwrap().len().min(limit) as i64;
        Ok((0..n)
            .map(|i| grok_admin::ImageTimelineEntry {
                account_name: format!("acc-{i}"),
                provider: "grok_web".into(),
                upstream_model: "grok-imagine-image".into(),
                status: "completed".into(),
                latency_ms: 5000,
                created_at: Utc::now(),
            })
            .collect())
    }
}

// ── 账号 store（summary/analytics 需要；复用 admin_accounts 语义简版）──

fn build_account(
    id: i64,
    provider: Provider,
    enabled: bool,
    status: grok_domain::AuthStatus,
) -> grok_domain::Account {
    grok_domain::Account {
        id,
        identity_key: format!("key-{id}"),
        provider,
        name: format!("acc-{id}"),
        enabled,
        auth_status: status,
        priority: 10,
        max_concurrent: 4,
        ..Default::default()
    }
}

// ── fixture ───────────────────────────────────────────────────────

#[allow(clippy::type_complexity)]
async fn setup() -> (AdminRouter, String) {
    let auth_store = Arc::new(AuthStore::default());
    let auth = AdminAuthService::new(
        Arc::new(FakeAdminRepo(auth_store.clone())),
        Arc::new(FakeSessionRepo(auth_store)),
        TokenService::new(&secret()),
        Duration::hours(1),
        Duration::days(7),
    );
    auth.bootstrap("admin", "password123")
        .await
        .expect("bootstrap");
    let (_, tokens) = auth
        .login("admin", "password123", "127.0.0.1")
        .await
        .expect("login");

    // 账号 store（用 admin_accounts 简版：只有 summary/analytics 需要）
    let account_store = AccountStore::default();
    account_store.seed(build_account(
        1,
        Provider::GrokBuild,
        true,
        grok_domain::AuthStatus::Active,
    ));
    account_store.seed(build_account(
        2,
        Provider::GrokBuild,
        true,
        grok_domain::AuthStatus::Active,
    ));
    account_store.seed(build_account(
        3,
        Provider::GrokWeb,
        false,
        grok_domain::AuthStatus::ReauthRequired,
    ));

    let models = Arc::new(ModelStoreFake::default());
    models.seed("grok_web", "grok-3");
    let keys = Arc::new(KeyStoreFake::default());
    let audits = Arc::new(AuditStoreFake::default());
    audits.seed("success", 12);
    audits.seed("error", 99);
    let tickets = Arc::new(TicketFake::default());
    let media = Arc::new(MediaFake::default());
    media.seed("img_1");

    let domains = AdminDomains {
        models: Some(ModelAdminService::new(models.clone())),
        client_keys: Some(ClientKeyAdminService::new(keys.clone())),
        audits: Some(AuditAdminService::new(audits.clone())),
        dashboard: Some(DashboardService::new(Arc::new(DashboardFake))),
        settings: Some(SettingsService::new(Arc::new(SettingsFake::default()))),
        chrome_tickets: Some(ChromeTicketService::new(tickets.clone())),
        media: Some(MediaService::new(media.clone())),
        system: Some(SystemService::with_version("test")),
    };

    let router = AdminRouter::new(
        auth,
        grok_admin::AccountAdminService::new(Arc::new(account_store)),
    )
    .with_domains(domains);
    (router, tokens.access_token)
}

#[derive(Default)]
struct AccountStore {
    accounts: std::sync::Mutex<Vec<grok_domain::Account>>,
    next_id: AtomicI64,
}
impl AccountStore {
    fn seed(&self, mut account: grok_domain::Account) -> i64 {
        account.id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.accounts.lock().unwrap().push(account.clone());
        account.id
    }
}
#[async_trait::async_trait]
impl grok_admin::AdminStore for AccountStore {
    async fn list_accounts(
        &self,
        filter: &grok_admin::AccountListFilter,
        page: i64,
        page_size: i64,
    ) -> AdminResult<grok_admin::AccountPage> {
        let accounts = self.accounts.lock().unwrap();
        let filtered: Vec<_> = accounts
            .iter()
            .filter(|a| filter.provider.is_none_or(|p| a.provider == p))
            .filter(|a| filter.enabled.is_none_or(|e| a.enabled == e))
            .filter(|a| filter.auth_status.is_none_or(|s| a.auth_status == s))
            .cloned()
            .collect();
        let total = filtered.len() as i64;
        let offset = ((page - 1) * page_size).max(0) as usize;
        let items = filtered
            .iter()
            .skip(offset)
            .take(page_size as usize)
            .map(grok_admin::AccountView::from)
            .collect();
        Ok(grok_admin::AccountPage {
            items,
            page,
            page_size,
            total,
        })
    }
    async fn get_account(&self, id: i64) -> AdminResult<Option<grok_domain::Account>> {
        Ok(self
            .accounts
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .cloned())
    }
    async fn update_account(
        &self,
        id: i64,
        input: &grok_admin::UpdateAccountInput,
    ) -> AdminResult<Option<grok_domain::Account>> {
        let mut accounts = self.accounts.lock().unwrap();
        let Some(a) = accounts.iter_mut().find(|a| a.id == id) else {
            return Ok(None);
        };
        if let Some(enabled) = input.enabled {
            a.enabled = enabled;
        }
        if let Some(raw) = &input.auth_status {
            a.auth_status = grok_admin::accounts::parse_auth_status(raw)?;
        }
        if let Some(priority) = input.priority {
            a.priority = priority;
        }
        if input.cooldown_until.is_some() {
            a.cooldown_until = input.cooldown_until;
        }
        Ok(Some(a.clone()))
    }
    async fn delete_account(&self, id: i64) -> AdminResult<bool> {
        let mut accounts = self.accounts.lock().unwrap();
        let before = accounts.len();
        accounts.retain(|a| a.id != id);
        Ok(accounts.len() != before)
    }
    async fn list_quota_windows(&self, _id: i64) -> AdminResult<Vec<QuotaWindow>> {
        Ok(vec![])
    }
    async fn upsert_quota_window(&self, w: QuotaWindow) -> AdminResult<QuotaWindow> {
        Ok(w)
    }
    async fn list_model_states(&self, _id: i64) -> AdminResult<Vec<ModelState>> {
        Ok(vec![])
    }
    async fn pool_summary(&self) -> AdminResult<grok_admin::AccountSummary> {
        let accounts = self.accounts.lock().unwrap();
        let mut summary = grok_admin::AccountSummary::default();
        // 对齐 Go summary：reauth_required 按 auth_status 统计（不因 disabled 遮蔽）。
        for a in accounts.iter() {
            summary.total += 1;
            if a.auth_status == grok_domain::AuthStatus::ReauthRequired {
                summary.reauth_required += 1;
            } else if !a.enabled {
                summary.disabled += 1;
            } else {
                summary.available += 1;
            }
        }
        Ok(summary)
    }
    async fn analytics(&self) -> AdminResult<grok_admin::AccountAnalytics> {
        Ok(grok_admin::AccountAnalytics {
            quota_unknown: self.accounts.lock().unwrap().len() as i64,
            ..Default::default()
        })
    }
    async fn refresh_billing(&self, id: i64) -> AdminResult<bool> {
        Ok(self.accounts.lock().unwrap().iter().any(|a| a.id == id))
    }
    async fn refresh_quota(&self, id: i64) -> AdminResult<bool> {
        Ok(self.accounts.lock().unwrap().iter().any(|a| a.id == id))
    }
    async fn refresh_token(&self, id: i64) -> AdminResult<bool> {
        Ok(self.accounts.lock().unwrap().iter().any(|a| a.id == id))
    }
    async fn reauth(&self, id: i64) -> AdminResult<bool> {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some(a) = accounts.iter_mut().find(|a| a.id == id) {
            a.auth_status = grok_domain::AuthStatus::ReauthRequired;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn import_accounts(
        &self,
        inputs: &[grok_admin::ImportAccountInput],
    ) -> AdminResult<grok_admin::ImportResult> {
        let mut result = grok_admin::ImportResult::default();
        for (index, input) in inputs.iter().enumerate() {
            let provider = match grok_admin::accounts::parse_provider(&input.provider) {
                Some(p) => p,
                None => {
                    result.failed += 1;
                    result.errors.push(grok_admin::ImportError {
                        index,
                        reason: format!("unknown provider: {}", input.provider),
                    });
                    continue;
                }
            };
            let identity_key = input.identity_key.trim();
            if identity_key.is_empty() {
                result.failed += 1;
                result.errors.push(grok_admin::ImportError {
                    index,
                    reason: "identity_key 不能为空".into(),
                });
                continue;
            }
            let mut accounts = self.accounts.lock().unwrap();
            if accounts.iter().any(|a| a.identity_key == identity_key) {
                result.failed += 1;
                result.errors.push(grok_admin::ImportError {
                    index,
                    reason: format!("identity_key 冲突: {identity_key}"),
                });
                continue;
            }
            let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
            accounts.push(grok_domain::Account {
                id,
                identity_key: identity_key.to_string(),
                provider,
                name: input
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("imported-{id}")),
                priority: input.priority.unwrap_or(1),
                max_concurrent: input.max_concurrent.unwrap_or(8),
                enabled: true,
                auth_status: grok_domain::AuthStatus::Unknown,
                created_at: Some(Utc::now()),
                updated_at: Some(Utc::now()),
                ..Default::default()
            });
            result.imported += 1;
        }
        Ok(result)
    }

    async fn timeseries(&self, _days: i64) -> AdminResult<Vec<grok_admin::TimeseriesPoint>> {
        // fake 无审计记录：返回空数组（真实实现从 grok_request_audits 聚合，TODO）。
        Ok(Vec::new())
    }

    async fn top_accounts(&self, limit: i64) -> AdminResult<Vec<grok_admin::TopAccountView>> {
        let accounts = self.accounts.lock().unwrap();
        let mut items: Vec<grok_admin::TopAccountView> = accounts
            .iter()
            .map(|a| grok_admin::TopAccountView {
                account_id: a.id,
                name: a.name.clone(),
                requests: 0,
                failed: 0,
                failure_rate: 0.0,
            })
            .collect();
        items.sort_by_key(|v| std::cmp::Reverse(v.requests));
        items.truncate(limit.max(0) as usize);
        Ok(items)
    }
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

// ── 测试 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn guard_returns_401_without_bearer() {
    let (router, _) = setup().await;
    for path in [
        "/admin/dashboard",
        "/admin/models",
        "/admin/settings",
        "/admin/system",
        "/admin/chrome-tickets",
    ] {
        let resp = router.handle("GET", path, None, None).await;
        assert_eq!(resp.status, 401, "{path} 无 token 应 401");
    }
}

#[tokio::test]
async fn dashboard_endpoint() {
    let (router, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/dashboard", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200, "dashboard: {}", resp.body);
    assert_eq!(resp.body["total_accounts"], 3);
    assert_eq!(resp.body["available_accounts"], 2);
    assert_eq!(resp.body["requests_24h"], 120);
}

#[tokio::test]
async fn models_crud_and_bindings() {
    let (router, token) = setup().await;
    // 列表
    let resp = router
        .handle("GET", "/admin/models", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 1);
    // 创建
    let create = r#"{"provider":"grok_build","upstream_model":"grok-4","aliases":["grok-4b"]}"#;
    let resp = router
        .handle("POST", "/admin/models", Some(&bearer(&token)), Some(create))
        .await;
    assert_eq!(resp.status, 201, "create: {}", resp.body);
    let new_id = resp.body["id"].as_i64().unwrap();
    // 更新
    let update = r#"{"enabled":false}"#;
    let resp = router
        .handle(
            "PATCH",
            &format!("/admin/models/{new_id}"),
            Some(&bearer(&token)),
            Some(update),
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["enabled"], false);
    // 删除
    let resp = router
        .handle(
            "DELETE",
            &format!("/admin/models/{new_id}"),
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["deleted"], true);
    // 删除不存在 → 404
    let resp = router
        .handle("DELETE", "/admin/models/999", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 404);
    // 绑定
    let resp = router
        .handle("GET", "/admin/models/accounts", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["items"][0]["upstream_model"], "grok-3");
}

#[tokio::test]
async fn client_keys_crud() {
    let (router, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/client-keys", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 0);
    let create = r#"{"name":"web-gateway"}"#;
    let resp = router
        .handle(
            "POST",
            "/admin/client-keys",
            Some(&bearer(&token)),
            Some(create),
        )
        .await;
    assert_eq!(resp.status, 201, "create key: {}", resp.body);
    assert!(resp.body["secret"].as_str().unwrap().starts_with("sk-"));
    let key_id = resp.body["key"]["id"].as_i64().unwrap();
    // 更新
    let resp = router
        .handle(
            "PATCH",
            &format!("/admin/client-keys/{key_id}"),
            Some(&bearer(&token)),
            Some(r#"{"enabled":false}"#),
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["enabled"], false);
    // 删除
    let resp = router
        .handle(
            "DELETE",
            &format!("/admin/client-keys/{key_id}"),
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    // 删除不存在 → 404
    let resp = router
        .handle(
            "DELETE",
            "/admin/client-keys/999",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 404);
}

#[tokio::test]
async fn audits_list_and_summary() {
    let (router, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/request-audits", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 2);
    let resp = router
        .handle(
            "GET",
            "/admin/request-audits/summary",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["requests_24h"], 2);
    assert_eq!(resp.body["succeeded_24h"], 1);
    assert!(resp.body["success_rate_24h"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn settings_get_and_put_versioned() {
    let (router, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/settings", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["version"], 0);
    let put = r#"{"values":{"maxConcurrentPerAccount":"8"}}"#;
    let resp = router
        .handle("PUT", "/admin/settings", Some(&bearer(&token)), Some(put))
        .await;
    assert_eq!(resp.status, 200, "settings put: {}", resp.body);
    assert_eq!(resp.body["version"], 1);
    assert_eq!(resp.body["values"]["maxConcurrentPerAccount"], "8");
}

#[tokio::test]
async fn chrome_tickets_list_stats_sweep() {
    let (router, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/chrome-tickets", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["items"][0]["name"], "web-4");
    let resp = router
        .handle(
            "GET",
            "/admin/chrome-tickets/stats",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["available"], 1);
    let resp = router
        .handle(
            "POST",
            "/admin/chrome-tickets/sweep",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["swept"], 1);
}

#[tokio::test]
async fn media_and_timeline_and_system() {
    let (router, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/media/images", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 1);
    let resp = router
        .handle(
            "GET",
            "/admin/media/images/stats",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total_images"], 1);
    let resp = router
        .handle(
            "GET",
            "/admin/image-timeline?limit=10",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["items"][0]["status"], "completed");
    let resp = router
        .handle("GET", "/admin/system", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["version"], "test");
    assert_eq!(resp.body["ready"], true);
}

#[tokio::test]
async fn accounts_summary_analytics_refresh() {
    let (router, token) = setup().await;
    // summary
    let resp = router
        .handle(
            "GET",
            "/admin/accounts/summary",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200, "summary: {}", resp.body);
    assert_eq!(resp.body["total"], 3);
    assert_eq!(resp.body["available"], 2);
    assert_eq!(resp.body["reauth_required"], 1);
    // analytics
    let resp = router
        .handle(
            "GET",
            "/admin/accounts/analytics",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200, "analytics: {}", resp.body);
    // refresh-* 账号不存在 → 404
    for kind in [
        "refresh-billing",
        "refresh-quota",
        "refresh-token",
        "reauth",
    ] {
        let resp = router
            .handle(
                "POST",
                &format!("/admin/accounts/999/{kind}"),
                Some(&bearer(&token)),
                None,
            )
            .await;
        assert_eq!(resp.status, 404, "{kind} on missing account");
    }
    // refresh-token 正常
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/1/refresh-token",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200, "refresh-token: {}", resp.body);
    assert_eq!(resp.body["refreshed"], "token");
    // reauth 正常
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/1/reauth",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["refreshed"], "reauth");
}
#[tokio::test]
async fn import_accounts_batch() {
    let (router, token) = setup().await;
    // 全部合法
    let body = r#"[{"identity_key":"k-import-1","provider":"grok_build"},{"identity_key":"k-import-2","provider":"grok_web","name":"w2","priority":5,"max_concurrent":2}]"#;
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/import",
            Some(&bearer(&token)),
            Some(body),
        )
        .await;
    assert_eq!(resp.status, 201, "import: {}", resp.body);
    assert_eq!(resp.body["imported"], 2);
    assert_eq!(resp.body["failed"], 0);

    // 部分失败：重复 identity_key + 非法 provider + 空 key + 1 条合法
    let body = r#"[
        {"identity_key":"k-import-1","provider":"grok_build"},
        {"identity_key":"k-fresh","provider":"grok_build"},
        {"identity_key":"k-new","provider":"bad_provider"},
        {"identity_key":"","provider":"grok_build"}
    ]"#;
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/import",
            Some(&bearer(&token)),
            Some(body),
        )
        .await;
    assert_eq!(resp.status, 201, "partial import: {}", resp.body);
    assert_eq!(resp.body["imported"], 1);
    assert_eq!(resp.body["failed"], 3);
    let errors = resp.body["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 3);
    // 校验错误在对应 index（index 1 = k-fresh 成功，不在 errors 中）
    assert!(errors.iter().any(|e| e["index"] == 0));
    assert!(!errors.iter().any(|e| e["index"] == 1));
    assert!(errors
        .iter()
        .any(|e| e["index"] == 2 && e["reason"].as_str().unwrap().contains("provider")));
    assert!(errors.iter().any(|e| e["index"] == 3));
}

#[tokio::test]
async fn import_requires_bearer_and_valid_json() {
    let (router, token) = setup().await;
    // 无 token → 401
    let resp = router
        .handle("POST", "/admin/accounts/import", None, Some("[]"))
        .await;
    assert_eq!(resp.status, 401);
    // 坏 JSON → 400
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/import",
            Some(&bearer(&token)),
            Some("not-json"),
        )
        .await;
    assert_eq!(resp.status, 400);
    // 空数组 → 201 空结果
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/import",
            Some(&bearer(&token)),
            Some("[]"),
        )
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.body["imported"], 0);
}

#[tokio::test]
async fn analytics_timeseries_and_top_accounts() {
    let (router, token) = setup().await;
    // timeseries：fake 无审计数据 → 空数组（200）
    let resp = router
        .handle(
            "GET",
            "/admin/analytics/timeseries?days=7",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200, "timeseries: {}", resp.body);
    assert_eq!(resp.body.as_array().unwrap().len(), 0);
    // 默认 days=7；非法 days 也回退默认
    let resp = router
        .handle(
            "GET",
            "/admin/analytics/timeseries",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    let resp = router
        .handle(
            "GET",
            "/admin/analytics/timeseries?days=abc",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    // top-accounts：3 个 seed 账号，请求量为 0 → 按 id 升序（limit 截断）
    let resp = router
        .handle(
            "GET",
            "/admin/analytics/top-accounts?limit=2",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200, "top-accounts: {}", resp.body);
    let items = resp.body.as_array().unwrap();
    assert_eq!(items.len(), 2);
    // 401 覆盖
    let resp = router
        .handle("GET", "/admin/analytics/timeseries", None, None)
        .await;
    assert_eq!(resp.status, 401);
    let resp = router
        .handle("GET", "/admin/analytics/top-accounts", None, None)
        .await;
    assert_eq!(resp.status, 401);
}

#[tokio::test]
async fn import_field_level_validation_records_errors() {
    let (router, token) = setup().await;
    // 超长 identity_key / 超长 name / priority 超范围 / max_concurrent 超范围 → 逐条 error 不 panic
    let long_key = "k".repeat(65);
    let long_name = "n".repeat(161);
    let body = format!(
        r#"[
            {{"identity_key":"{long_key}","provider":"grok_build"}},
            {{"identity_key":"ok-1","provider":"grok_web","name":"{long_name}"}},
            {{"identity_key":"ok-2","provider":"grok_build","priority":1001}},
            {{"identity_key":"ok-3","provider":"grok_web","max_concurrent":0}},
            {{"identity_key":"ok-4","provider":"grok_build"}}
        ]"#
    );
    let resp = router
        .handle(
            "POST",
            "/admin/accounts/import",
            Some(&bearer(&token)),
            Some(&body),
        )
        .await;
    assert_eq!(resp.status, 201, "import: {}", resp.body);
    assert_eq!(resp.body["imported"], 1, "only ok-4 imported");
    assert_eq!(resp.body["failed"], 4);
    let errors = resp.body["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 4);
    assert!(errors
        .iter()
        .any(|e| e["index"] == 0 && e["reason"].as_str().unwrap().contains("超长")));
    assert!(errors
        .iter()
        .any(|e| e["index"] == 1 && e["reason"].as_str().unwrap().contains("超长")));
    assert!(errors
        .iter()
        .any(|e| e["index"] == 2 && e["reason"].as_str().unwrap().contains("priority")));
    assert!(errors
        .iter()
        .any(|e| e["index"] == 3 && e["reason"].as_str().unwrap().contains("max_concurrent")));
}

#[tokio::test]
async fn media_get_and_size_summary() {
    let (router, token) = setup().await;
    let bearer = bearer(&token);
    // 存在 → 200 详情
    let resp = router
        .handle("GET", "/admin/media/images/img_1", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 200, "media get: {}", resp.body);
    assert_eq!(resp.body["asset_id"], "img_1");
    // 不存在 → 404
    let resp = router
        .handle("GET", "/admin/media/images/nope", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 404, "media get missing: {}", resp.body);
    // size-summary → 200 汇总
    let resp = router
        .handle("GET", "/admin/media/size-summary", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 200, "size summary: {}", resp.body);
    assert_eq!(resp.body["total_images"], 1);
    // 401 覆盖
    let resp = router
        .handle("GET", "/admin/media/images/img_1", None, None)
        .await;
    assert_eq!(resp.status, 401);
}

#[tokio::test]
async fn system_config_and_logs_and_models_ext() {
    let (router, token) = setup().await;
    let bearer = bearer(&token);
    // system/config → 200 布尔视图
    let resp = router
        .handle("GET", "/admin/system/config", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 200, "system config: {}", resp.body);
    assert!(resp.body.get("admin_password_set").is_some());
    // system/logs → 200（含访问日志）
    let resp = router
        .handle("GET", "/admin/system/logs?limit=5", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 200, "system logs: {}", resp.body);
    eprintln!("LOGS_BODY={}", resp.body);
    let items = resp.body["items"].as_array().unwrap();
    assert!(!items.is_empty(), "logs should record access entries");
    // models/aliases → 200
    let resp = router
        .handle("GET", "/admin/models/aliases", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 200, "aliases: {}", resp.body);
    assert!(!resp.body["items"].as_array().unwrap().is_empty());
    // models/sync-state → 200
    let resp = router
        .handle("GET", "/admin/models/sync-state", Some(&bearer), None)
        .await;
    assert_eq!(resp.status, 200, "sync-state: {}", resp.body);
    assert_eq!(
        resp.body["items"].as_array().unwrap()[0]["account_count"],
        1
    );
    // 401 覆盖
    let resp = router
        .handle("GET", "/admin/system/config", None, None)
        .await;
    assert_eq!(resp.status, 401);
}
