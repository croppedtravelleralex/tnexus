//! ChatEngine — grok Web chat/OCR 编排（docs/39d §4.3 时序，39 主文档 §4.2）。
//!
//! 流程（G1 单池 + 内存 lease + mock/simplified bridge）：
//!   pool.select → egress.acquire(grok_web) → prepare attachments
//!     → build_web_chat_payload(ocr 控制 enableImageGeneration)
//!     → bridge.fetch_chat → 文本
//! audit 异步写（`grok-audit`）；dispatch 记账（成功/失败 → 冷却）。
//!
//! 跨账号重试（item 1）：`chat_outcome` 在可重试上游错误（429 / 403）时
//! 自动换账号重试，最多 `retry_max` 次（env `GROK_CHAT_RETRY_MAX`，默认 2，硬上限 8）。

use std::sync::Arc;
use std::time::Duration;

use grok_audit::AuditSink;
use grok_domain::egress::Scope;
use grok_domain::SsoTokenProvider;
use grok_egress::{GateId, LeaseManager};
use grok_pool::SharedPool;
use tokio::sync::Semaphore;

use crate::attachments::prepare_file_attachments;
use crate::bridge::BridgeClient;
use crate::chat::{build_web_chat_payload, DEFAULT_OCR_SYSTEM_PROMPT};
use grok_domain::ProviderError;
use grok_domain::{ChatBackend, ChatOutcome, ChatRequest, ChatStreamEvent};

/// egress lease 获取超时（G1 单槽 web scope；时长可经构造覆盖）。
const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(5);
/// 跨账号重试默认次数（可通过 GROK_CHAT_RETRY_MAX 覆盖）。
const DEFAULT_RETRY_MAX: usize = 2;
/// 跨账号重试硬上限（防止失控重试拖垮号池）。
const HARD_RETRY_CAP: usize = 8;
const DEFAULT_OCR_GLOBAL_CONCURRENCY: usize = 4;

/// 账号健康写回端口（PG 持久化冷却状态）。
///
/// `None` 时跳过写回（单测 / 无 DB 场景）；写回失败仅 warn 日志，不阻断 chat 响应。
#[async_trait::async_trait]
pub trait AccountHealthSink: Send + Sync {
    /// 记录限速/拒绝失败：写入冷却截止时间与最后错误原因。
    async fn record_rate_limit_failure(
        &self,
        account_id: i64,
        cooldown_until: chrono::DateTime<chrono::Utc>,
        reason: &str,
    );
    /// 记录成功：清零失败计数与最后错误。
    async fn record_success(&self, account_id: i64);
}

/// grok Web chat 引擎（依赖池 / lease / bridge 均可注入，便于单测）。
pub struct ChatEngine {
    pool: SharedPool,
    lease: Arc<dyn LeaseManager>,
    bridge: Arc<dyn BridgeClient>,
    audit: Option<Arc<AuditSink>>,
    /// 无 chrome 直连路径：按账号取 sso token（bridge 模式为 None）。
    sso: Option<Arc<dyn SsoTokenProvider>>,
    lease_duration: Duration,
    /// 跨账号重试上限（含首次尝试）。
    retry_max: usize,
    /// PG 健康写回（可选；None 时只写内存冷却）。
    health_sink: Option<Arc<dyn AccountHealthSink>>,
    ocr_global_gate: Arc<Semaphore>,
}

impl ChatEngine {
    /// 组装引擎。`audit` 可传 None（G1 未接入时）。
    pub fn new(
        pool: SharedPool,
        lease: Arc<dyn LeaseManager>,
        bridge: Arc<dyn BridgeClient>,
        audit: Option<Arc<AuditSink>>,
    ) -> Self {
        // 从环境变量读取重试次数（默认 2，硬上限 8）
        let retry_max = std::env::var("GROK_CHAT_RETRY_MAX")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_RETRY_MAX)
            .min(HARD_RETRY_CAP);
        let ocr_conc = std::env::var("GROK_OCR_GLOBAL_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_OCR_GLOBAL_CONCURRENCY)
            .max(1);
        Self {
            pool,
            lease,
            bridge,
            audit,
            sso: None,
            lease_duration: DEFAULT_LEASE_DURATION,
            retry_max,
            health_sink: None,
            ocr_global_gate: Arc::new(Semaphore::new(ocr_conc)),
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

    /// 覆盖跨账号重试上限（最大 `HARD_RETRY_CAP`；主要用于测试）。
    pub fn with_retry_max(mut self, n: usize) -> Self {
        self.retry_max = n.min(HARD_RETRY_CAP);
        self
    }

    /// 注入 PG 账号健康写回实现（None 时只维护内存冷却）。
    pub fn with_health_sink(mut self, sink: Arc<dyn AccountHealthSink>) -> Self {
        self.health_sink = Some(sink);
        self
    }

    /// 执行一次 chat/OCR，返回上游文本。
    pub async fn chat(&self, req: &ChatRequest) -> Result<String, ProviderError> {
        Ok(self.chat_outcome(req).await?.text)
    }

    /// 执行一次 chat/OCR，返回文本与调度账号。
    ///
    /// 遭遇可重试上游错误（429 / 403）时自动换账号，最多重试 `retry_max` 次。
    /// 所有尝试均失败时返回最后一次的真实上游错误，而非通用错误。
    pub async fn chat_outcome(&self, req: &ChatRequest) -> Result<ChatOutcome, ProviderError> {
        let ocr_gate = if req.ocr {
            Some(
                self.ocr_global_gate
                    .acquire()
                    .await
                    .map_err(|e| ProviderError::Upstream(format!("ocr global gate: {e}")))?,
            )
        } else {
            None
        };

        let mut tried: Vec<i64> = Vec::new();
        let mut last_err: Option<ProviderError> = None;
        let mut last_weak: Option<ChatOutcome> = None;

        for attempt in 0..self.retry_max {
            let account_id = match self.select_account_with_keys_skip(&tried).await {
                Ok(id) => id,
                Err(e) => {
                    let _ = ocr_gate;
                    if let Some(o) = last_weak {
                        return Ok(o);
                    }
                    return Err(last_err.unwrap_or(e));
                }
            };
            tried.push(account_id);

            match self.execute_for_account(account_id, req).await {
                Ok(outcome) if req.ocr && is_weak_ocr_text(&outcome.text) && attempt + 1 < self.retry_max => {
                    tracing::warn!(
                        account_id,
                        attempt,
                        "OCR 弱识别（空/无文字），换账号重试"
                    );
                    last_weak = Some(outcome);
                    // 弱识别不算账号硬失败，避免单账号池被冷却耗尽。
                }
                Ok(outcome) => {
                    let _ = ocr_gate;
                    return Ok(outcome);
                }
                Err(e) if is_retryable_upstream_error(&e) && attempt + 1 < self.retry_max => {
                    tracing::warn!(account_id, attempt, "上游限速/拒绝，换账号重试: {e}");
                    last_err = Some(e);
                }
                Err(e) => {
                    let _ = ocr_gate;
                    return Err(e);
                }
            }
        }
        let _ = ocr_gate;
        if let Some(o) = last_weak {
            return Ok(o);
        }
        Err(last_err.unwrap_or(ProviderError::NoAvailableAccount))
    }

    /// 对指定账号执行养号/定向对话（跳过池随机选择）。
    pub async fn chat_for_account(
        &self,
        account_id: i64,
        req: &ChatRequest,
    ) -> Result<ChatOutcome, ProviderError> {
        if !self.bridge.has_pure_http_keys(account_id) {
            return Err(ProviderError::NoAvailableAccount);
        }
        self.execute_for_account(account_id, req).await
    }

    async fn execute_for_account(
        &self,
        account_id: i64,
        req: &ChatRequest,
    ) -> Result<ChatOutcome, ProviderError> {
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

        if let Some(sink) = &req.event_sink {
            sink(ChatStreamEvent::Account(account_id));
        }

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

        let sso_token = match &self.sso {
            Some(provider) => Some(provider.sso_token(account_id).await?),
            None => None,
        };
        let result = self
            .bridge
            .fetch_chat_sink(
                &payload,
                sso_token.as_deref(),
                Some(account_id),
                req.event_sink.as_ref(),
            )
            .await;

        match result {
            Ok(text) => {
                self.pool.dispatch_success(account_id).await;
                self.record_audit(req, account_id, true);
                // 异步写回 PG 成功状态（失败仅 warn，不阻断响应）
                if let Some(sink) = &self.health_sink {
                    let sink = Arc::clone(sink);
                    tokio::spawn(async move {
                        sink.record_success(account_id).await;
                    });
                }
                Ok(ChatOutcome {
                    text,
                    account_id: Some(account_id),
                })
            }
            Err(e) => {
                let retryable = is_retryable_upstream_error(&e);
                if retryable {
                    // 429 / 403：指数退避长冷却
                    self.pool.dispatch_rate_limited(account_id).await;
                    // 异步写回 PG 冷却（失败仅 warn，不阻断响应）
                    if let Some(sink) = &self.health_sink {
                        let sink = Arc::clone(sink);
                        let reason = e.to_string();
                        tokio::spawn(async move {
                            // PG 端写固定 60s 基础冷却；in-memory 池另有指数退避
                            let cooldown = chrono::Utc::now() + chrono::Duration::seconds(60);
                            sink.record_rate_limit_failure(account_id, cooldown, &reason)
                                .await;
                        });
                    }
                } else {
                    // 普通瞬时失败：2s 短冷却
                    self.pool.dispatch_failure(account_id).await;
                }
                self.record_audit(req, account_id, false);
                Err(e)
            }
        }
    }

    /// 从号池选一个具备 pure_http_keys 的账号，跳过 `tried` 中已尝试过的 id。
    async fn select_account_with_keys_skip(&self, tried: &[i64]) -> Result<i64, ProviderError> {
        const MAX_ATTEMPTS: usize = 64;
        // tried 中的账号已进入冷却，无需再传 skip（cooldown 会自动排除）；
        // 但显式排除确保即便冷却未生效也不重选同一账号。
        let mut skip: Vec<i64> = tried.to_vec();
        for _ in 0..MAX_ATTEMPTS {
            let Some(id) = self.pool.select_skip(None, &skip).await else {
                break;
            };
            if self.bridge.has_pure_http_keys(id) {
                return Ok(id);
            }
            // 没有 keys → 静默跳过，不进入冷却
            skip.push(id);
        }
        Err(ProviderError::NoAvailableAccount)
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

/// 判断上游错误是否可重试（换账号有意义的情况）。
///
/// 纯函数：仅依赖错误消息文本，无 IO，便于单元测试。
/// - HTTP 429 → 当前账号 IP 被限速，换账号可能绕过
/// - HTTP 403 → 当前账号被临时拒绝，换账号可能成功
/// - 其余（400 / 网络 / lease / bridge）→ 换账号无意义，立即返回
pub fn is_retryable_upstream_error(e: &ProviderError) -> bool {
    match e {
        ProviderError::Upstream(msg) => msg.contains("429") || msg.contains("403"),
        _ => false,
    }
}

/// OCR 弱识别：空串或默认「无文字内容」→ 换号重试有意义。
fn is_weak_ocr_text(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || t == "无文字内容"
}

#[async_trait::async_trait]
impl ChatBackend for ChatEngine {
    async fn chat_outcome(
        &self,
        req: &ChatRequest,
    ) -> Result<ChatOutcome, grok_domain::ProviderError> {
        ChatEngine::chat_outcome(self, req).await
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

    fn engine_with_retry(
        pool: SharedPool,
        bridge: Arc<dyn BridgeClient>,
        retry_max: usize,
    ) -> ChatEngine {
        let lease = Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
        ChatEngine::new(pool, lease, bridge, None).with_retry_max(retry_max)
    }

    const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn req() -> ChatRequest {
        ChatRequest {
            prompt: "[user]\n描述这张图".to_string(),
            images: vec![DATA_URI.to_string()],
            ocr: true,
            system_prompt: None,
            request_id: "req-1".to_string(),
            event_sink: None,
        }
    }

    // ── 重试谓词单测 ────────────────────────────────────────────────────────

    #[test]
    fn is_retryable_predicate_correct() {
        // 可重试：429 限速 / 403 拒绝
        assert!(is_retryable_upstream_error(&ProviderError::Upstream(
            "chat status 429 Too Many Requests".into()
        )));
        assert!(is_retryable_upstream_error(&ProviderError::Upstream(
            "chat status 403 Forbidden".into()
        )));
        // 不可重试：其他上游错误
        assert!(!is_retryable_upstream_error(&ProviderError::Upstream(
            "chat status 400 Bad Request".into()
        )));
        assert!(!is_retryable_upstream_error(&ProviderError::Upstream(
            "empty chat response".into()
        )));
        assert!(!is_retryable_upstream_error(&ProviderError::Upstream(
            "chat status 500 Internal Server Error".into()
        )));
        // 不可重试：非 Upstream 变体
        assert!(!is_retryable_upstream_error(
            &ProviderError::NoAvailableAccount
        ));
        assert!(!is_retryable_upstream_error(&ProviderError::Lease(
            "timeout".into()
        )));
        assert!(!is_retryable_upstream_error(&ProviderError::Bridge(
            "network error".into()
        )));
    }

    // ── 基础功能测试 ─────────────────────────────────────────────────────────

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
        // OCR golden：bridge 收到的 payload 禁生图 + fast mode。
        let got = concrete.last_chat_payload.lock().await;
        let payload = got.as_ref().unwrap();
        assert_eq!(payload["modeId"], "fast");
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
        let pool = grok_pool::SimplifiedPool::new();
        pool.load_in_memory(vec![sample_account(1)]).await;
        pool.pin(1).await;
        let lease = Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 1)]));
        let bridge = Arc::new(MockBridgeClient::new());

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
        let bridge = Arc::new(MockBridgeClient::new());
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
        // 非 OCR 路径使用 fast modeId 且开启生图（原字段 "model" 已迁移至 modeId）
        assert_eq!(payload["modeId"], "fast");
        assert_eq!(payload["enableImageGeneration"], true);
    }

    // ── 跨账号重试测试 ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn retry_on_429_switches_to_second_account() {
        // 账号 1 返回 429；账号 2 成功 → engine 应在账号 2 上成功并返回文本。
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(1), sample_account(2)])
            .await;
        let mut mock = MockBridgeClient::new();
        mock.chat_text = "成功".to_string();
        mock.fail_for_accounts = vec![1]; // 账号 1 模拟 429
        let bridge: Arc<dyn BridgeClient> = Arc::new(mock);
        let e = engine_with_retry(pool.clone(), bridge, 2);
        let outcome = e.chat_outcome(&req()).await.unwrap();
        assert_eq!(outcome.text, "成功");
        assert_eq!(outcome.account_id, Some(2), "应切换到账号 2");
        // 账号 1 应进入限速冷却（远长于 2s 普通冷却）
        assert!(pool.in_cooldown(1).await, "账号 1 应在限速冷却中");
    }

    #[tokio::test]
    async fn all_accounts_fail_returns_last_error() {
        // 所有账号都返回 429 → 返回最后一次的真实错误而非通用错误。
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(1), sample_account(2)])
            .await;
        let mut mock = MockBridgeClient::new();
        mock.fail_for_accounts = vec![1, 2]; // 两个账号都 429
        let bridge: Arc<dyn BridgeClient> = Arc::new(mock);
        let e = engine_with_retry(pool.clone(), bridge, 2);
        let err = e.chat_outcome(&req()).await.unwrap_err();
        // 应返回上游 429 错误，不是 NoAvailableAccount
        assert!(
            matches!(&err, ProviderError::Upstream(msg) if msg.contains("429")),
            "应返回上游 429 错误，实际: {err:?}"
        );
    }

    #[tokio::test]
    async fn retry_max_one_means_no_retry() {
        // retry_max=1 意味着只有首次尝试，没有重试。
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(1), sample_account(2)])
            .await;
        let mut mock = MockBridgeClient::new();
        mock.fail_for_accounts = vec![1]; // 账号 1 失败
        mock.chat_text = "second".to_string();
        let bridge: Arc<dyn BridgeClient> = Arc::new(mock);
        // retry_max=1：只尝试一次，账号 1 失败后直接返回错误（不重试）
        let e = engine_with_retry(pool.clone(), bridge, 1);
        let err = e.chat_outcome(&req()).await.unwrap_err();
        assert!(
            matches!(&err, ProviderError::Upstream(_)),
            "retry_max=1 时应直接返回错误"
        );
    }

    #[tokio::test]
    async fn event_sink_gets_account_then_stripped_token() {
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(7)]).await;
        let mut mock = MockBridgeClient::new();
        mock.chat_text = "hi<grok:render>x</grok:render>".to_string();
        let bridge: Arc<dyn BridgeClient> = Arc::new(mock);
        let e = engine(pool, bridge);
        let events = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let ev2 = Arc::clone(&events);
        let mut r = req();
        r.event_sink = Some(Arc::new(move |ev| {
            ev2.lock().unwrap().push(match ev {
                ChatStreamEvent::Account(id) => format!("account:{id}"),
                ChatStreamEvent::Token(t) => format!("token:{t}"),
            });
        }));
        let out = e.chat_outcome(&r).await.unwrap();
        assert_eq!(out.text, "hi");
        assert_eq!(out.account_id, Some(7));
        let got = events.lock().unwrap().clone();
        assert!(
            got.iter().any(|s| s == "account:7"),
            "应先推送选中账号, got={got:?}"
        );
        assert!(
            got.iter().any(|s| s == "token:hi"),
            "应推送剥离后的 token, got={got:?}"
        );
        assert!(!got.iter().any(|s| s.contains("grok:render")));
    }
}
