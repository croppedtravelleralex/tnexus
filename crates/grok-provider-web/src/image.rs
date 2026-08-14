//! ImageEngine — grok Web 生图（imagine / imagine-lite）编排（G2）。
//!
//! 端口自 Go `provider/web/image.go`（docs/39d §4.1）。流程（39 主文档 §4.3 参考）：
//!   pool.select → egress.acquire(grok_web) → (可选 prompt 扩写)
//!     → grok-image-pipeline::ImagePipeline.reserve_slot + begin_trace + record_segment(PS)
//!     → bridge.fetch_imagine → 图片结果(url/b64)
//!     → trace.finish + dispatch 记账 + audit
//!
//! 跨账号重试 + 全局并发槽（对齐 GPT `IMAGE_GLOBAL_CONCURRENCY`）：
//! env `GROK_IMAGE_GLOBAL_CONCURRENCY`（默认 2）、`GROK_IMAGE_RETRY_MAX`（默认 3）。

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use grok_audit::AuditSink;
use grok_domain::egress::Scope;
use grok_egress::{GateId, LeaseManager};
use grok_image_pipeline::{ImagePipeline, Status, TraceRecorder};
use grok_pool::SharedPool;
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::bridge::BridgeClient;
use crate::engine::is_retryable_upstream_error;
use crate::expand::expand_prompt;
use grok_domain::ProviderError;
use grok_domain::{ImageBackend, ImagineRequest, ImagineResult};

const DEFAULT_LEASE_SECS: u64 = 120;
const DEFAULT_SLOT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_GLOBAL_CONCURRENCY: usize = 2;
const DEFAULT_RETRY_MAX: usize = 3;
const HARD_RETRY_CAP: usize = 8;

/// grok Web 生图引擎（依赖池 / lease / bridge / pipeline 均可注入，便于单测）。
pub struct ImageEngine {
    pool: SharedPool,
    lease: Arc<dyn LeaseManager>,
    bridge: Arc<dyn BridgeClient>,
    audit: Option<Arc<AuditSink>>,
    /// 无 chrome 直连路径：按账号取 sso token（bridge 模式为 None）。
    sso: Option<Arc<dyn grok_domain::SsoTokenProvider>>,
    pipeline: ImagePipeline,
    lease_duration: Duration,
    slot_timeout: Duration,
    /// 生图模型名（上游）。
    model: String,
    /// lite 模型名。
    model_lite: String,
    global_gate: Arc<Semaphore>,
    retry_max: usize,
}

impl ImageEngine {
    /// 组装生图引擎。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SharedPool,
        lease: Arc<dyn LeaseManager>,
        bridge: Arc<dyn BridgeClient>,
        audit: Option<Arc<AuditSink>>,
        pipeline: ImagePipeline,
    ) -> Self {
        let global_conc = std::env::var("GROK_IMAGE_GLOBAL_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_GLOBAL_CONCURRENCY)
            .max(1);
        let lease_secs = std::env::var("GROK_IMAGE_LEASE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_LEASE_SECS)
            .max(30);
        let retry_max = std::env::var("GROK_IMAGE_RETRY_MAX")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_RETRY_MAX)
            .clamp(1, HARD_RETRY_CAP);
        Self {
            pool,
            lease,
            bridge,
            audit,
            sso: None,
            pipeline,
            lease_duration: Duration::from_secs(lease_secs),
            slot_timeout: DEFAULT_SLOT_TIMEOUT,
            model: "grok-imagine-image".to_string(),
            model_lite: "grok-imagine-lite".to_string(),
            global_gate: Arc::new(Semaphore::new(global_conc)),
            retry_max,
        }
    }

    /// 覆盖 lease 超时（测试/调优）。
    pub fn with_lease_duration(mut self, d: Duration) -> Self {
        self.lease_duration = d;
        self
    }

    /// 覆盖跨账号重试上限（测试用）。
    pub fn with_retry_max(mut self, n: usize) -> Self {
        self.retry_max = n.clamp(1, HARD_RETRY_CAP);
        self
    }

    /// 注入 sso token 提供者（无 chrome 直连路径）。bridge 模式不注入。
    pub fn with_sso(mut self, sso: Arc<dyn grok_domain::SsoTokenProvider>) -> Self {
        self.sso = Some(sso);
        self
    }

    /// 执行一次生图，返回图片清单（含跨号重试 + 全局并发槽）。
    pub async fn imagine(&self, req: &ImagineRequest) -> Result<ImagineResult, ProviderError> {
        let _global = self
            .global_gate
            .acquire()
            .await
            .map_err(|e| ProviderError::Upstream(format!("image global gate: {e}")))?;

        let mut tried: Vec<i64> = Vec::new();
        let mut last_err: Option<ProviderError> = None;

        for attempt in 0..self.retry_max {
            let account_id = match self.select_account_with_keys_skip(&tried).await {
                Ok(id) => id,
                Err(e) => return Err(last_err.unwrap_or(e)),
            };
            tried.push(account_id);

            match self.imagine_once(account_id, req).await {
                Ok(result) => return Ok(result),
                Err(e) if is_retryable_imagine_error(&e) && attempt + 1 < self.retry_max => {
                    tracing::warn!(account_id, attempt, "生图可重试错误，换账号: {e}");
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or(ProviderError::NoAvailableAccount))
    }

    async fn imagine_once(
        &self,
        account_id: i64,
        req: &ImagineRequest,
    ) -> Result<ImagineResult, ProviderError> {
        let _lease = match self
            .lease
            .acquire(
                Scope::GrokWeb,
                GateId::from(account_id.to_string()),
                self.lease_duration,
            )
            .await
        {
            Ok(l) => l,
            Err(e) => {
                self.pool.dispatch_failure(account_id).await;
                return Err(ProviderError::Lease(e.to_string()));
            }
        };

        let _slot = match self
            .pipeline
            .reserve_slot_timeout("ps", self.slot_timeout)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                self.pool.dispatch_failure(account_id).await;
                return Err(ProviderError::Upstream(format!("slot: {e}")));
            }
        };

        let model = if req.lite {
            &self.model_lite
        } else {
            &self.model
        };
        let mut rec = self.begin_trace(req).await?;

        let mut final_prompt = req.prompt.clone();
        if req.enhance {
            let start = Utc::now();
            match expand_prompt(
                self.bridge.as_ref(),
                &serde_json::json!(req.prompt),
                Scope::GrokWeb,
            )
            .await
            {
                Ok(p) => final_prompt = p,
                Err(e) => {
                    tracing::warn!(err = %e, "prompt expand failed, fall back to raw");
                }
            }
            rec.add_segment(
                grok_image_pipeline::Stage::Ps,
                (Utc::now() - start).num_milliseconds(),
                -1,
            );
        } else {
            rec.add_segment(grok_image_pipeline::Stage::Ps, 0, -1);
        }

        let aspect_ratio = if req.aspect_ratio.trim().is_empty() {
            "1:1"
        } else {
            req.aspect_ratio.as_str()
        };
        let payload = serde_json::json!({
            "model": model,
            "prompt": final_prompt,
            "n": req.n.max(1),
            "response_format": if req.response_format == "b64_json" { "b64_json" } else { "url" },
            "aspect_ratio": aspect_ratio,
        });
        let sso_token = match &self.sso {
            Some(provider) => Some(provider.sso_token(account_id).await?),
            None => None,
        };
        let upstream = match self
            .bridge
            .fetch_imagine(&payload, sso_token.as_deref(), Some(account_id))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                self.mark_imagine_failure(account_id, &e).await;
                self.record_audit(req, account_id, model, false);
                rec.finish(Status::Failed).await.ok();
                return Err(e);
            }
        };

        let items = extract_images(&upstream);
        if items.is_empty() {
            self.pool.dispatch_failure(account_id).await;
            self.record_audit(req, account_id, model, false);
            rec.finish(Status::Failed).await.ok();
            return Err(ProviderError::Upstream(
                "no image data in imagine response".into(),
            ));
        }

        self.pool.dispatch_success(account_id).await;
        rec.add_segment(grok_image_pipeline::Stage::Sse, 0, -1);
        rec.finish(Status::Succeeded).await.ok();
        self.record_audit(req, account_id, model, true);
        Ok(ImagineResult {
            b64: req.response_format == "b64_json",
            items,
        })
    }

    async fn mark_imagine_failure(&self, account_id: i64, e: &ProviderError) {
        if is_retryable_upstream_error(e) {
            self.pool.dispatch_rate_limited(account_id).await;
        } else {
            self.pool.dispatch_failure(account_id).await;
        }
    }

    async fn select_account_with_keys_skip(&self, tried: &[i64]) -> Result<i64, ProviderError> {
        const MAX_ATTEMPTS: usize = 64;
        let mut skip: Vec<i64> = tried.to_vec();
        for _ in 0..MAX_ATTEMPTS {
            let Some(id) = self.pool.select_skip(None, &skip).await else {
                break;
            };
            if self.bridge.has_pure_http_keys(id) {
                return Ok(id);
            }
            skip.push(id);
        }
        Err(ProviderError::NoAvailableAccount)
    }

    /// 开始一条 trace（记录主记录为 Running）。
    async fn begin_trace(&self, req: &ImagineRequest) -> Result<TraceRecorder, ProviderError> {
        let trace = grok_image_pipeline::PipelineTrace {
            id: String::new(),
            request_id: req.request_id.clone(),
            lane: 0,
            status: Status::Queued,
            model: if req.lite {
                self.model_lite.clone()
            } else {
                self.model.clone()
            },
            account_id: None,
            account_name: String::new(),
            error_code: String::new(),
            started_at: Utc::now(),
            ended_at: None,
            queue_ms: 0,
            upload_queue_ms: 0,
            ps_queue_ms: 0,
            ss_queue_ms: 0,
            download_queue_ms: 0,
            expand_ms: 0,
            ssems: 0,
            download_ms: 0,
            total_ms: 0,
            soft_stop: false,
        };
        self.pipeline
            .begin_trace(trace)
            .await
            .map_err(|e| ProviderError::Upstream(format!("trace: {e}")))
    }

    /// 记录审计（异步、非阻塞）。
    fn record_audit(&self, req: &ImagineRequest, account_id: i64, model: &str, ok: bool) {
        if let Some(sink) = &self.audit {
            let audit = grok_audit::CreateAudit {
                event_id: grok_audit::CreateAudit::new_event_id(),
                request_id: req.request_id.clone(),
                account_id: Some(account_id),
                provider: "grok_web".into(),
                operation: grok_audit::Operation::Image,
                status_code: if ok { 200 } else { 502 },
                model_public_id: Some(model.to_string()),
                media_input_images: 0,
                streaming: false,
                ..Default::default()
            };
            let _ = sink.record(audit);
        }
    }
}

/// 生图失败是否值得换号重试（WSS RST / 限速 / 空图 / lease 争用）。
fn is_retryable_imagine_error(e: &ProviderError) -> bool {
    match e {
        ProviderError::Lease(_) => true,
        ProviderError::Upstream(msg) => {
            is_retryable_upstream_error(e)
                || msg.contains("Connection reset")
                || msg.contains("WebSocket")
                || msg.contains("imagine ws")
                || msg.contains("rate limit")
                || msg.contains("no image data")
        }
        ProviderError::Bridge(msg) => {
            msg.contains("imagine ws")
                || msg.contains("Connection reset")
                || msg.contains("WebSocket")
                || msg.contains("timeout")
        }
        _ => false,
    }
}

fn extract_images(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(items) = v.get("data").and_then(Value::as_array) {
        for it in items {
            if let Some(u) = it.get("url").and_then(Value::as_str) {
                out.push(u.to_string());
            } else if let Some(b) = it.get("b64_json").and_then(Value::as_str) {
                out.push(b.to_string());
            }
        }
    }
    if out.is_empty() {
        if let Some(imgs) = v.get("images").and_then(Value::as_array) {
            for it in imgs {
                if let Some(u) = it.as_str() {
                    out.push(u.to_string());
                }
            }
        }
    }
    out
}

#[async_trait::async_trait]
impl ImageBackend for ImageEngine {
    async fn imagine(
        &self,
        req: &ImagineRequest,
    ) -> Result<ImagineResult, grok_domain::ProviderError> {
        ImageEngine::imagine(self, req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_domain::{Account, AuthStatus, Provider};
    use grok_egress::InMemoryLeaseManager;
    use grok_image_pipeline::{ImagePipeline, InMemoryTraceRepository, SlotManager};
    use std::sync::Arc;

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

    fn test_pipeline() -> ImagePipeline {
        ImagePipeline::new(
            SlotManager::new(&[("ps", 2), ("ss", 1)]),
            Arc::new(InMemoryTraceRepository::new()),
        )
    }

    fn req() -> ImagineRequest {
        ImagineRequest {
            prompt: "a red fox".to_string(),
            n: 1,
            response_format: "url".to_string(),
            lite: false,
            enhance: false,
            request_id: "req-img-1".to_string(),
            aspect_ratio: "1:1".to_string(),
        }
    }

    #[tokio::test]
    async fn imagine_returns_url_and_records_trace() {
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(1)]).await;
        let mut b = crate::bridge::MockBridgeClient::new();
        b.imagine_response = serde_json::json!({ "data": [ {"url": "https://x/img.png"} ] });
        let concrete: Arc<crate::bridge::MockBridgeClient> = Arc::new(b);
        let bridge: Arc<dyn BridgeClient> = concrete.clone();
        let e = ImageEngine::new(
            pool,
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)])),
            bridge,
            None,
            test_pipeline(),
        );
        let res = e.imagine(&req()).await.expect("imagine");
        assert_eq!(res.items, vec!["https://x/img.png".to_string()]);
        assert!(!res.b64);
        let got = concrete.last_imagine_payload.lock().await;
        let p = got.as_ref().unwrap();
        assert_eq!(p["model"], "grok-imagine-image");
        assert_eq!(p["prompt"], "a red fox");
    }

    #[tokio::test]
    async fn imagine_lite_uses_lite_model() {
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(2)]).await;
        let mut mock = crate::bridge::MockBridgeClient::new();
        mock.imagine_response = serde_json::json!({ "data": [ {"url": "https://x/lite.png"} ] });
        let concrete: Arc<crate::bridge::MockBridgeClient> = Arc::new(mock);
        let bridge: Arc<dyn BridgeClient> = concrete.clone();
        let e = ImageEngine::new(
            pool,
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)])),
            bridge,
            None,
            test_pipeline(),
        );
        let mut r = req();
        r.lite = true;
        e.imagine(&r).await.expect("imagine lite");
        let got = concrete.last_imagine_payload.lock().await;
        assert_eq!(got.as_ref().unwrap()["model"], "grok-imagine-lite");
    }

    #[tokio::test]
    async fn imagine_b64_preserves_flag() {
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        pool.load_in_memory(vec![sample_account(3)]).await;
        let mut b = crate::bridge::MockBridgeClient::new();
        b.imagine_response = serde_json::json!({ "data": [ {"b64_json": "AAAA"} ] });
        let bridge: Arc<dyn BridgeClient> = Arc::new(b);
        let e = ImageEngine::new(
            pool,
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)])),
            bridge,
            None,
            test_pipeline(),
        );
        let mut r = req();
        r.response_format = "b64_json".to_string();
        let res = e.imagine(&r).await.expect("imagine b64");
        assert!(res.b64);
        assert_eq!(res.items, vec!["AAAA".to_string()]);
    }

    #[tokio::test]
    async fn no_available_account_errors() {
        let pool: SharedPool = Arc::new(grok_pool::SimplifiedPool::new());
        let bridge: Arc<dyn BridgeClient> = Arc::new(crate::bridge::MockBridgeClient::new());
        let e = ImageEngine::new(
            pool,
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)])),
            bridge,
            None,
            test_pipeline(),
        );
        let r = e.imagine(&req()).await;
        assert!(matches!(r, Err(ProviderError::NoAvailableAccount)));
    }

    #[test]
    fn extract_images_variants() {
        assert_eq!(
            extract_images(&serde_json::json!({"data":[{"url":"a"}]})),
            vec!["a".to_string()]
        );
        assert_eq!(
            extract_images(&serde_json::json!({"data":[{"b64_json":"b"}]})),
            vec!["b".to_string()]
        );
        assert_eq!(
            extract_images(&serde_json::json!({"images":["c","d"]})),
            vec!["c".to_string(), "d".to_string()]
        );
        assert!(extract_images(&serde_json::json!({"x":1})).is_empty());
    }

    #[test]
    fn retryable_imagine_errors() {
        assert!(is_retryable_imagine_error(
            &ProviderError::Upstream("imagine ws: Connection reset".into())
        ));
        assert!(is_retryable_imagine_error(
            &ProviderError::Upstream("no image data in imagine response".into())
        ));
        assert!(!is_retryable_imagine_error(
            &ProviderError::Upstream("chat status 400".into())
        ));
    }
}
