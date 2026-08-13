//! `/admin/*` 管理台挂载（N5 真实接线）。
//!
//! grok-admin 的 [`AdminRouter`] 是「纯函数」形态（`handle(method, path, auth, body)`
//! → `AdminHttpResponse`），本模块负责：
//! - 内存版 [`AdminRepository`] / [`AdminSessionRepository`] / [`AdminStore`]
//!   （G4 未提供 PG 实现；账号数据真实源后续接 grok-storage 写路径）
//! - 启动幂等 bootstrap 管理员（`GROK_ADMIN_USERNAME`/`GROK_ADMIN_PASSWORD`）
//! - `POST /admin/auth/login` + `POST /admin/auth/refresh`：**绕过** [`AdminRouter::handle`]
//!   的全局 guard（否则登录请求本身被 401 拦截，形成死锁）
//! - axum `/{*path}` 泛型 handler 把 HTTP 请求映射到 [`AdminRouter::handle`]
//!
//! JWT secret：`GROK_ADMIN_SECRET`；缺省随机生成并告警（重启后 token 失效）。
//! `GROK_ADMIN_PASSWORD` 缺省不 bootstrap → 登录恒 401（admin 不可用 + 启动告警）。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use grok_admin::{
    Admin, AdminAuthService, AdminRepository, AdminResult, AdminRouter, AdminSessionRepository,
    AdminStore, Session, TokenService,
};
use grok_domain::{Account, QuotaWindow};
use serde::Deserialize;
use serde_json::json;

use crate::grok_nurture_ops::GrokNurtureService;
use crate::web_quota::WebQuotaService;

/// 内存认证存储（管理员 + 会话）。
#[derive(Default)]
pub struct InMemoryAuthStore {
    admins: Mutex<Vec<Admin>>,
    sessions: Mutex<Vec<Session>>,
    next_admin_id: AtomicI64,
    next_session_id: AtomicI64,
}

/// 内存管理员 repo（G4 无 PG 实现；够 bootstrap + login + refresh）。
pub struct InMemoryAdminRepo(pub Arc<InMemoryAuthStore>);

/// 内存会话 repo。
pub struct InMemorySessionRepo(pub Arc<InMemoryAuthStore>);

/// 内存账号 store：账号数据真实源留 grok-storage 写路径（TODO）；
/// `/admin/accounts` 列表当前返回空集，但路由/鉴权/分页形状完整。
#[derive(Default)]
pub struct InMemoryAdminStore {
    accounts: Mutex<Vec<Account>>,
}

#[async_trait::async_trait]
impl AdminRepository for InMemoryAdminRepo {
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
        let mut admins = self.0.admins.lock().unwrap();
        if let Some(a) = admins.iter_mut().find(|a| a.id == admin_id) {
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
impl AdminSessionRepository for InMemorySessionRepo {
    async fn get_by_token_hash(&self, hash: &str) -> AdminResult<Option<Session>> {
        Ok(self
            .0
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.refresh_token_hash == hash)
            .cloned())
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
    async fn rotate(
        &self,
        id: i64,
        old_hash: &str,
        new_hash: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> AdminResult<bool> {
        let mut sessions = self.0.sessions.lock().unwrap();
        let Some(s) = sessions
            .iter_mut()
            .find(|s| s.id == id && s.refresh_token_hash == old_hash)
        else {
            return Ok(false);
        };
        s.refresh_token_hash = new_hash.to_string();
        s.expires_at = expires_at;
        Ok(true)
    }
    async fn revoke(&self, id: i64) -> AdminResult<()> {
        self.0.sessions.lock().unwrap().retain(|s| s.id != id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl AdminStore for InMemoryAdminStore {
    async fn list_accounts(
        &self,
        _filter: &grok_admin::AccountListFilter,
        page: i64,
        page_size: i64,
    ) -> AdminResult<grok_admin::AccountPage> {
        // TODO(grok-storage write-path)：接 PgAccountRepository。当前空集占位。
        let accounts = self.accounts.lock().unwrap();
        let total = accounts.len() as i64;
        let offset = ((page - 1) * page_size).max(0) as usize;
        let items = accounts
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
    async fn get_account(&self, _id: i64) -> AdminResult<Option<Account>> {
        Ok(None)
    }
    async fn update_account(
        &self,
        _id: i64,
        _input: &grok_admin::UpdateAccountInput,
    ) -> AdminResult<Option<Account>> {
        Ok(None)
    }
    async fn delete_account(&self, _id: i64) -> AdminResult<bool> {
        Ok(false)
    }
    async fn list_quota_windows(&self, _id: i64) -> AdminResult<Vec<QuotaWindow>> {
        Ok(vec![])
    }
    async fn upsert_quota_window(&self, w: QuotaWindow) -> AdminResult<QuotaWindow> {
        Ok(w)
    }
    async fn list_model_states(&self, _id: i64) -> AdminResult<Vec<grok_domain::ModelState>> {
        Ok(vec![])
    }
    async fn pool_summary(&self) -> AdminResult<grok_admin::AccountSummary> {
        Ok(Default::default())
    }
    async fn analytics(&self) -> AdminResult<grok_admin::AccountAnalytics> {
        Ok(Default::default())
    }
    async fn refresh_billing(&self, _id: i64) -> AdminResult<bool> {
        Ok(false)
    }
    async fn refresh_quota(&self, _id: i64) -> AdminResult<bool> {
        Ok(false)
    }
    async fn refresh_token(&self, _id: i64) -> AdminResult<bool> {
        Ok(false)
    }
    async fn reauth(&self, _id: i64) -> AdminResult<bool> {
        Ok(false)
    }
    async fn import_accounts(
        &self,
        _inputs: &[grok_admin::ImportAccountInput],
    ) -> AdminResult<grok_admin::ImportResult> {
        Ok(Default::default())
    }
    async fn timeseries(&self, _days: i64) -> AdminResult<Vec<grok_admin::TimeseriesPoint>> {
        Ok(vec![])
    }
    async fn top_accounts(&self, _limit: i64) -> AdminResult<Vec<grok_admin::TopAccountView>> {
        Ok(vec![])
    }
}

/// 管理台扩展路由（养号 / 批量额度刷新），不走 grok-admin AdminRouter。
#[derive(Clone, Default)]
pub struct AdminExtras {
    pub nurture: Option<Arc<GrokNurtureService>>,
    pub quota: Option<Arc<WebQuotaService>>,
}

/// 管理台 HTTP 挂载包：受 guard 保护的 [`AdminRouter`] + 登录/刷新所需的 auth service。
pub struct AdminHttpBundle {
    router: AdminRouter,
    auth: Arc<AdminAuthService>,
    nurture: Option<Arc<GrokNurtureService>>,
    quota: Option<Arc<WebQuotaService>>,
}

/// 构造 admin bundle：内存认证存储 + 幂等 bootstrap 管理员。
///
/// `password` 为 None（`GROK_ADMIN_PASSWORD` 未配置）→ 不 bootstrap，
/// `/admin/auth/login` 恒 401（admin 不可用，启动已告警）。
pub async fn build_admin_bundle(
    username: &str,
    password: Option<&str>,
    secret: &str,
    extras: AdminExtras,
) -> AdminHttpBundle {
    let auth_store = Arc::new(InMemoryAuthStore::default());
    let repo = Arc::new(InMemoryAdminRepo(auth_store.clone()));
    let sessions = Arc::new(InMemorySessionRepo(auth_store));
    let store: Arc<dyn AdminStore> = Arc::new(InMemoryAdminStore::default());
    let domains = crate::admin_domains::build_admin_domains();
    build_bundle(
        repo, sessions, store, username, password, secret, extras, domains,
    )
    .await
}

/// 共享组装：鉴权 service（guard 与 login/refresh 各一份但共享同一底层存储）+
/// bootstrap + 路由。`pg_admin.rs` 的 PG 数据面复用本函数。
pub(crate) async fn build_bundle(
    repo: Arc<dyn AdminRepository>,
    sessions: Arc<dyn AdminSessionRepository>,
    store: Arc<dyn AdminStore>,
    username: &str,
    password: Option<&str>,
    secret: &str,
    extras: AdminExtras,
    domains: grok_admin::AdminDomains,
) -> AdminHttpBundle {
    let ttl = (chrono::Duration::hours(1), chrono::Duration::days(7));
    // guard 与 login/refresh 各持一个 AdminAuthService，但共享同一底层存储
    // （bootstrap / login 写入对 guard 端可见）。
    let router_auth = AdminAuthService::new(
        repo.clone(),
        sessions.clone(),
        TokenService::new(secret),
        ttl.0,
        ttl.1,
    );
    let login_auth = Arc::new(AdminAuthService::new(
        repo,
        sessions,
        TokenService::new(secret),
        ttl.0,
        ttl.1,
    ));
    match password {
        Some(pw) => match login_auth.bootstrap(username, pw).await {
            Ok(()) => tracing::info!("admin bootstrap ok: {username}"),
            Err(e) => tracing::warn!("admin bootstrap 失败（密码过短或已存在）: {e}"),
        },
        None => {
            tracing::warn!(
                "GROK_ADMIN_PASSWORD 未配置：admin 不可用（/admin/auth/login 将返回 401），生产前必须设置"
            );
        }
    }
    AdminHttpBundle {
        router: AdminRouter::new(router_auth, grok_admin::AccountAdminService::new(store))
            .with_domains(domains),
        auth: login_auth,
        nurture: extras.nurture,
        quota: extras.quota,
    }
}

#[derive(Clone)]
struct AdminState {
    router: Arc<AdminRouter>,
    auth: Arc<AdminAuthService>,
    nurture: Option<Arc<GrokNurtureService>>,
    quota: Option<Arc<WebQuotaService>>,
}

/// 构建 `/admin/*` 路由（axum）：login/refresh 绕过 guard + 泛型受保护 handler。
pub fn admin_app(bundle: AdminHttpBundle) -> Router {
    let state = AdminState {
        router: Arc::new(bundle.router),
        auth: bundle.auth,
        nurture: bundle.nurture,
        quota: bundle.quota,
    };
    Router::new()
        .route("/admin/auth/login", axum::routing::post(admin_login))
        .route("/admin/auth/refresh", axum::routing::post(admin_refresh))
        .route("/admin/{*path}", axum::routing::any(admin_handle))
        .with_state(state)
}

/// 登录请求体。
#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

/// 刷新请求体。
#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

/// `POST /admin/auth/login`：绕过 guard 直接调 [`AdminAuthService::login`]。
async fn admin_login(
    State(state): State<AdminState>,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    // 上游反代应透传真实来源 IP（X-Forwarded-For）；当前未接 ConnectInfo，用占位。
    const REMOTE: &str = "127.0.0.1";
    match state
        .auth
        .login(&body.username, &body.password, REMOTE)
        .await
    {
        Ok((admin, tokens)) => (
            StatusCode::OK,
            Json(json!({
                "admin": { "id": admin.id, "username": admin.username },
                "tokens": {
                    "access_token": tokens.access_token,
                    "access_token_expires_at": tokens.access_token_expires_at,
                    "refresh_token": tokens.refresh_token,
                    "refresh_token_expires_at": tokens.refresh_token_expires_at,
                },
            })),
        ),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_credentials" })),
        ),
    }
}

/// `POST /admin/auth/refresh`：绕过 guard 直接调 [`AdminAuthService::refresh`]。
async fn admin_refresh(
    State(state): State<AdminState>,
    Json(body): Json<RefreshRequest>,
) -> impl IntoResponse {
    match state.auth.refresh(&body.refresh_token).await {
        Ok(tokens) => (
            StatusCode::OK,
            Json(json!({
                "tokens": {
                    "access_token": tokens.access_token,
                    "access_token_expires_at": tokens.access_token_expires_at,
                    "refresh_token": tokens.refresh_token,
                    "refresh_token_expires_at": tokens.refresh_token_expires_at,
                },
            })),
        ),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_session" })),
        ),
    }
}

/// 泛型 `/admin/*` handler：method + 完整路径 + Bearer 头 + body → 响应。
async fn admin_handle(
    State(state): State<AdminState>,
    method: Method,
    uri: axum::http::Uri,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // axum 的 {*path} 不含前导斜杠与 query；AdminRouter 通过 split_query 解析分页参数。
    let full_path = match uri.query() {
        Some(q) if !q.is_empty() => format!("/admin/{path}?{q}"),
        _ => format!("/admin/{path}"),
    };
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body_text = if body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&body).to_string())
    };

    if let Some(nurture) = &state.nurture {
        if let Some((status, body)) = nurture
            .handle(method.as_str(), &full_path, body_text.as_deref())
            .await
        {
            return (
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(body),
            );
        }
    }

    if method == Method::POST && full_path == "/admin/accounts/web/refresh-quotas" {
        if let Some(quota) = &state.quota {
            let limit = body_text
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|v| v.get("limit").and_then(|l| l.as_i64()))
                .unwrap_or(64);
            let (ok, fail) = quota.refresh_enabled_batch(limit).await;
            return (StatusCode::OK, Json(json!({ "ok": ok, "fail": fail })));
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({ "error": "quotaNotWired", "message": "额度刷新未接线（需 GROK2API_DIRECT）" }),
            ),
        );
    }

    let resp = state
        .router
        .handle(
            method.as_str(),
            &full_path,
            authorization.as_deref(),
            body_text.as_deref(),
        )
        .await;
    (
        StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(resp.body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存组装（bootstrap admin）→ 直接调 router.handle（与 axum 层同语义）。
    async fn bundle_with_admin() -> (AdminRouter, String) {
        let bundle = build_admin_bundle(
            "admin",
            Some("password123"),
            "01234567890123456789012345678901",
            AdminExtras::default(),
        )
        .await;
        let auth = bundle.auth.clone();
        let (_, tokens) = auth
            .login("admin", "password123", "127.0.0.1")
            .await
            .expect("login");
        (bundle.router, tokens.access_token)
    }

    #[tokio::test]
    async fn domains_wired_no_longer_503() {
        let (router, token) = bundle_with_admin().await;
        for path in [
            "/admin/models",
            "/admin/client-keys",
            "/admin/request-audits",
            "/admin/dashboard",
            "/admin/settings",
            "/admin/chrome-tickets",
            "/admin/media/images",
            "/admin/system/config",
        ] {
            let resp = router
                .handle("GET", path, Some(&format!("Bearer {token}")), None)
                .await;
            assert_eq!(
                resp.status, 200,
                "{path} 应已接线（200），实际 {} {}",
                resp.status, resp.body
            );
        }
    }

    #[tokio::test]
    async fn models_domain_read_write_roundtrip() {
        let (router, token) = bundle_with_admin().await;
        let auth = |bearer: &str| Some(format!("Bearer {bearer}"));
        let created = router
            .handle(
                "POST",
                "/admin/models",
                auth(&token).as_deref(),
                Some(r#"{"provider":"grok_web","upstream_model":"grok-4","aliases":["grok-4"]}"#),
            )
            .await;
        assert_eq!(created.status, 201, "{}", created.body);
        let id = created.body["id"].as_i64().expect("id");
        let listed = router
            .handle("GET", "/admin/models", auth(&token).as_deref(), None)
            .await;
        assert_eq!(listed.status, 200, "{}", listed.body);
        assert!(
            listed.body["items"]
                .as_array()
                .is_some_and(|v| v.iter().any(|r| r["id"] == serde_json::json!(id))),
            "列表应含新建模型: {}",
            listed.body
        );
    }

    #[tokio::test]
    async fn accounts_list_honors_page_query() {
        let (router, token) = bundle_with_admin().await;
        let auth = |bearer: &str| Some(format!("Bearer {bearer}"));
        let page1 = router
            .handle(
                "GET",
                "/admin/accounts?page=1&pageSize=5",
                auth(&token).as_deref(),
                None,
            )
            .await;
        assert_eq!(page1.status, 200, "{}", page1.body);
        assert_eq!(page1.body["pageSize"], 5, "{}", page1.body);
        assert_eq!(page1.body["page"], 1, "{}", page1.body);
        let page2 = router
            .handle(
                "GET",
                "/admin/accounts?page=2&pageSize=5",
                auth(&token).as_deref(),
                None,
            )
            .await;
        assert_eq!(page2.status, 200, "{}", page2.body);
        assert_eq!(page2.body["page"], 2, "{}", page2.body);
        assert_eq!(page2.body["pageSize"], 5, "{}", page2.body);
    }

    #[tokio::test]
    async fn import_accepts_wrapped_format() {
        let (router, token) = bundle_with_admin().await;
        let body = r#"{"format":"jsonl","items":[
            {"identity_key":"u1@x.com","provider":"grok_web","name":"u1","credential":"c1"},
            {"identity_key":"b1","provider":"grok_build"}
        ]}"#;
        let resp = router
            .handle(
                "POST",
                "/admin/accounts/import",
                Some(&format!("Bearer {token}")),
                Some(body),
            )
            .await;
        // 内存 store 的 import 当前返回空结果（存储写入在 PgAdminStore）；此处验证路由 + 契约形状。
        assert_eq!(resp.status, 201, "{}", resp.body);
        assert!(resp.body.get("imported").is_some(), "{}", resp.body);
        assert!(resp.body.get("failed").is_some(), "{}", resp.body);
    }

    #[tokio::test]
    async fn import_accepts_bare_array() {
        let (router, token) = bundle_with_admin().await;
        let body = r#"[{"identity_key":"u2@x.com","provider":"grok_web"}]"#;
        let resp = router
            .handle(
                "POST",
                "/admin/accounts/import",
                Some(&format!("Bearer {token}")),
                Some(body),
            )
            .await;
        assert_eq!(resp.status, 201, "{}", resp.body);
    }

    #[tokio::test]
    async fn import_rejects_garbage() {
        let (router, token) = bundle_with_admin().await;
        let resp = router
            .handle(
                "POST",
                "/admin/accounts/import",
                Some(&format!("Bearer {token}")),
                Some(r#"{"format":"jsonl"}"#),
            )
            .await;
        assert_eq!(resp.status, 400, "{}", resp.body);
    }
}
