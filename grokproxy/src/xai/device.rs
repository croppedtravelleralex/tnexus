//! The OAuth device flow against `auth.x.ai`.
//!
//! This host answers any TLS client, so it uses the ordinary reqwest client
//! rather than the impersonating one `accounts.x.ai` requires.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use tracing::debug;

pub const AUTH_BASE_URL: &str = "https://auth.x.ai";
pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

/// Polling stops here even if the upstream advertises longer; a device code
/// that has not been approved by now never will be, and the account is holding
/// a worker the whole time.
const MAX_POLL_SECS: u64 = 180;

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    #[serde(default)]
    pub verification_uri_complete: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Clone)]
pub struct DeviceTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

pub struct DeviceFlow {
    client: reqwest::Client,
}

impl DeviceFlow {
    pub fn new(proxy_url: &str, timeout_secs: u64) -> Result<Self> {
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(timeout_secs));
        if !proxy_url.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
        }
        Ok(DeviceFlow {
            client: builder.build()?,
        })
    }

    pub async fn request_code(&self) -> Result<DeviceCode> {
        let response = self
            .client
            .post(format!("{AUTH_BASE_URL}/oauth2/device/code"))
            .form(&[("client_id", CLIENT_ID), ("scope", SCOPE)])
            .send()
            .await?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            bail!("device code request failed: HTTP {status} {}", trim(&text));
        }
        let code: DeviceCode = serde_json::from_str(&text)
            .map_err(|err| anyhow!("device code response unparseable: {err}: {}", trim(&text)))?;
        if code.device_code.is_empty() || code.user_code.is_empty() {
            bail!("device code response incomplete: {}", trim(&text));
        }
        Ok(code)
    }

    /// Poll until the device is approved.
    ///
    /// `authorization_pending` and `slow_down` are the flow working as designed
    /// and must not be charged to the account; anything else is terminal.
    pub async fn poll_token(&self, code: &DeviceCode) -> Result<DeviceTokens> {
        let mut interval = code.interval.max(1);
        let budget = Duration::from_secs(
            code.expires_in
                .clamp(interval * 2, MAX_POLL_SECS)
                .max(interval * 2),
        );
        let deadline = tokio::time::Instant::now() + budget;
        let mut network_errors = 0;

        loop {
            if tokio::time::Instant::now() >= deadline {
                bail!("device authorization timed out after {}s", budget.as_secs());
            }
            let sent = self
                .client
                .post(format!("{AUTH_BASE_URL}/oauth2/token"))
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", code.device_code.as_str()),
                    ("client_id", CLIENT_ID),
                ])
                .send()
                .await;

            let response = match sent {
                Ok(response) => response,
                Err(err) => {
                    // A dropped connection says nothing about the device code.
                    network_errors += 1;
                    if network_errors > 8 {
                        bail!("token polling network failure after 8 retries: {err}");
                    }
                    tokio::time::sleep(Duration::from_secs(interval)).await;
                    continue;
                }
            };
            network_errors = 0;
            let text = response.text().await.unwrap_or_default();
            let body: TokenResponse = serde_json::from_str(&text).unwrap_or(TokenResponse {
                access_token: String::new(),
                refresh_token: String::new(),
                id_token: String::new(),
                expires_in: 0,
                error: "unparseable".into(),
                error_description: trim(&text),
            });

            if !body.access_token.is_empty() {
                if body.refresh_token.is_empty() {
                    // Without it the account is single-use and dies in hours.
                    bail!("token response has no refresh_token");
                }
                return Ok(DeviceTokens {
                    access_token: body.access_token,
                    refresh_token: body.refresh_token,
                    id_token: body.id_token,
                    expires_in: if body.expires_in > 0 {
                        body.expires_in
                    } else {
                        21_600
                    },
                });
            }

            match body.error.as_str() {
                "authorization_pending" => {}
                "slow_down" => interval += 5,
                other => bail!(
                    "device authorization rejected: {other} {}",
                    body.error_description
                ),
            }
            debug!(error = %body.error, interval, "device authorization pending");
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    }
}

fn trim(text: &str) -> String {
    crate::upstream::truncate(text.trim(), 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_code_response_parses_with_defaults() {
        let body = r#"{"device_code":"dc","user_code":"2GST-DW8V",
                       "verification_uri_complete":"https://accounts.x.ai/oauth2/device?user_code=2GST-DW8V",
                       "expires_in":1800,"interval":5}"#;
        let code: DeviceCode = serde_json::from_str(body).unwrap();
        assert_eq!(code.user_code, "2GST-DW8V");
        assert_eq!(code.interval, 5);
    }

    #[test]
    fn a_response_without_an_interval_still_polls() {
        // Missing interval means "unspecified", not "poll as fast as possible".
        let code: DeviceCode =
            serde_json::from_str(r#"{"device_code":"dc","user_code":"AAAA-BBBB"}"#).unwrap();
        assert_eq!(code.interval, 5);
    }

    #[test]
    fn polling_is_bounded_even_when_the_upstream_offers_half_an_hour() {
        // The upstream advertises 1800s; holding a worker that long for a code
        // nobody is going to approve is worse than failing.
        let code = DeviceCode {
            device_code: "dc".into(),
            user_code: "A".into(),
            verification_uri_complete: String::new(),
            interval: 5,
            expires_in: 1800,
        };
        let budget = code
            .expires_in
            .clamp(code.interval * 2, MAX_POLL_SECS)
            .max(code.interval * 2);
        assert_eq!(budget, MAX_POLL_SECS);
    }

    #[test]
    fn a_short_expiry_is_still_given_two_polls() {
        let code = DeviceCode {
            device_code: "dc".into(),
            user_code: "A".into(),
            verification_uri_complete: String::new(),
            interval: 5,
            expires_in: 1,
        };
        let budget = code
            .expires_in
            .clamp(code.interval * 2, MAX_POLL_SECS)
            .max(code.interval * 2);
        assert_eq!(budget, 10);
    }

    #[test]
    fn an_oauth_error_body_parses_into_a_reportable_reason() {
        let body: TokenResponse = serde_json::from_str(
            r#"{"error":"invalid_grant","error_description":"Access denied"}"#,
        )
        .unwrap();
        assert_eq!(body.error, "invalid_grant");
        assert_eq!(body.error_description, "Access denied");
        assert!(body.access_token.is_empty());
    }
}
