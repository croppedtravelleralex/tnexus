//! Upstream client for the free Build promo path (`cli-chat-proxy`).
//!
//! Two rules encoded here, both learned the hard way:
//!   * never pin an exact model id — upstream renames it without notice;
//!   * a rotated refresh token must reach the store, or the account dies.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::model::Health;

pub const DEFAULT_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";
pub const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const CLIENT_VERSION: &str = "0.2.93";
/// Only used when `/models` says nothing usable.
pub const FALLBACK_MODEL: &str = "grok-4.6";

/// How an upstream failure should affect the account's health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Refresh token rejected — terminal until re-imported.
    Revoked,
    /// Entitlement denial; retrying this account is pointless for a while.
    Forbidden,
    /// Rate limited or out of quota — back off and retry later.
    Cooling(i64),
    /// Transport hiccup; the account itself is probably fine.
    Transient,
}

impl Failure {
    pub fn health(&self) -> Health {
        match self {
            Failure::Revoked => Health::NeedsReauth,
            Failure::Forbidden => Health::Forbidden,
            Failure::Cooling(_) | Failure::Transient => Health::Cooling,
        }
    }

    pub fn cooling_secs(&self) -> i64 {
        match self {
            Failure::Cooling(secs) => *secs,
            Failure::Transient => 30,
            _ => 0,
        }
    }
}

/// Map an upstream status + body onto a health decision.
///
/// `402`/`429` are transient-by-quota, `403` is an entitlement denial, and a
/// missing status (status 0) means the request never got an answer, which says
/// nothing about the account.
pub fn classify(status: u16, body: &str) -> Failure {
    let lower = body.to_ascii_lowercase();
    if lower.contains("invalid_grant") || lower.contains("refresh token has been revoked") {
        return Failure::Revoked;
    }
    match status {
        0 => Failure::Transient,
        401 => Failure::Revoked,
        402 => Failure::Cooling(1_800),
        403 => {
            if lower.contains("spending-limit") || lower.contains("insufficient") {
                Failure::Cooling(3_600)
            } else {
                Failure::Forbidden
            }
        }
        408 | 409 | 425 | 500 | 502 | 503 | 504 => Failure::Transient,
        429 => Failure::Cooling(600),
        _ => Failure::Transient,
    }
}

/// Newest plain `grok-<major>.<minor>` advertised by `/models`.
///
/// Suffixed variants are special-purpose aliases, not the promo chat model.
pub fn pick_chat_model(ids: &[String]) -> Option<String> {
    let mut best: Option<((u32, u32), String)> = None;
    for id in ids {
        let Some(rest) = id.strip_prefix("grok-") else {
            continue;
        };
        let Some((major, minor)) = rest.split_once('.') else {
            continue;
        };
        let (Ok(major), Ok(minor)) = (major.parse::<u32>(), minor.parse::<u32>()) else {
            continue;
        };
        let version = (major, minor);
        if best.as_ref().map(|(v, _)| version > *v).unwrap_or(true) {
            best = Some((version, id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
}

#[derive(Debug)]
pub struct UpstreamError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upstream {} {}", self.status, truncate(&self.body, 200))
    }
}

impl std::error::Error for UpstreamError {}

impl UpstreamError {
    pub fn failure(&self) -> Failure {
        classify(self.status, &self.body)
    }
}

pub fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

/// A token endpoint that has not answered in this long is not going to.
///
/// Chat needs a generous timeout, but reusing it for refresh makes every dead
/// account cost the full chat budget before the scheduler can move on — a
/// mostly-stale pool then fails requests purely on timeouts.
const REFRESH_TIMEOUT_SECS: u64 = 15;

#[derive(Clone)]
pub struct Upstream {
    base_url: String,
    timeout: Duration,
    refresh_timeout: Duration,
    default_proxy: String,
}

impl Upstream {
    pub fn new(base_url: impl Into<String>, timeout_secs: u64) -> Self {
        let timeout = Duration::from_secs(timeout_secs.max(5));
        Upstream {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            refresh_timeout: timeout.min(Duration::from_secs(REFRESH_TIMEOUT_SECS)),
            timeout,
            default_proxy: String::new(),
        }
    }

    #[cfg(test)]
    pub fn refresh_timeout(&self) -> Duration {
        self.refresh_timeout
    }

    /// Egress for accounts that carry no sticky `proxy_url` of their own.
    pub fn with_default_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.default_proxy = proxy.into().trim().to_string();
        self
    }

    /// Per-account sticky egress wins; the configured default is the fallback.
    pub fn effective_proxy<'a>(&'a self, account_proxy: &'a str) -> &'a str {
        let trimmed = account_proxy.trim();
        if trimmed.is_empty() {
            &self.default_proxy
        } else {
            trimmed
        }
    }

    /// One client per call: each account may carry a different sticky egress,
    /// and reqwest bakes the proxy into the client.
    fn client(&self, proxy_url: &str) -> Result<reqwest::Client> {
        self.client_with_timeout(proxy_url, self.timeout)
    }

    fn client_with_timeout(&self, proxy_url: &str, timeout: Duration) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(format!("grok-cli/{CLIENT_VERSION}"));
        let proxy = self.effective_proxy(proxy_url);
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        Ok(builder.build()?)
    }

    fn cli_headers(&self, extra: &serde_json::Value) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        for (key, value) in [
            ("X-XAI-Token-Auth", "xai-grok-cli"),
            ("x-grok-client-version", CLIENT_VERSION),
            ("x-grok-client-identifier", "grok-shell"),
        ] {
            headers.insert(
                HeaderName::from_static_str_checked(key),
                HeaderValue::from_static(value),
            );
        }
        if let Some(map) = extra.as_object() {
            for (key, value) in map {
                let (Ok(name), Some(text)) = (HeaderName::try_from(key.as_str()), value.as_str())
                else {
                    continue;
                };
                if let Ok(parsed) = HeaderValue::from_str(text) {
                    headers.insert(name, parsed);
                }
            }
        }
        headers
    }

    pub async fn refresh_token(&self, refresh_token: &str, proxy_url: &str) -> Result<TokenPair> {
        let client = self.client_with_timeout(proxy_url, self.refresh_timeout)?;
        let response = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;

        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        let parsed: TokenResponse = serde_json::from_str(&text).map_err(|err| UpstreamError {
            status,
            body: format!("bad token json: {err}"),
        })?;
        let expires_at = crate::jwt::access_token_expiry(&parsed.access_token)
            .unwrap_or_else(|| crate::now() + parsed.expires_in.unwrap_or(21_600));
        Ok(TokenPair {
            access_token: parsed.access_token,
            // Empty means "upstream kept the old one" — the store treats it as no-op.
            refresh_token: parsed.refresh_token.unwrap_or_default(),
            expires_at,
        })
    }

    pub async fn list_models(
        &self,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
    ) -> Result<Vec<String>> {
        let client = self.client(proxy_url)?;
        let response = client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(access_token)
            .headers(self.cli_headers(extra_headers))
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        let parsed: ModelsResponse =
            serde_json::from_str(&text).unwrap_or(ModelsResponse { data: Vec::new() });
        Ok(parsed
            .data
            .into_iter()
            .map(|entry| entry.id)
            .filter(|id| !id.is_empty())
            .collect())
    }

    /// Forward an OpenAI-shaped chat request. Returns the raw upstream JSON.
    pub async fn chat_completions(
        &self,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let client = self.client(proxy_url)?;
        let response = client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(access_token)
            .headers(self.cli_headers(extra_headers))
            .json(payload)
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        serde_json::from_str(&text).map_err(|err| anyhow!("bad upstream json: {err}"))
    }

    pub async fn responses(
        &self,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let client = self.client(proxy_url)?;
        let response = client
            .post(format!("{}/responses", self.base_url))
            .bearer_auth(access_token)
            .headers(self.cli_headers(extra_headers))
            .json(payload)
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        serde_json::from_str(&text).map_err(|err| anyhow!("bad upstream json: {err}"))
    }
}

/// `HeaderName::from_static` panics on a bad name; this keeps startup safe.
trait HeaderNameExt {
    fn from_static_str_checked(value: &'static str) -> reqwest::header::HeaderName;
}

impl HeaderNameExt for reqwest::header::HeaderName {
    fn from_static_str_checked(value: &'static str) -> reqwest::header::HeaderName {
        reqwest::header::HeaderName::try_from(value)
            .unwrap_or(reqwest::header::HeaderName::from_static("x-grok-unused"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn newest_model_wins() {
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.5", "grok-4.6"])).as_deref(),
            Some("grok-4.6")
        );
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.6", "grok-4.10"])).as_deref(),
            Some("grok-4.10")
        );
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.9", "grok-5.0"])).as_deref(),
            Some("grok-5.0")
        );
    }

    #[test]
    fn suffixed_aliases_do_not_win() {
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.20-0309-non-reasoning", "grok-4.6"])).as_deref(),
            Some("grok-4.6")
        );
    }

    #[test]
    fn no_usable_model_is_none() {
        assert_eq!(pick_chat_model(&ids(&["gpt-4o", "grok-3-mini"])), None);
        assert_eq!(pick_chat_model(&[]), None);
    }

    #[test]
    fn revoked_refresh_is_terminal() {
        let failure = classify(400, r#"{"error":"invalid_grant"}"#);
        assert_eq!(failure, Failure::Revoked);
        assert_eq!(failure.health(), Health::NeedsReauth);
    }

    #[test]
    fn permission_denied_is_not_a_cooldown() {
        let failure = classify(403, r#"{"code":"permission-denied"}"#);
        assert_eq!(failure, Failure::Forbidden);
        assert_eq!(failure.health(), Health::Forbidden);
    }

    #[test]
    fn spending_limit_cools_instead_of_banning() {
        let failure = classify(403, "personal-team-blocked:spending-limit");
        assert!(matches!(failure, Failure::Cooling(_)));
    }

    #[test]
    fn quota_and_rate_limit_cool_down() {
        assert!(matches!(classify(402, "no credit"), Failure::Cooling(_)));
        assert!(matches!(classify(429, "slow down"), Failure::Cooling(_)));
    }

    #[test]
    fn unanswered_request_blames_the_network_not_the_account() {
        let failure = classify(0, "connection reset");
        assert_eq!(failure, Failure::Transient);
        assert_eq!(failure.health(), Health::Cooling);
        assert!(failure.cooling_secs() < 60);
    }

    #[test]
    fn server_errors_are_transient() {
        for status in [500u16, 502, 503, 504] {
            assert_eq!(classify(status, "boom"), Failure::Transient);
        }
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("中文很长的内容", 2), "中文…");
    }

    #[test]
    fn account_sticky_proxy_beats_the_default() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5).with_default_proxy("http://default:1");
        assert_eq!(
            upstream.effective_proxy("http://sticky:2"),
            "http://sticky:2"
        );
    }

    #[test]
    fn default_proxy_covers_accounts_without_one() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5).with_default_proxy("http://default:1");
        assert_eq!(upstream.effective_proxy(""), "http://default:1");
        assert_eq!(upstream.effective_proxy("   "), "http://default:1");
    }

    #[test]
    fn no_proxy_configured_means_direct() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5);
        assert_eq!(upstream.effective_proxy(""), "");
    }

    #[test]
    fn refresh_gives_up_long_before_the_chat_budget() {
        // A stale pool must not spend the full chat timeout per dead account.
        let upstream = Upstream::new(DEFAULT_BASE_URL, 120);
        assert_eq!(
            upstream.refresh_timeout(),
            Duration::from_secs(REFRESH_TIMEOUT_SECS)
        );
    }

    #[test]
    fn a_short_chat_timeout_also_shortens_refresh() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 8);
        assert_eq!(upstream.refresh_timeout(), Duration::from_secs(8));
    }
}
