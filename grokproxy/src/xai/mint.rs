//! Turning a web SSO cookie into a Build credential, in-process.
//!
//! Previously this ran on the registration machine: mint to a JSON file, then
//! ship the file over SSH or HTTP, then import it. Doing it here collapses that
//! into one step — the browser's only job is to produce an SSO cookie, and the
//! account lands directly in the pool that is about to schedule it.

use anyhow::{Context, Result};
use tracing::info;

use super::consent::Consent;
use super::device::DeviceFlow;
use crate::model::{AccountImport, Provider};
use crate::store::Store;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MintRequest {
    pub email: String,
    /// Raw cookie value, or the whole `sso=...; Path=/` string.
    pub sso_token: String,
    /// Sticky egress for this account. The consent host judges the IP as well
    /// as the TLS signature, so a datacenter address is refused outright.
    #[serde(default)]
    pub proxy_url: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MintOutcome {
    pub email: String,
    pub user_code: String,
    pub expires_at: i64,
}

/// Default per-call ceiling. The device flow is several round trips plus a
/// poll, so this is deliberately looser than a chat timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

pub async fn mint(store: &Store, request: &MintRequest) -> Result<MintOutcome> {
    let email = request.email.trim().to_lowercase();
    if email.is_empty() {
        anyhow::bail!("missing email");
    }

    let consent = Consent::new(&request.sso_token, &request.proxy_url, DEFAULT_TIMEOUT_SECS)
        .context("building the impersonating client")?;
    // Check the cookie before spending a device code on it: a rejected SSO
    // otherwise surfaces much later as an unexplained approval failure.
    consent.validate_sso().await.context("validating sso")?;

    let flow = DeviceFlow::new(&request.proxy_url, DEFAULT_TIMEOUT_SECS)?;
    let code = flow
        .request_code()
        .await
        .context("requesting device code")?;
    info!(email = %email, user_code = %code.user_code, "minting build credential");

    consent
        .approve(&code.user_code, &code.verification_uri_complete, &email)
        .await
        .context("approving the device")?;
    let tokens = flow
        .poll_token(&code)
        .await
        .context("polling for the token")?;

    let now = crate::now();
    let expires_at = now + tokens.expires_in;
    let import: AccountImport = serde_json::from_value(serde_json::json!({
        "email": email,
        "access_token": tokens.access_token,
        "refresh_token": tokens.refresh_token,
        "id_token": tokens.id_token,
        "expires_at": expires_at,
        "proxy_url": request.proxy_url,
    }))?;
    store.import(Some(Provider::Build), &[import], now)?;

    Ok(MintOutcome {
        email,
        user_code: code.user_code,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_without_an_email_is_refused_before_any_network_call() {
        let store = Store::open_in_memory().unwrap();
        let err = mint(
            &store,
            &MintRequest {
                email: "  ".into(),
                sso_token: "abc".into(),
                proxy_url: String::new(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("missing email"));
    }

    #[tokio::test]
    async fn a_request_without_an_sso_never_spends_a_device_code() {
        // Requesting a code first would burn one per bad request and trip the
        // upstream's rate limit on the shared client id.
        let store = Store::open_in_memory().unwrap();
        let err = mint(
            &store,
            &MintRequest {
                email: "a@b.c".into(),
                sso_token: String::new(),
                proxy_url: String::new(),
            },
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("missing sso"), "{err:#}");
    }
}
