//! Web dispatch 探针：对号池账号做 rate-limits 轻量探测。

use std::sync::Arc;

use async_trait::async_trait;
use grok_domain::{Account, SsoTokenProvider, WebLane};
use grok_ops::probe::ProbeBackend;
use grok_ops::OpsResult;
use grok_pool::SharedPool;
use grok_provider_web::bridge::BridgeClient;
use grok_provider_web::HttpDirectClient;

pub struct WebRateLimitsProbeBackend {
    direct: Arc<HttpDirectClient>,
    sso: Arc<dyn SsoTokenProvider>,
}

impl WebRateLimitsProbeBackend {
    pub fn new(direct: Arc<HttpDirectClient>, sso: Arc<dyn SsoTokenProvider>) -> Self {
        Self { direct, sso }
    }
}

#[async_trait]
impl ProbeBackend for WebRateLimitsProbeBackend {
    async fn dispatch_probe(&self, account: &Account, _lane: WebLane) -> OpsResult<bool> {
        if !self.direct.has_pure_http_keys(account.id) {
            return Ok(false);
        }
        let token = self
            .sso
            .sso_token(account.id)
            .await
            .map_err(|e| grok_ops::OpsError::Probe(e.to_string()))?;
        Ok(self
            .direct
            .fetch_rate_limits(Some(&token), Some(account.id))
            .await
            .is_ok())
    }
}

/// 构造 Web dispatch 探针（共享号池）。
pub fn build_web_dispatch_probe(
    pool: SharedPool,
    direct: Arc<HttpDirectClient>,
    sso: Arc<dyn SsoTokenProvider>,
) -> grok_ops::probe::WebDispatchProbe {
    let backend = Arc::new(WebRateLimitsProbeBackend::new(direct, sso));
    grok_ops::probe::WebDispatchProbe::new(pool, backend)
}
