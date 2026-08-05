//! G4-P1 Admin 认证集成测试（迁移 Go `adminauth/service_test.go` + token 语义）。
//!
//! Go 用 SQLite repo；Rust 用内存 fake 实现 `AdminRepository` / `AdminSessionRepository`
//! / `RateLimiter`，逐用例对齐语义（轮换冲突 / 撤销 / 改密 / 限流 / 并发恰好一次）。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use grok_admin::repos::{AdminRepository, AdminSessionRepository, RateLimiter};
use grok_admin::{
    hash_password, Admin, AdminAuthService, AdminError, AdminResult, Session, TokenService,
};

fn secret() -> String {
    "12345678901234567890123456789012".to_string()
}

fn new_service(
    store: Arc<Store>,
    access_ttl: Duration,
    refresh_ttl: Duration,
) -> AdminAuthService {
    AdminAuthService::new(
        Arc::new(FakeAdminRepo::new(store.clone())),
        Arc::new(FakeSessionRepo::new(store)),
        TokenService::new(&secret()),
        access_ttl,
        refresh_ttl,
    )
}

async fn bootstrap_login(service: &AdminAuthService) -> (Admin, grok_admin::Tokens) {
    service
        .bootstrap("admin", "password123")
        .await
        .expect("bootstrap");
    service
        .login("admin", "password123", "127.0.0.1")
        .await
        .expect("login")
}

// ── 内存 fake ─────────────────────────────────────────────────────

/// 共享存储：admins + sessions（模拟关系表）。
#[derive(Default)]
pub struct Store {
    admins: Mutex<Vec<Admin>>,
    sessions: Mutex<Vec<Session>>,
    next_admin_id: AtomicI64,
    next_session_id: AtomicI64,
}

pub struct FakeAdminRepo {
    store: Arc<Store>,
}

pub struct FakeSessionRepo {
    store: Arc<Store>,
}

impl FakeAdminRepo {
    fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl FakeSessionRepo {
    fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl AdminRepository for FakeAdminRepo {
    async fn count(&self) -> AdminResult<i64> {
        Ok(self.store.admins.lock().unwrap().len() as i64)
    }

    async fn create(&self, mut admin: Admin) -> AdminResult<Admin> {
        let mut admins = self.store.admins.lock().unwrap();
        admin.id = self.store.next_admin_id.fetch_add(1, Ordering::SeqCst) + 1;
        admins.push(admin.clone());
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
        let store = &self.store;
        {
            let mut admins = store.admins.lock().unwrap();
            if let Some(admin) = admins.iter_mut().find(|a| a.id == admin_id) {
                admin.password_hash = password_hash.to_string();
                admin.updated_at = Utc::now();
            }
        }
        store
            .sessions
            .lock()
            .unwrap()
            .retain(|s| s.admin_id != admin_id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl AdminSessionRepository for FakeSessionRepo {
    async fn get_by_token_hash(&self, token_hash: &str) -> AdminResult<Option<Session>> {
        Ok(self
            .store
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.refresh_token_hash == token_hash)
            .cloned())
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
        let mut sessions = self.store.sessions.lock().unwrap();
        session.id = self.store.next_session_id.fetch_add(1, Ordering::SeqCst) + 1;
        sessions.push(session.clone());
        Ok(session)
    }

    async fn rotate(
        &self,
        session_id: i64,
        old_token_hash: &str,
        new_token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AdminResult<bool> {
        let mut sessions = self.store.sessions.lock().unwrap();
        let Some(session) = sessions.iter_mut().find(|s| s.id == session_id) else {
            return Ok(false);
        };
        if session.refresh_token_hash != old_token_hash {
            return Ok(false);
        }
        session.refresh_token_hash = new_token_hash.to_string();
        session.expires_at = expires_at;
        Ok(true)
    }

    async fn revoke(&self, id: i64) -> AdminResult<()> {
        self.store.sessions.lock().unwrap().retain(|s| s.id != id);
        Ok(())
    }
}

struct RejectingLimiter;

#[async_trait::async_trait]
impl RateLimiter for RejectingLimiter {
    async fn allow(&self, _key: &str, _limit: i32, _now: DateTime<Utc>) -> AdminResult<bool> {
        Ok(false)
    }
}

struct FailingAdminRepo;

#[async_trait::async_trait]
impl AdminRepository for FailingAdminRepo {
    async fn count(&self) -> AdminResult<i64> {
        Ok(0)
    }
    async fn create(&self, _admin: Admin) -> AdminResult<Admin> {
        Err(AdminError::RuntimeUnavailable("noop".into()))
    }
    async fn get_by_username(&self, _username: &str) -> AdminResult<Option<Admin>> {
        Err(AdminError::RuntimeUnavailable("database unavailable".into()))
    }
    async fn get_by_id(&self, _id: i64) -> AdminResult<Option<Admin>> {
        Err(AdminError::RuntimeUnavailable("database unavailable".into()))
    }
    async fn update_password_and_revoke_sessions(
        &self,
        _admin_id: i64,
        _password_hash: &str,
    ) -> AdminResult<()> {
        Ok(())
    }
}

struct NoopSessions;

#[async_trait::async_trait]
impl AdminSessionRepository for NoopSessions {
    async fn get_by_token_hash(&self, _h: &str) -> AdminResult<Option<Session>> {
        Ok(None)
    }
    async fn get_by_id(&self, _id: i64) -> AdminResult<Option<Session>> {
        Ok(None)
    }
    async fn create(&self, _s: Session) -> AdminResult<Session> {
        Err(AdminError::RuntimeUnavailable("noop".into()))
    }
    async fn rotate(
        &self,
        _id: i64,
        _o: &str,
        _n: &str,
        _e: DateTime<Utc>,
    ) -> AdminResult<bool> {
        Ok(true)
    }
    async fn revoke(&self, _id: i64) -> AdminResult<()> {
        Ok(())
    }
}

// ── 用例 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn refresh_token_rotation_and_logout() {
    let service = new_service(Arc::new(Store::default()), Duration::minutes(1), Duration::hours(1));
    let (_admin, tokens) = bootstrap_login(&service).await;

    let rotated = service.refresh(&tokens.refresh_token).await.expect("refresh");

    // 旧 refresh token 已轮换 → 失效
    let err = service.refresh(&tokens.refresh_token).await.expect_err("old refresh usable");
    assert_eq!(err, AdminError::InvalidSession);

    // 注销后 access / refresh 均失效
    service.logout(&rotated.refresh_token).await.expect("logout");
    let err = service
        .authenticate_access(&rotated.access_token)
        .await
        .expect_err("access usable after logout");
    assert_eq!(err, AdminError::InvalidSession);
    let err = service
        .refresh(&rotated.refresh_token)
        .await
        .expect_err("refresh usable after logout");
    assert_eq!(err, AdminError::InvalidSession);
}

#[tokio::test]
async fn change_password_revokes_all_sessions() {
    let store = Arc::new(Store::default());
    let service = new_service(store.clone(), Duration::minutes(1), Duration::hours(1));
    let (admin, tokens) = bootstrap_login(&service).await;

    service
        .change_password(admin.id, "password123", "password456")
        .await
        .expect("change password");

    let err = service
        .authenticate_access(&tokens.access_token)
        .await
        .expect_err("access usable after password change");
    assert_eq!(err, AdminError::InvalidSession);
    let err = service
        .refresh(&tokens.refresh_token)
        .await
        .expect_err("refresh usable after password change");
    assert_eq!(err, AdminError::InvalidSession);

    let err = service
        .login("admin", "password123", "127.0.0.1")
        .await
        .expect_err("old password still works");
    assert_eq!(err, AdminError::InvalidCredentials);
    service
        .login("admin", "password456", "127.0.0.1")
        .await
        .expect("new password login");
}

#[tokio::test]
async fn login_rate_limiter_failure_is_enforced() {
    let mut service = new_service(Arc::new(Store::default()), Duration::minutes(1), Duration::hours(1));
    service.set_login_rate_limiter(Arc::new(RejectingLimiter));
    let err = service
        .login("admin", "password123", "127.0.0.1")
        .await
        .expect_err("rate limited");
    assert_eq!(err, AdminError::LoginRateLimited);
}

#[tokio::test]
async fn login_distinguishes_persistence_failure() {
    let service = AdminAuthService::new(
        Arc::new(FailingAdminRepo),
        Arc::new(NoopSessions),
        TokenService::new(&secret()),
        Duration::minutes(1),
        Duration::hours(1),
    );
    let err = service
        .login("admin", "password123", "127.0.0.1")
        .await
        .expect_err("persistence failure");
    assert!(matches!(err, AdminError::RuntimeUnavailable(_)));
}

#[tokio::test]
async fn concurrent_refresh_allows_exactly_one_rotation() {
    let store = Arc::new(Store::default());
    let service = Arc::new(new_service(
        store.clone(),
        Duration::minutes(1),
        Duration::hours(1),
    ));
    let (_admin, tokens) = bootstrap_login(&service).await;

    let service_a = service.clone();
    let service_b = service.clone();
    let token_a = tokens.refresh_token.clone();
    let token_b = tokens.refresh_token.clone();
    let (ra, rb) = tokio::join!(
        async move { service_a.refresh(&token_a).await },
        async move { service_b.refresh(&token_b).await },
    );

    let (success, invalid) = match (&ra, &rb) {
        (Ok(_), Err(AdminError::InvalidSession)) => (1, 1),
        (Err(AdminError::InvalidSession), Ok(_)) => (1, 1),
        (Ok(_), Ok(_)) => (2, 0),
        (Err(_), Err(_)) => (0, 2),
        _ => panic!("unexpected concurrent refresh results"),
    };
    assert_eq!((success, invalid), (1, 1), "exactly one rotation must win");

    // 胜者 token 可继续轮换。
    let winner = match (&ra, &rb) {
        (Ok(t), _) => t,
        (_, Ok(t)) => t,
        _ => unreachable!(),
    };
    service.refresh(&winner.refresh_token).await.expect("winner refresh usable");
}

#[tokio::test]
async fn access_token_round_trip_and_tamper_rejection() {
    let tokens = TokenService::new(&secret());
    let (token, expires_at) = tokens
        .create_access_token(7, 42, Duration::minutes(1))
        .expect("create");
    let identity = tokens.parse_access_token(&token).expect("parse");
    assert_eq!(identity.admin_id, 7);
    assert_eq!(identity.session_id, 42);
    assert!(expires_at > Utc::now());

    // 篡改签名 → 拒绝
    let tampered = format!("{}x", &token[..token.len() - 1]);
    assert_eq!(tokens.parse_access_token(&tampered), Err(AdminError::InvalidSession));

    // 篡改 payload → 签名校验失败
    let parts: Vec<&str> = token.split('.').collect();
    let tampered_payload = format!("{}.{}.{}", parts[0], "eyJpbnZhbGlkIjoieSI", parts[2]);
    assert_eq!(
        tokens.parse_access_token(&tampered_payload),
        Err(AdminError::InvalidSession)
    );
}

#[tokio::test]
async fn expired_access_token_rejected() {
    let tokens = TokenService::new(&secret());
    // 负 TTL → 立即过期
    let (token, _) = tokens
        .create_access_token(7, 42, Duration::seconds(-1))
        .expect("create");
    assert_eq!(tokens.parse_access_token(&token), Err(AdminError::InvalidSession));
}

#[tokio::test]
async fn expired_refresh_session_rejected() {
    let store = Arc::new(Store::default());
    let service = new_service(store.clone(), Duration::minutes(1), Duration::hours(1));
    let (admin, tokens) = bootstrap_login(&service).await;

    // 直接注入已过期会话（模拟 DB 中 session 过期）
    {
        let mut sessions = store.sessions.lock().unwrap();
        for session in sessions.iter_mut() {
            if session.admin_id == admin.id {
                session.expires_at = Utc::now() - Duration::seconds(1);
            }
        }
    }
    let err = service
        .refresh(&tokens.refresh_token)
        .await
        .expect_err("expired refresh session");
    assert_eq!(err, AdminError::InvalidSession);
    let err = service
        .authenticate_access(&tokens.access_token)
        .await
        .expect_err("expired session access");
    assert_eq!(err, AdminError::InvalidSession);
}

#[tokio::test]
async fn bootstrap_requires_username_and_8_char_password() {
    let store = Arc::new(Store::default());
    let service = new_service(store.clone(), Duration::minutes(1), Duration::hours(1));
    let err = service
        .bootstrap("", "password123")
        .await
        .expect_err("empty username");
    assert_eq!(err, AdminError::BootstrapRequired);
    let err = service
        .bootstrap("admin", "short")
        .await
        .expect_err("short password");
    assert_eq!(err, AdminError::BootstrapRequired);
    service.bootstrap("admin", "password123").await.expect("ok");
    // 已有管理员 → 幂等返回 Ok
    service.bootstrap("other", "password123").await.expect("idempotent");
}

#[tokio::test]
async fn change_password_validation_and_wrong_current() {
    let store = Arc::new(Store::default());
    let service = new_service(store.clone(), Duration::minutes(1), Duration::hours(1));
    let (admin, _tokens) = bootstrap_login(&service).await;
    let err = service
        .change_password(admin.id, "password123", "short")
        .await
        .expect_err("short new password");
    assert_eq!(err, AdminError::InvalidPassword);
    let err = service
        .change_password(admin.id, "wrong-current", "password456")
        .await
        .expect_err("wrong current password");
    assert_eq!(err, AdminError::InvalidCredentials);
}

#[tokio::test]
async fn login_wrong_password_is_invalid_credentials() {
    let store = Arc::new(Store::default());
    let service = new_service(store.clone(), Duration::minutes(1), Duration::hours(1));
    service.bootstrap("admin", "password123").await.expect("bootstrap");
    let err = service
        .login("admin", "wrong-password", "127.0.0.1")
        .await
        .expect_err("wrong password");
    assert_eq!(err, AdminError::InvalidCredentials);
    let err = service
        .login("no-such-user", "password123", "127.0.0.1")
        .await
        .expect_err("unknown user");
    assert_eq!(err, AdminError::InvalidCredentials);
}

#[tokio::test]
async fn auth_guard_parses_bearer_and_rejects_missing() {
    use grok_admin::{authenticate_bearer, bearer_token};

    assert_eq!(bearer_token("Bearer abc.def"), Some("abc.def"));
    assert_eq!(bearer_token("Basic abc"), None);

    let store = Arc::new(Store::default());
    let service = new_service(store.clone(), Duration::minutes(1), Duration::hours(1));
    let (_admin, tokens) = bootstrap_login(&service).await;

    let ctx = authenticate_bearer(&service, &format!("Bearer {}", tokens.access_token))
        .await
        .expect("guard ok");
    assert_eq!(ctx.admin.username, "admin");
    assert!(ctx.session_id > 0);

    let err = authenticate_bearer(&service, "Basic abc").await.expect_err("no bearer");
    assert_eq!(err, AdminError::InvalidSession);
    let err = authenticate_bearer(&service, "Bearer invalid.token.value")
        .await
        .expect_err("invalid token");
    assert_eq!(err, AdminError::InvalidSession);
}

#[test]
fn password_hash_is_bcrypt() {
    let hash = hash_password("password123").unwrap();
    assert!(hash.starts_with("$2"));
}
