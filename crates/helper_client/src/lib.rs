//! Client for the local curl_cffi / protocol bridge helper.

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Shared-secret header the helper requires on every `/v1/internal/*` route.
const HELPER_TOKEN_HEADER: &str = "X-Helper-Token";

#[derive(Clone)]
pub struct HelperClient {
    base: String,
    http: Client,
    /// Long-running image client: the shared one would cut the request short.
    image_http: Client,
    token: Option<String>,
}

impl std::fmt::Debug for HelperClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperClient")
            .field("base", &self.base)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinAccount {
    pub email: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

/// A pool entry as reported by `/v1/internal/accounts/candidates`.
///
/// Deliberately credential-free — the helper resolves the real token and proxy
/// from its own pool row by email. Reusing [`PinAccount`] here would let
/// `#[serde(default)]` silently produce an empty `access_token` that reads as a
/// valid account and then fails opaquely upstream.
#[derive(Debug, Clone, Deserialize)]
pub struct CandidateAccount {
    pub email: String,
    #[serde(default)]
    pub has_token: bool,
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

impl CandidateAccount {
    /// Reference for the helper to resolve; carries no credentials by design.
    pub fn to_pin(&self) -> PinAccount {
        PinAccount {
            email: self.email.clone(),
            access_token: String::new(),
            device_id: None,
            proxy: None,
            user_agent: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TextRunRequest {
    pub account: PinAccount,
    pub prompt: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Serialize)]
pub struct ImageRunRequest {
    pub account: PinAccount,
    pub prompt: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub size: String,
}

#[derive(Debug, Serialize)]
pub struct QuotaRefreshRequest {
    pub account: PinAccount,
    #[serde(default = "default_min_remaining")]
    pub min_remaining: i64,
}

#[allow(dead_code)] // referenced by serde default=
fn default_min_remaining() -> i64 {
    1
}

#[derive(Debug, Deserialize)]
pub struct BridgeOk {
    pub ok: bool,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub b64_json: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub fault: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub raw: Option<Value>,
    #[serde(default)]
    pub quota: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct QuotaOk {
    pub ok: bool,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub remaining: Option<i64>,
    #[serde(default)]
    pub restore_at: Option<String>,
    #[serde(default)]
    pub image_quota_unknown: Option<bool>,
    #[serde(default)]
    pub min_remaining: Option<i64>,
    #[serde(default)]
    pub imageable: Option<bool>,
    #[serde(default)]
    pub image_gen: Option<Value>,
    #[serde(default)]
    pub fault: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
}

impl HelperClient {
    pub fn new(base: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .context("build helper http client")?;
        // Image round trips run ~40-80s and have been seen at 111s; built once
        // here rather than per call so the connection pool is actually reused.
        let image_http = Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .context("build helper image client")?;
        Ok(Self {
            base: base.into().trim_end_matches('/').to_string(),
            http,
            image_http,
            token: std::env::var("HELPER_INTERNAL_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }

    /// Whether the shared secret is configured. The helper fails closed without
    /// it, so a gateway missing the token can reach nothing but `/health`.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => rb.header(HELPER_TOKEN_HEADER, t),
            None => rb,
        }
    }

    pub async fn health(&self) -> Result<Value> {
        let url = format!("{}/health", self.base);
        let resp = self.http.get(url).send().await.context("helper health")?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            return Err(anyhow!("helper health status={status} body={body}"));
        }
        Ok(body)
    }

    pub async fn refresh_quota(&self, req: &QuotaRefreshRequest) -> Result<QuotaOk> {
        let url = format!("{}/v1/internal/quota/refresh", self.base);
        let resp = self
            .auth(self.http.post(url))
            .json(req)
            .send()
            .await
            .context("helper POST /v1/internal/quota/refresh")?;
        let status = resp.status();
        let parsed: QuotaOk = resp.json().await.context("helper decode quota refresh")?;
        if !status.is_success() && parsed.error.is_none() {
            return Err(anyhow!(
                "helper quota status={status} err={:?}",
                parsed.error
            ));
        }
        Ok(parsed)
    }

    pub async fn list_candidates(&self, limit: usize) -> Result<Vec<CandidateAccount>> {
        let url = format!(
            "{}/v1/internal/accounts/candidates?limit={limit}",
            self.base
        );
        let resp = self
            .auth(self.http.get(url))
            .send()
            .await
            .context("helper GET /v1/internal/accounts/candidates")?;
        let status = resp.status();
        #[derive(Deserialize)]
        struct CandBody {
            #[serde(default)]
            ok: bool,
            #[serde(default)]
            accounts: Vec<CandidateAccount>,
            #[serde(default)]
            error: Option<String>,
        }
        let parsed: CandBody = resp.json().await.context("helper decode candidates")?;
        if !status.is_success() || !parsed.ok {
            return Err(anyhow!(
                "helper candidates status={status} err={:?}",
                parsed.error
            ));
        }
        Ok(parsed.accounts)
    }

    pub async fn run_text(&self, req: &TextRunRequest) -> Result<BridgeOk> {
        self.post_json("/v1/internal/text", req).await
    }

    /// Stream SSE from helper `/v1/internal/text/stream` (caller proxies body).
    pub async fn run_text_stream(&self, req: &TextRunRequest) -> Result<reqwest::Response> {
        let url = format!("{}/v1/internal/text/stream", self.base);
        // `image_http`'s longer deadline applies to the whole body, so a stream
        // on the 120s client would be severed mid-flight on slow generations.
        let resp = self
            .auth(self.image_http.post(url))
            .json(req)
            .send()
            .await
            .context("helper POST /v1/internal/text/stream")?;
        Ok(resp)
    }

    pub async fn run_image(&self, req: &ImageRunRequest) -> Result<BridgeOk> {
        let url = format!("{}/v1/internal/image", self.base);
        let resp = self
            .auth(self.image_http.post(url))
            .json(req)
            .send()
            .await
            .context("helper POST /v1/internal/image")?;
        let status = resp.status();
        let parsed: BridgeOk = resp.json().await.context("helper decode image")?;
        if !status.is_success() && parsed.error.is_none() {
            return Err(anyhow!(
                "helper image status={status} err={:?}",
                parsed.error
            ));
        }
        Ok(parsed)
    }

    async fn post_json<T: Serialize>(&self, path: &str, body: &T) -> Result<BridgeOk> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .auth(self.http.post(url))
            .json(body)
            .send()
            .await
            .with_context(|| format!("helper POST {path}"))?;
        let status = resp.status();
        let parsed: BridgeOk = resp
            .json()
            .await
            .with_context(|| format!("helper decode {path}"))?;
        if !status.is_success() && parsed.error.is_none() {
            return Err(anyhow!(
                "helper {path} status={status} err={:?}",
                parsed.error
            ));
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_to_pin_carries_no_credentials() {
        let cand = CandidateAccount {
            email: "a@example.com".into(),
            has_token: true,
            proxy_host: Some("proxy.example.com".into()),
            status: None,
        };
        let pin = cand.to_pin();
        assert_eq!(pin.email, "a@example.com");
        assert!(pin.access_token.is_empty());
        assert!(pin.proxy.is_none(), "proxy must be resolved helper-side");
    }

    #[test]
    fn debug_redacts_token() {
        let mut c = HelperClient::new("http://127.0.0.1:1").unwrap();
        c.token = Some("super-secret".into());
        let rendered = format!("{c:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn base_url_trailing_slash_is_normalised() {
        let c = HelperClient::new("http://127.0.0.1:19001/").unwrap();
        assert_eq!(c.base, "http://127.0.0.1:19001");
    }
}
