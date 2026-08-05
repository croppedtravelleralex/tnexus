//! G4-P2 Admin 账号管理端点集成测试。
//!
//! 覆盖：Bearer guard（无/坏 token → 401）、列表过滤/分页、详情（含额度窗口与
//! 模型状态）、更新（enabled/auth_status/priority/cooldown）、删除、额度窗口读写、
//! 模型状态查询、参数校验错误映射（400/404）。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use grok_admin::repos::{AdminRepository, AdminSessionRepository};
use grok_admin::{
    AccountAdminService, AccountListFilter, AccountPage, Admin, AdminAuthService, AdminResult,
    AdminRouter, AdminStore, Session, TokenService,
};
use grok_domain::{
    Account, AuthStatus, ModelState, ModelStatus, Provider, QuotaSource, QuotaWindow,
};

fn secret() -> String {
    "12345678901234567890123456789012".to_string()
}

// ── auth fakes（复用 tests/service.rs 的模式）──────────────────────

#[derive(Default)]
pub struct AuthStore {
    admins: Mutex<Vec<Admin>>,
    sessions: Mutex<Vec<Session>>,
    next_admin_id: AtomicI64,
    next_session_id: AtomicI64,
}

pub struct FakeAdminRepo {
    store: Arc<AuthStore>,
}
pub struct FakeSessionRepo {
    store: Arc<AuthStore>,
}

impl FakeAdminRepo {
    fn new(store: Arc<AuthStore>) -> Self {
        Self { store }
    }
}

impl FakeSessionRepo {
    fn new(store: Arc<AuthStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl AdminRepository for FakeAdminRepo {
    async fn count(&self) -> AdminResult<i64> {
        Ok(self.store.admins.lock().unwrap().len() as i64)
    }
    async fn create(&self, mut admin: Admin) -> AdminResult<Admin> {
        admin.id = self.store.next_admin_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.store.admins.lock().unwrap().push(admin.clone());
        Ok(admin)
    }
    async fn get_by_username(&self, username: &str) -> AdminResult<Option<Admin>> {
        Ok(self
            .store
            .admins
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.username == username)
            .cloned())
    }
    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Admin>> {
        Ok(self
            .store
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
        {
            let mut admins = self.store.admins.lock().unwrap();
            if let Some(admin) = admins.iter_mut().find(|a| a.id == admin_id) {
                admin.password_hash = password_hash.to_string();
            }
        }
        self.store
            .sessions
            .lock()
            .unwrap()
            .retain(|s| s.admin_id != admin_id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl AdminSessionRepository for FakeSessionRepo {
    async fn get_by_token_hash(&self, _token_hash: &str) -> AdminResult<Option<Session>> {
        Ok(None)
    }
    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Session>> {
        Ok(self
            .store
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .cloned())
    }
    async fn create(&self, mut session: Session) -> AdminResult<Session> {
        session.id = self.store.next_session_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.store.sessions.lock().unwrap().push(session.clone());
        Ok(session)
    }
    async fn rotate(
        &self,
        _session_id: i64,
        _old_token_hash: &str,
        _new_token_hash: &str,
        _expires_at: chrono::DateTime<chrono::Utc>,
    ) -> AdminResult<bool> {
        Ok(true)
    }
    async fn revoke(&self, id: i64) -> AdminResult<()> {
        self.store
            .sessions
            .lock()
            .unwrap()
            .retain(|s| s.id != id);
        Ok(())
    }
}

// ── 账号内存 fake store ───────────────────────────────────────────

#[derive(Default)]
struct AccountStore {
    accounts: Mutex<Vec<Account>>,
    windows: Mutex<Vec<QuotaWindow>>,
    model_states: Mutex<Vec<ModelState>>,
    next_id: AtomicI64,
}

impl AccountStore {
    fn seed(&self, mut account: Account) -> i64 {
        account.id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.accounts.lock().unwrap().push(account.clone());
        account.id
    }
}

fn build_account(id: i64, provider: Provider, enabled: bool, status: AuthStatus) -> Account {
    Account {
        id,
        identity_key: format!("key-{id}"),
        provider,
        name: format!("acc-{id}"),
        enabled,
        auth_status: status,
        priority: 10,
        max_concurrent: 4,
        failure_count: 0,
        cooldown_until: None,
        last_error: None,
        observed_model: None,
        created_at: Some(Utc::now() - Duration::hours(1)),
        updated_at: Some(Utc::now()),
        ..Default::default()
    }
}

#[async_trait::async_trait]
impl AdminStore for AccountStore {
    async fn list_accounts(
        &self,
        filter: &AccountListFilter,
        page: i64,
        page_size: i64,
    ) -> AdminResult<AccountPage> {
        let accounts = self.accounts.lock().unwrap();
        let filtered: Vec<&Account> = accounts
            .iter()
            .filter(|a| filter.provider.is_none_or(|p| a.provider == p))
            .filter(|a| filter.enabled.is_none_or(|e| a.enabled == e))
            .filter(|a| filter.auth_status.is_none_or(|s| a.auth_status == s))
            .collect();
        let total = filtered.len() as i64;
        let offset = ((page - 1) * page_size).max(0) as usize;
        let items: Vec<grok_admin::AccountView> = filtered
            .iter()
            .skip(offset)
            .take(page_size as usize)
            .map(|a| grok_admin::AccountView::from(*a))
            .collect();
        Ok(grok_admin::AccountPage { items, page, page_size, total })
    }

    async fn get_account(&self, id: i64) -> AdminResult<Option<Account>> {
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
    ) -> AdminResult<Option<Account>> {
        let mut accounts = self.accounts.lock().unwrap();
        let Some(account) = accounts.iter_mut().find(|a| a.id == id) else {
            return Ok(None);
        };
        if let Some(enabled) = input.enabled {
            account.enabled = enabled;
        }
        if let Some(raw) = &input.auth_status {
            account.auth_status = grok_admin::accounts::parse_auth_status(raw)?;
        }
        if let Some(priority) = input.priority {
            account.priority = priority;
        }
        if input.cooldown_until.is_some() {
            account.cooldown_until = input.cooldown_until;
        }
        account.updated_at = Some(Utc::now());
        Ok(Some(account.clone()))
    }

    async fn delete_account(&self, id: i64) -> AdminResult<bool> {
        let mut accounts = self.accounts.lock().unwrap();
        let before = accounts.len();
        accounts.retain(|a| a.id != id);
        Ok(accounts.len() != before)
    }

    async fn list_quota_windows(&self, account_id: i64) -> AdminResult<Vec<QuotaWindow>> {
        Ok(self
            .windows
            .lock()
            .unwrap()
            .iter()
            .filter(|w| w.account_id == account_id)
            .cloned()
            .collect())
    }

    async fn upsert_quota_window(&self, window: QuotaWindow) -> AdminResult<QuotaWindow> {
        let mut windows = self.windows.lock().unwrap();
        if let Some(existing) = windows
            .iter_mut()
            .find(|w| w.account_id == window.account_id && w.mode == window.mode)
        {
            *existing = window.clone();
        } else {
            windows.push(window.clone());
        }
        Ok(window)
    }

    async fn list_model_states(&self, account_id: i64) -> AdminResult<Vec<ModelState>> {
        Ok(self
            .model_states
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.account_id == account_id)
            .cloned()
            .collect())
    }
}

// ── fixture ───────────────────────────────────────────────────────

async fn setup() -> (AdminRouter, Arc<AccountStore>, String) {
    let auth_store = Arc::new(AuthStore::default());
    let auth = AdminAuthService::new(
        Arc::new(FakeAdminRepo::new(auth_store.clone())),
        Arc::new(FakeSessionRepo::new(auth_store)),
        TokenService::new(&secret()),
        Duration::hours(1),
        Duration::days(7),
    );
    auth.bootstrap("admin", "password123").await.expect("bootstrap");
    let (_, tokens) = auth
        .login("admin", "password123", "127.0.0.1")
        .await
        .expect("login");

    let account_store = Arc::new(AccountStore::default());
    // 3 build 账号（2 active + 1 disabled）+ 1 web 账号
    let _b1 = account_store.seed(build_account(1, Provider::GrokBuild, true, AuthStatus::Active));
    let _b2 = account_store.seed(build_account(2, Provider::GrokBuild, true, AuthStatus::Active));
    let _b3 = account_store.seed(build_account(3, Provider::GrokBuild, false, AuthStatus::Active));
    let w1 = account_store.seed(build_account(4, Provider::GrokWeb, true, AuthStatus::ReauthRequired));
    // 给 w1 挂额度窗口 + 模型状态
    account_store.windows.lock().unwrap().push(QuotaWindow {
        account_id: w1,
        mode: "imagine".into(),
        remaining: 5,
        total: 10,
        reset_at: None,
        synced_at: Some(Utc::now()),
        source: QuotaSource::Upstream,
        updated_at: Utc::now(),
    });
    account_store.model_states.lock().unwrap().push(ModelState {
        account_id: w1,
        upstream_model: "grok-imagine-image".into(),
        status: ModelStatus::Available,
        reason: Some("probed".into()),
        consecutive_failures: 0,
        last_attempt_at: Some(Utc::now()),
        last_success_at: Some(Utc::now()),
        cooldown_until: None,
        updated_at: Utc::now(),
    });

    let router = AdminRouter::new(
        auth,
        AccountAdminService::new(account_store.clone()),
    );
    let token = tokens.access_token.clone();
    (router, account_store, token)
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

// ── guard ─────────────────────────────────────────────────────────

#[tokio::test]
async fn rejects_missing_or_bad_token() {
    let (router, _, _) = setup().await;
    let no_token = router
        .handle("GET", "/admin/accounts", None, None)
        .await;
    assert_eq!(no_token.status, 401);
    let bad_token = router
        .handle("GET", "/admin/accounts", Some("Bearer garbage.token"), None)
        .await;
    assert_eq!(bad_token.status, 401);
}

// ── 列表 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn lists_accounts_with_filter_and_pagination() {
    let (router, _, token) = setup().await;

    // 全部（4 个）
    let resp = router.handle("GET", "/admin/accounts", Some(&bearer(&token)), None).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 4);
    assert_eq!(resp.body["items"].as_array().unwrap().len(), 4);

    // provider 过滤 → grok_build 3 个
    let resp = router
        .handle(
            "GET",
            "/admin/accounts?provider=grok_build",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 3);

    // enabled=false → 1 个
    let resp = router
        .handle("GET", "/admin/accounts?enabled=false", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 1);
    assert_eq!(resp.body["items"][0]["enabled"], false);

    // authStatus=reauthRequired → 1 个
    let resp = router
        .handle(
            "GET",
            "/admin/accounts?authStatus=reauthRequired",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 1);
    assert_eq!(resp.body["items"][0]["auth_status"], "reauthRequired");

    // 分页：pageSize=2, page=2 → 2 个
    let resp = router
        .handle(
            "GET",
            "/admin/accounts?page=2&pageSize=2",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["total"], 4);
    assert_eq!(resp.body["items"].as_array().unwrap().len(), 2);
    assert_eq!(resp.body["page"], 2);

    // 无效 provider → 400
    let resp = router
        .handle(
            "GET",
            "/admin/accounts?provider=bad",
            Some(&bearer(&token)),
            None,
        )
        .await;
    assert_eq!(resp.status, 400);
}

// ── 详情 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn gets_account_detail_with_quota_and_model_states() {
    let (router, _, token) = setup().await;
    // w1 = id 4
    let resp = router
        .handle("GET", "/admin/accounts/4", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["id"], 4);
    assert_eq!(resp.body["provider"], "grok_web");
    assert_eq!(resp.body["quota_windows"].as_array().unwrap().len(), 1);
    assert_eq!(resp.body["quota_windows"][0]["mode"], "imagine");
    assert_eq!(resp.body["model_states"].as_array().unwrap().len(), 1);
    assert_eq!(resp.body["model_states"][0]["status"], "available");

    // 不存在 → 404
    let resp = router
        .handle("GET", "/admin/accounts/999", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 404);
    // 非法 id → 400
    let resp = router
        .handle("GET", "/admin/accounts/abc", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 400);
}

// ── 更新 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn updates_account_fields() {
    let (router, store, token) = setup().await;
    let resp = router
        .handle(
            "PATCH",
            "/admin/accounts/1",
            Some(&bearer(&token)),
            Some(r#"{"enabled":false,"priority":99,"cooldown_until":"2030-01-01T00:00:00Z"}"#),
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["enabled"], false);
    assert_eq!(resp.body["priority"], 99);

    // auth_status 更新
    let resp = router
        .handle(
            "PATCH",
            "/admin/accounts/1",
            Some(&bearer(&token)),
            Some(r#"{"auth_status":"banned"}"#),
        )
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["auth_status"], "banned");
    let stored = store.get_account(1).await.unwrap().unwrap();
    assert_eq!(stored.auth_status, AuthStatus::Banned);

    // 不存在 → 404
    let resp = router
        .handle(
            "PATCH",
            "/admin/accounts/999",
            Some(&bearer(&token)),
            Some(r#"{"enabled":false}"#),
        )
        .await;
    assert_eq!(resp.status, 404);

    // 非法 auth_status → 400
    let resp = router
        .handle(
            "PATCH",
            "/admin/accounts/1",
            Some(&bearer(&token)),
            Some(r#"{"auth_status":"nope"}"#),
        )
        .await;
    assert_eq!(resp.status, 400);
}

// ── 删除 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn deletes_account() {
    let (router, store, token) = setup().await;
    let resp = router
        .handle("DELETE", "/admin/accounts/3", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["deleted"], true);
    assert!(store.get_account(3).await.unwrap().is_none());

    // 再删 → 404
    let resp = router
        .handle("DELETE", "/admin/accounts/3", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 404);
}

// ── 额度窗口 ──────────────────────────────────────────────────────

#[tokio::test]
async fn reads_and_writes_quota_windows() {
    let (router, store, token) = setup().await;

    // 初始：w1(id=4) 有 imagine 5/10
    let resp = router
        .handle("GET", "/admin/accounts/4/quota", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["items"].as_array().unwrap().len(), 1);

    // 新增 fast 窗口
    let resp = router
        .handle(
            "PUT",
            "/admin/accounts/4/quota",
            Some(&bearer(&token)),
            Some(r#"{"mode":"fast","remaining":30,"total":30,"source":"upstream"}"#),
        )
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.body["mode"], "fast");
    assert_eq!(resp.body["remaining"], 30);
    assert_eq!(resp.body["source"], "upstream");

    // 更新 fast 窗口（upsert 语义）
    let resp = router
        .handle(
            "PUT",
            "/admin/accounts/4/quota",
            Some(&bearer(&token)),
            Some(r#"{"mode":"fast","remaining":20,"total":30,"source":"estimated"}"#),
        )
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.body["remaining"], 20);
    let windows = store.list_quota_windows(4).await.unwrap();
    assert_eq!(windows.len(), 2, "upsert 不新增行");
    assert_eq!(windows.iter().find(|w| w.mode == "fast").unwrap().remaining, 20);

    // 空 mode → 400
    let resp = router
        .handle(
            "PUT",
            "/admin/accounts/4/quota",
            Some(&bearer(&token)),
            Some(r#"{"mode":"","remaining":1,"total":1}"#),
        )
        .await;
    assert_eq!(resp.status, 400);

    // 负值 → 400
    let resp = router
        .handle(
            "PUT",
            "/admin/accounts/4/quota",
            Some(&bearer(&token)),
            Some(r#"{"mode":"auto","remaining":-1,"total":10}"#),
        )
        .await;
    assert_eq!(resp.status, 400);

    // 账号不存在 → 404
    let resp = router
        .handle(
            "PUT",
            "/admin/accounts/999/quota",
            Some(&bearer(&token)),
            Some(r#"{"mode":"fast","remaining":1,"total":1}"#),
        )
        .await;
    assert_eq!(resp.status, 404);
}

// ── 模型状态 ──────────────────────────────────────────────────────

#[tokio::test]
async fn reads_model_states() {
    let (router, _, token) = setup().await;
    let resp = router
        .handle("GET", "/admin/accounts/4/model-states", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body["items"].as_array().unwrap().len(), 1);
    assert_eq!(resp.body["items"][0]["upstream_model"], "grok-imagine-image");

    let resp = router
        .handle("GET", "/admin/accounts/999/model-states", Some(&bearer(&token)), None)
        .await;
    assert_eq!(resp.status, 404);
}
