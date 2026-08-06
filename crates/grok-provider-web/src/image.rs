//! ImageEngine — grok Web 生图（imagine / imagine-lite）编排（G2）。
//!
//! 端口自 Go `provider/web/image.go`（docs/39d §4.1）。流程（39 主文档 §4.3 参考）：
//!   pool.select → egress.acquire(grok_web) → (可选 prompt 扩写)
//!     → grok-image-pipeline::ImagePipeline.reserve_slot + begin_trace + record_segment(PS)
//!     → bridge.fetch_imagine → 图片结果(url/b64)
//!     → trace.finish + dispatch 记账 + audit
//!
//! 生图元数据写入 `grok_pipeline_traces` / `grok_pipeline_segments`（G2-A2 阶段耗时字段）。

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use grok_audit::AuditSink;
use grok_domain::egress::Scope;
use grok_egress::{GateId, LeaseManager};
use grok_image_pipeline::{ImagePipeline, Status, TraceRecorder};
use grok_pool::SharedPool;
use serde_json::Value;

use crate::bridge::BridgeClient;
use crate::expand::expand_prompt;
use grok_domain::ProviderError;
use grok_domain::{ImageBackend, ImagineRequest, ImagineResult};

/// egress lease 超时（生图可能较长）。
const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(30);
const DEFAULT_SLOT_TIMEOUT: Duration = Duration::from_secs(10);

/// grok Web 生图引擎（依赖池 / lease / bridge / pipeline 均可注入，便于单测）。
pub struct ImageEngine {
    pool: SharedPool,
    lease: Arc<dyn LeaseManager>,
    bridge: Arc<dyn BridgeClient>,
    audit: Option<Arc<AuditSink>>,
    pipeline: ImagePipeline,
    lease_duration: Duration,
    slot_timeout: Duration,
    /// 生图模型名（上游）。
    model: String,
    /// lite 模型名。
    model_lite: String,
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
        Self {
            pool,
            lease,
            bridge,
            audit,
            pipeline,
            lease_duration: DEFAULT_LEASE_DURATION,
            slot_timeout: DEFAULT_SLOT_TIMEOUT,
            model: "grok-imagine-image".to_string(),
            model_lite: "grok-imagine-lite".to_string(),
        }
    }

    /// 覆盖 lease 超时（测试/调优）。
    pub fn with_lease_duration(mut self, d: Duration) -> Self {
        self.lease_duration = d;
        self
    }

    /// 执行一次生图，返回图片清单。
    pub async fn imagine(&self, req: &ImagineRequest) -> Result<ImagineResult, ProviderError> {
        // 1) 账号 + lease（grok_web scope）。
        let account_id = self
            .pool
            .select(None)
            .await
            .ok_or(ProviderError::NoAvailableAccount)?;
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

        // 2) 生图并发槽（imagine pipeline 自身槽位）。
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

        // 3) trace（PS 阶段含扩写耗时）。
        let model = if req.lite {
            &self.model_lite
        } else {
            &self.model
        };
        let mut rec = self.begin_trace(req).await?;

        // 4) prompt 扩写（enhance），计入 PS 段。
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
                    // 扩写失败不致命：用原 prompt 继续（Go 有回退语义）。
                    tracing::warn!(err = %e, "prompt expand failed, fall back to raw");
                }
            }
            rec.add_segment(
                grok_image_pipeline::Stage::Ps,
                (Utc::now() - start).num_milliseconds(),
                -1,
            );
        } else {
            // 无扩写也记一个短 PS 段（阶段耗时字段存在）。
            rec.add_segment(grok_image_pipeline::Stage::Ps, 0, -1);
        }

        // 5) 组上游 payload + bridge 生图。
        let payload = serde_json::json!({
            "model": model,
            "prompt": final_prompt,
            "n": req.n.max(1),
            "response_format": if req.response_format == "b64_json" { "b64_json" } else { "url" },
        });
        let upstream = match self.bridge.fetch_imagine(&payload).await {
            Ok(v) => v,
            Err(e) => {
                self.pool.dispatch_failure(account_id).await;
                self.record_audit(req, account_id, model, false);
                rec.finish(Status::Failed).await.ok();
                return Err(e);
            }
        };

        // 6) 解析图片。
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

    /// 开始一条 trace（记录主记录为 Running）。
    async fn begin_trace(&self, req: &ImagineRequest) -> Result<TraceRecorder, ProviderError> {
        let trace = grok_image_pipeline::PipelineTrace {
            id: String::new(), // 自动生成 g2-<uuid>
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
    // 兼容嵌套 `images` 字段。
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
        // bridge 收到 imagine payload：模型 + prompt
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
}
