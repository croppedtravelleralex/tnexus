//! ChatEngine — grok Web chat/OCR 编排（docs/39d §4.3 时序，39 主文档 §4.2）。
//!
//! 流程（G1 单池 + 内存 lease + mock/simplified bridge）：
//!   pool.select → egress.acquire(grok_web) → prepare attachments
//!     → build_web_chat_payload(ocr 控制 enableImageGeneration)
//!     → bridge.fetch_chat → 文本
//! audit 异步写（`grok-audit`）；dispatch 记账（成功/失败 → 冷却）。

use std::sync::Arc;
use std::time::Duration;

use grok_audit::AuditSink;
use grok_domain::egress::Scope;
use grok_domain::SsoTokenProvider;
use grok_egress::{GateId, LeaseManager};
use grok_pool::SharedPool;

use crate::attachments::prepare_file_attachments;
use crate::bridge::BridgeClient;
use crate::chat::{build_web_chat_payload, DEFAULT_OCR_SYSTEM_PROMPT};
use grok_domain::ProviderError;
use grok_domain::{ChatBackend, ChatRequest};

/// egress lease 获取超时（G1 单槽 web scope；时长可经构造覆盖）。
const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(5);

/// grok Web chat 引擎（依赖池 / lease / bridge 均可注入，便于单测）。
pub struct ChatEngine {
    pool: SharedPool,
    lease: Arc<dyn LeaseManager>,
    bridge: Arc<dyn BridgeClient>,
    audit: Option<Arc<AuditSink>>,
    /// 无 chrome 直连路径：按账号取 sso token（bridge 模式为 None）。
    sso: Option<Arc<dyn SsoTokenProvider>>,
    lease_duration: Duration,
}

impl ChatEngine {
    /// 组装引擎。`audit` 可传 None（G1 未接入时）。
    pub fn new(
        pool: SharedPool,
        lease: Arc<dyn LeaseManager>,
        bridge: Arc<dyn BridgeClient>,
        audit: Option<Arc<AuditSink>>,
    ) -> Self {
        Self {
            pool,
            lease,
            bridge,
            audit,
            sso: None,
            lease_duration: DEFAULT_LEASE_DURATION,
        }
    }

    /// 注入 sso token 提供者（无 chrome 直连路径）。bridge 模式不注入。
    pub fn with_sso(mut self, sso: Arc<dyn SsoTokenProvider>) -> Self {
        self.sso = Some(sso);
        self
    }

    /// 注入可选的 sso token 提供者（直连模式 Some / bridge 模式 None）。
    pub fn with_sso_opt(mut self, sso: Option<Arc<dyn SsoTokenProvider>>) -> Self {
        self.sso = sso;
        self
    }

    /// 覆盖 lease 获取超时（测试/生产调优）。
    pub fn with_lease_duration(mut self, d: Duration) -> Self {
        self.lease_duration = d;
        self
    }

    /// 执行一次 chat/OCR，返回上游文本。
    pub async fn chat(&self, req: &ChatRequest) -> Result<String, ProviderError> {
        let account_id = self
            .pool
            .select(None)
            .await
            .ok_or(ProviderError::NoAvailableAccount)?;

        // egress lease（grok_web scope）。失败 → 记 dispatch 失败并返回。
        let _lease = match self
            .lease
            .acquire(
                Scope::GrokWeb,
                GateId::from(account_id.to_string()),
                self.lease_duration,
            )
            .await
        {
            Ok(lease) => lease,
            Err(e) => {
                self.pool.dispatch_failure(account_id).await;
                return Err(ProviderError::Lease(e.to_string()));
            }
        };

        // 附件准备 + payload（OCR 控制 enableImageGeneration）。
        // 附件失败（bridge 下载空/失败）视为上游/provider 失败，应冷却该账号。
        let attachments = match prepare_file_attachments(&req.images, self.bridge.as_ref()).await {
            Ok(a) => a,
            Err(e) => {
                self.pool.dispatch_failure(account_id).await;
                self.record_audit(req, account_id, false);
                return Err(e);
            }
        };
        let system_prompt = req
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_OCR_SYSTEM_PROMPT);
        let payload = build_web_chat_payload(&req.prompt, &attachments, req.ocr, system_prompt);

        // 无 chrome 直连：按账号取 sso token；bridge 模式跳过。
        let sso_token = match &self.sso {
            Some(provider) => Some(provider.sso_token(account_id).await?),
            None => None,
        };
        let result = self.bridge.fetch_chat(&payload, sso_token.as_deref()).await;

        match result {
            Ok(text) => {
                self.pool.dispatch_success(account_id).await;
                self.record_audit(req, account_id, true);
                Ok(text)
            }
            Err(e) => {
                self.pool.dispatch_failure(account_id).await;
                self.record_audit(req, account_id, false);
                Err(e)
            }
        }
    }

    /// 记录审计（异步、非阻塞；sink 为 None 时跳过）。
    fn record_audit(&self, req: &ChatRequest, account_id: i64, ok: bool) {
        if let Some(sink) = &self.audit {
            let mut audit = grok_audit::CreateAudit {
                event_id: grok_audit::CreateAudit::new_event_id(),
                request_id: req.request_id.clone(),
                account_id: Some(account_id),
                provider: "grok_web".into(),
                operation: grok_audit::Operation::Chat,
                status_code: if ok { 200 } else { 502 },
                media_input_images: req.images.len() as i64,
                streaming: false,
                ..Default::default()
            };
            if req.ocr {
                audit.model_public_id = Some(crate::chat::ALIAS_OCR.to_string());
                audit.model_upstream_model = Some(crate::chat::UPSTREAM_OCR_MODEL.to_string());
            }
            let _ = sink.record(audit);
        }
    }
}

#[async_trait::async_trait]
impl ChatBackend for ChatEngine {
    async fn chat(&self, req: &ChatRequest) -> Result<String, grok_domain::ProviderError> {
        ChatEngine::chat(self, req).await
    }
}

// ---- 测试 ----
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{BridgeClient, MockBridgeClient};
    use grok_domain::{Account, AuthStatus, Provider};
    use grok_egress::InMemoryLeaseManager;
    use std::collections::HashMap;

    fn sample_account(id: i64) -> Account {
        Account {
            id,
            identity_key: format!("web-{id}"),
            provider: Provider::GrokWeb,
            enabled: true,
            auth_status: AuthStatus::Active,
            priority: 0,
            observed_model: None,
            ..Default::default()
        }
    }

    fn engine(pool: SharedPool, bridge: Arc<dyn BridgeClient>) -> ChatEngine {
        let lease = Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
        ChatEngine::new(pool, lease, bridge, None)
    }

    const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn req() -> ChatRequest {
        ChatRequest {
            prompt: "[user]\n描述这张图".to_string(),
            images: vec![DATA_URI.to_string()],
            ocr: true,
            system_prompt: None,
            request_id: "req-1".to_string(),
        }
    }

    #[tokio::test]
    async fn chat_returns_text_and_records_dispatch() {
        let pool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(7)]).await;
        let mut mock = MockBridgeClient::new();
        mock.chat_text = "图中文字是「你好」".to_string();
        let concrete: Arc<MockBridgeClient> = Arc::new(mock);
        let bridge: Arc<dyn BridgeClient> = concrete.clone();
        let e = ChatEngine::new(
            pool,
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)])),
            bridge,
            None,
        );
        let text = e.chat(&req()).await.unwrap();
        assert_eq!(text, "图中文字是「你好」");
        // OCR golden：bridge 收到的 payload 禁生图 + fast 模型。
        let got = concrete.last_chat_payload.lock().await;
        let payload = got.as_ref().unwrap();
        assert_eq!(payload["model"], crate::chat::UPSTREAM_OCR_MODEL);
        assert_eq!(payload["enableImageGeneration"], false);
        assert_eq!(payload["enableImageStreaming"], false);
    }

    #[tokio::test]
    async fn no_available_account_errors() {
        let pool = grok_pool::SimplifiedPool::new(); // empty
        let bridge = Arc::new(MockBridgeClient::new());
        let e = engine(Arc::new(pool), bridge);
        let r = e.chat(&req()).await;
        assert!(matches!(r, Err(ProviderError::NoAvailableAccount)));
    }

    #[tokio::test]
    async fn lease_timeout_errors() {
        // web scope 上限 1：先占满一个长 lease，再用短时长请求 → Timeout。
        // 这里用 with_lease_duration 极小以稳定触发。
        let pool = grok_pool::SimplifiedPool::new();
        pool.load_in_memory(vec![sample_account(1)]).await;
        pool.pin(1).await;
        let lease = Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 1)]));
        let bridge = Arc::new(MockBridgeClient::new());

        // 占住 engine 将用的 gate（账号 1 的 gate "1"）：acquire 一个长 lease 并持住。
        let held = lease
            .acquire(Scope::GrokWeb, "1".into(), Duration::from_secs(60))
            .await
            .unwrap();

        let e = ChatEngine::new(Arc::new(pool), lease, bridge, None)
            .with_lease_duration(Duration::from_millis(20));
        let r = e.chat(&req()).await;
        assert!(
            matches!(r, Err(ProviderError::Lease(_))),
            "expected lease timeout, got {r:?}"
        );
        held.release();
    }

    #[tokio::test]
    async fn empty_bridge_bytes_counts_as_failure_and_cooldown() {
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(2)]).await;
        // bridge 返回空字节 → attachment 阶段报错 → dispatch_failure → 冷却
        let bridge = Arc::new(MockBridgeClient::new()); // images 空
                                                        // 用远端 URL 使 bridge 找不到字节。
        let mut reqc = req();
        reqc.images = vec!["https://x.com/missing.png".to_string()];
        let e = engine(pool.clone(), bridge);
        let r = e.chat(&reqc).await;
        assert!(r.is_err());
        assert!(pool.in_cooldown(2).await, "failure should cooldown account");
    }

    #[tokio::test]
    async fn non_ocr_remote_image_payload_enables_generation() {
        let pool = grok_pool::SimplifiedPool::new();
        pool.load_in_memory(vec![sample_account(3)]).await;
        let mut b = MockBridgeClient::new();
        b.images = HashMap::from([(
            "https://x.com/a.png".to_string(),
            vec![0x89, 0x50, 0x4E, 0x47],
        )]);
        b.chat_text = "hi".to_string();
        let concrete: Arc<MockBridgeClient> = Arc::new(b);
        let bridge: Arc<dyn BridgeClient> = concrete.clone();
        let mut reqc = req();
        reqc.ocr = false;
        reqc.images = vec!["https://x.com/a.png".to_string()];
        let e = engine(Arc::new(pool), bridge);
        e.chat(&reqc).await.unwrap();
        let got = concrete.last_chat_payload.lock().await;
        let payload = got.as_ref().unwrap();
        assert_eq!(payload["model"], "grok-chat");
        assert_eq!(payload["enableImageGeneration"], true);
    }
}
