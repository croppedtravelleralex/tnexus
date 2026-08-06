//! `/admin/*` 管理台挂载（N5 真实接线）。
//!
//! grok-admin 的 [`AdminRouter`] 是「纯函数」形态（`handle(method, path, auth, body)`
//! → `AdminHttpResponse`），本模块负责：
//! - 内存版 [`AdminRepository`] / [`AdminSessionRepository`] / [`AdminStore`]
//!   （G4 未提供 PG 实现；账号数据真实源后续接 grok-storage 写路径）
//! - 启动幂等 bootstrap 管理员（`GROK_ADMIN_USERNAME`/`GROK_ADMIN_PASSWORD`）
//! - axum `/{*path}` 泛型 handler 把 HTTP 请求映射到 [`AdminRouter::handle`]
//!
//! JWT secret：`GROK_ADMIN_SECRET`；缺省随机生成并告警（重启后 token 失效）。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use grok_admin::{
    Admin, AdminAuthService, AdminRepository, AdminResult, AdminRouter, AdminSessionRepository,
    AdminStore, Session, TokenService,
};
use grok_domain::{Account, QuotaWindow};

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

/// 构造 admin router：内存认证存储 + 幂等 bootstrap 管理员。
pub async fn build_admin_router(username: &str, password: &str, secret: &str) -> AdminRouter {
    let auth_store = Arc::new(InMemoryAuthStore::default());
    let auth = AdminAuthService::new(
        Arc::new(InMemoryAdminRepo(auth_store.clone())),
        Arc::new(InMemorySessionRepo(auth_store)),
        TokenService::new(secret),
        chrono::Duration::hours(1),
        chrono::Duration::days(7),
    );
    // bootstrap 幂等（count>0 跳过）；密码过短时忽略（运维可后补）。
    let _ = auth.bootstrap(username, password).await;
    let store = Arc::new(InMemoryAdminStore::default());
    AdminRouter::new(auth, grok_admin::AccountAdminService::new(store))
}

/// 构建 `/admin/*` 路由（axum 泛型 handler 映射到 [`AdminRouter::handle`]）。
pub fn admin_app(router: AdminRouter) -> axum::Router {
    let state = AdminState {
        router: Arc::new(router),
    };
    axum::Router::new()
        .route("/admin/{*path}", axum::routing::any(admin_handle))
        .with_state(state)
}

#[derive(Clone)]
struct AdminState {
    router: Arc<AdminRouter>,
}

/// 泛型 `/admin/*` handler：method + 完整路径 + Bearer 头 + body → 响应。
async fn admin_handle(
    State(state): State<AdminState>,
    method: Method,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // axum 的 {*path} 不含前导斜杠；AdminRouter 期待完整路径（含 /admin 前缀）。
    let full_path = format!("/admin/{path}");
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body_text = if body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&body).to_string())
    };
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
