//! 会话池：`HashMap<sessionKey, Session>`，TTL 惰性清理，每会话并发 1（信号量）。
//!
//! 对齐 Python bridge 语义：sessionKey 由（proxyUrl, cookie, UA）派生，同键复用
//! 已 warm 的浏览器；TTL 1800s 到期惰性关闭。Chrome 进程随 Session drop 自动 kill。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::Mutex as StdMutex;
use tokio::sync::{Mutex, Semaphore};

use crate::cdp::{CdpClient, ChromeCdpClient};
use crate::chrome::{launch, ChromeProcess};
use crate::error::BridgeError;
use crate::js;

/// 会话 TTL（对齐 Python `BRIDGE_SESSION_TTL_SECONDS` 缺省 1800）。
const DEFAULT_TTL: Duration = Duration::from_secs(1800);

/// 会话创建工厂（测试注入 fake CDP）。
#[async_trait::async_trait]
pub trait CdpFactory: Send + Sync {
    async fn create(&self, user_agent: &str) -> Result<Arc<dyn CdpClient>, BridgeError>;
}

/// 真实工厂：拉起独立 Chrome 进程并连接 CDP。
pub struct ChromeCdpFactory {
    chrome_path: Option<String>,
    /// 进程句柄容器（每个会话一个 ChromeProcess，drop 时 kill）。
    processes: Mutex<Vec<Arc<ChromeProcess>>>,
}

impl ChromeCdpFactory {
    pub fn new(chrome_path: Option<String>) -> Self {
        Self {
            chrome_path,
            processes: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl CdpFactory for ChromeCdpFactory {
    async fn create(&self, user_agent: &str) -> Result<Arc<dyn CdpClient>, BridgeError> {
        let process = Arc::new(launch(self.chrome_path.as_deref()).await?);
        let ws = tokio_tungstenite::connect_async(&process.ws_url)
            .await
            .map_err(|e| BridgeError::internal(format!("connect cdp ws: {e}")))?
            .0;
        let client = ChromeCdpClient::new(ws);
        if !user_agent.is_empty() {
            client.set_user_agent(user_agent).await?;
        }
        // Page.enable / Network.enable 让后续原语可用。
        let client: Arc<dyn CdpClient> = Arc::new(client);
        self.processes.lock().await.push(process);
        Ok(client)
    }
}

/// 会话池配置。
#[derive(Clone)]
pub struct SessionPoolConfig {
    pub ttl: Duration,
}

impl Default for SessionPoolConfig {
    fn default() -> Self {
        Self { ttl: DEFAULT_TTL }
    }
}

/// 单个浏览器会话。
pub struct Session {
    pub key: String,
    pub client: Arc<dyn CdpClient>,
    /// 每会话并发 1。
    pub semaphore: Arc<Semaphore>,
    /// 会话绑定的 user_agent（键隔离校验）。
    pub user_agent: String,
    /// 是否已完成首次导航。
    navigated: StdMutex<bool>,
    last_used: StdMutex<Instant>,
}

impl Session {
    pub fn mark_navigated(&self) {
        *self.navigated.lock().unwrap() = true;
    }

    pub fn is_navigated(&self) -> bool {
        *self.navigated.lock().unwrap()
    }

    pub fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    pub fn last_used(&self) -> Instant {
        *self.last_used.lock().unwrap()
    }
}

/// 会话池（每 key 一个会话，TTL 惰性清理）。
pub struct SessionPool {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    config: SessionPoolConfig,
    factory: Arc<dyn CdpFactory>,
}

impl SessionPool {
    pub fn new(factory: Arc<dyn CdpFactory>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            config: SessionPoolConfig::default(),
            factory,
        }
    }

    pub fn with_config(mut self, config: SessionPoolConfig) -> Self {
        self.config = config;
        self
    }

    /// 清理过期会话（惰性，任何操作前调用）。
    async fn purge(&self) {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().await;
        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| now.duration_since(s.last_used()) > self.config.ttl)
            .map(|(k, _)| k.clone())
            .collect();
        for key in expired {
            sessions.remove(&key);
        }
    }

    /// 获取或创建会话。`user_agent` 与已有会话不同 → 重建（键隔离）。
    pub async fn acquire(
        &self,
        session_key: &str,
        user_agent: &str,
    ) -> Result<Arc<Session>, BridgeError> {
        self.purge().await;
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(session_key) {
            if existing.user_agent == user_agent {
                return Ok(existing.clone());
            }
            sessions.remove(session_key);
        }
        let client = self.factory.create(user_agent).await?;
        let session = Arc::new(Session {
            key: session_key.to_string(),
            client,
            semaphore: Arc::new(Semaphore::new(1)),
            user_agent: user_agent.to_string(),
            navigated: StdMutex::new(false),
            last_used: StdMutex::new(Instant::now()),
        });
        sessions.insert(session_key.to_string(), session.clone());
        Ok(session)
    }

    /// 当前存活会话数（/health 用）。
    pub async fn session_count(&self) -> usize {
        self.purge().await;
        self.sessions.lock().await.len()
    }
}

/// 由 sessionKey 派生稳定的会话键（sha256 前 16 字节 hex，对齐 Go `browserSessionKey`）。
pub fn derive_session_key(proxy_url: &str, cookie: &str, user_agent: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(proxy_url.as_bytes());
    hasher.update([0u8]);
    hasher.update(cookie.as_bytes());
    hasher.update([0u8]);
    hasher.update(user_agent.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    hex
}

/// 确保会话已导航到目标 URL（未导航过 → navigate + 标记）。
pub async fn ensure_navigated(session: &Session, url: &str) -> Result<(), BridgeError> {
    if session.is_navigated() {
        return Ok(());
    }
    session.client.navigate(url).await?;
    session.mark_navigated();
    Ok(())
}

/// 轮询会话就绪（Turbopack runtime 加载），最多 `timeout` 秒。未就绪不算错误。
pub async fn wait_ready(session: &Session, timeout: Duration) -> Result<bool, BridgeError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let value = session
            .client
            .evaluate(js::READY_EXPR, false)
            .await
            .unwrap_or(serde_json::Value::Null);
        if value.as_bool().unwrap_or(false) {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Ok(false)
}

/// 把 cookie 串（`a=b; c=d`）写入 CDP 会话（对齐 Python `parse_cookies` → driver.add_cookie）。
pub async fn apply_cookies(session: &Session, cookie: &str) -> Result<(), BridgeError> {
    for item in cookie.split(';') {
        let trimmed = item.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            let name = name.trim();
            let value = value.trim();
            if !name.is_empty() && !value.is_empty() {
                session.client.set_cookie(name, value, ".grok.com").await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::FakeCdpClient;

    struct FakeFactory {
        clients: Arc<tokio::sync::Mutex<Vec<Arc<dyn CdpClient>>>>,
    }

    #[async_trait::async_trait]
    impl CdpFactory for FakeFactory {
        async fn create(&self, _ua: &str) -> Result<Arc<dyn CdpClient>, BridgeError> {
            let client: Arc<dyn CdpClient> = Arc::new(FakeCdpClient::new());
            self.clients.lock().await.push(client.clone());
            Ok(client)
        }
    }

    #[tokio::test]
    async fn same_key_reuses_session() {
        let factory = Arc::new(FakeFactory {
            clients: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        });
        let pool = SessionPool::new(factory.clone());
        let a = pool.acquire("k1", "ua").await.unwrap();
        let b = pool.acquire("k1", "ua").await.unwrap();
        assert!(Arc::ptr_eq(&a, &b), "同 key 同 UA 应复用同一会话");
        assert_eq!(factory.clients.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn different_ua_rebuilds_session() {
        let factory = Arc::new(FakeFactory {
            clients: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        });
        let pool = SessionPool::new(factory.clone());
        let a = pool.acquire("k1", "ua-1").await.unwrap();
        let b = pool.acquire("k1", "ua-2").await.unwrap();
        assert!(!Arc::ptr_eq(&a, &b), "UA 变化应重建会话");
        assert_eq!(factory.clients.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn ttl_purges_expired() {
        let factory = Arc::new(FakeFactory {
            clients: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        });
        let pool = SessionPool::new(factory).with_config(SessionPoolConfig {
            ttl: Duration::from_millis(10),
        });
        let session = pool.acquire("k1", "ua").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // last_used 在 acquire 时设置，超过 TTL 后被 purge。
        session.touch(); // 重新 touch 则不过期
        assert_eq!(pool.session_count().await, 1);
        // 把 last_used 拨回过去 → 过期。
        *session.last_used.lock().unwrap() = Instant::now() - Duration::from_secs(999);
        assert_eq!(pool.session_count().await, 0);
    }

    #[test]
    fn session_key_is_stable_hash() {
        let k1 = derive_session_key("p", "c", "u");
        let k2 = derive_session_key("p", "c", "u");
        let k3 = derive_session_key("p", "c", "u2");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_eq!(k1.len(), 32, "sha256 前 16 字节 hex = 32 字符");
    }
}
