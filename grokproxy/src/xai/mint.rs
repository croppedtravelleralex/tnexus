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
use crate::model::{AccountImport, Health, Provider};
use crate::store::{RemintCandidate, Store};
use crate::upstream;

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

/// Consent refuses a datacenter IP, so an empty `proxy_url` still has to leave
/// through the sticky relay — pinned to the email so the several round trips
/// of one mint share an exit address.
pub fn effective_mint_proxy(email: &str, proxy_url: &str, sticky_relay: &str) -> String {
    let relay = sticky_relay.trim();
    let given = proxy_url.trim();
    if given.is_empty() {
        if relay.is_empty() {
            return String::new();
        }
        return format!("http://{}:sticky@{relay}", sticky_slot(email));
    }
    if relay.is_empty() {
        return given.to_string();
    }
    crate::upstream::rewrite_proxy_host(given, relay)
}

fn sticky_slot(email: &str) -> String {
    // FNV-1a: stable across processes, no extra crate, good enough to pin a
    // residential slot. The relay only needs "same user string → same exit".
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in email.trim().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("mint{hash:010x}")
}

pub async fn mint(store: &Store, request: &MintRequest, sticky_relay: &str) -> Result<MintOutcome> {
    let email = request.email.trim().to_lowercase();
    if email.is_empty() {
        anyhow::bail!("missing email");
    }

    let proxy = effective_mint_proxy(&email, &request.proxy_url, sticky_relay);
    let consent = Consent::new(&request.sso_token, &proxy, DEFAULT_TIMEOUT_SECS)
        .context("building the impersonating client")?;
    // Check the cookie before spending a device code on it: a rejected SSO
    // otherwise surfaces much later as an unexplained approval failure.
    consent.validate_sso().await.context("validating sso")?;

    let flow = DeviceFlow::new(&proxy, DEFAULT_TIMEOUT_SECS)?;
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
        "proxy_url": proxy,
    }))?;
    store.import(Some(Provider::Build), &[import], now)?;

    Ok(MintOutcome {
        email,
        user_code: code.user_code,
        expires_at,
    })
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RemintReport {
    pub attempted: usize,
    pub revived: usize,
    pub sso_rejected: usize,
    pub failed: usize,
    pub remaining: usize,
}

/// Re-mint Build credentials for `needs_reauth` accounts that still have a
/// sibling Web SSO. Dead cookies are marked on the Web row so they are not
/// retried; everything else is left for the next batch.
pub async fn remint_batch(
    store: &Store,
    sticky_relay: &str,
    limit: usize,
    concurrency: usize,
) -> Result<RemintReport> {
    let candidates = store.remint_candidates(limit)?;
    let mut report = RemintReport {
        attempted: candidates.len(),
        ..RemintReport::default()
    };
    let width = concurrency.clamp(1, 8);
    for chunk in candidates.chunks(width) {
        let outcomes = futures::future::join_all(chunk.iter().map(|candidate| {
            let store = store.clone();
            let sticky = sticky_relay.to_string();
            let candidate = candidate.clone();
            async move { remint_one(&store, &sticky, candidate).await }
        }))
        .await;
        for outcome in outcomes {
            match outcome {
                RemintOutcome::Revived => report.revived += 1,
                RemintOutcome::SsoRejected => report.sso_rejected += 1,
                RemintOutcome::Failed => report.failed += 1,
            }
        }
    }
    report.remaining = store.remint_candidate_count()?;
    Ok(report)
}

enum RemintOutcome {
    Revived,
    SsoRejected,
    Failed,
}

async fn remint_one(
    store: &Store,
    sticky_relay: &str,
    candidate: RemintCandidate,
) -> RemintOutcome {
    let request = MintRequest {
        email: candidate.email.clone(),
        sso_token: candidate.sso_token,
        proxy_url: candidate.proxy_url,
    };
    match mint(store, &request, sticky_relay).await {
        Ok(_) => {
            info!(email = %candidate.email, "reminted build credential");
            RemintOutcome::Revived
        }
        Err(err) => {
            let msg = format!("{err:#}");
            if sso_is_dead(&msg) {
                if let Err(mark_err) = store.mark_health(
                    candidate.web_id,
                    Health::NeedsReauth,
                    0,
                    &upstream::truncate(&msg, 300),
                    crate::now(),
                ) {
                    tracing::warn!(
                        email = %candidate.email,
                        error = %mark_err,
                        "failed to blacklist a rejected sso"
                    );
                }
                tracing::warn!(email = %candidate.email, "remint skipped: sso rejected");
                RemintOutcome::SsoRejected
            } else {
                tracing::warn!(email = %candidate.email, error = %msg, "remint failed");
                RemintOutcome::Failed
            }
        }
    }
}

fn sso_is_dead(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("sso rejected")
        || lower.contains("sso not accepted")
        || lower.contains("missing sso")
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
            "",
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
            "",
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("missing sso"), "{err:#}");
    }

    #[test]
    fn an_empty_proxy_is_pinned_to_the_sticky_relay() {
        // Otherwise mint leaves through the datacenter IP and the edge blocks it.
        let proxy = effective_mint_proxy("a@b.c", "", "127.0.0.1:18100");
        assert!(proxy.starts_with("http://mint"), "{proxy}");
        assert!(proxy.ends_with("@127.0.0.1:18100"), "{proxy}");
        assert_eq!(
            effective_mint_proxy("a@b.c", "", "127.0.0.1:18100"),
            proxy,
            "the same email must reuse the same exit"
        );
        assert_ne!(
            effective_mint_proxy("other@b.c", "", "127.0.0.1:18100"),
            proxy,
            "different emails must not share an exit"
        );
    }

    #[test]
    fn a_caller_supplied_proxy_keeps_its_credentials_on_the_local_relay() {
        // Imported URLs name whatever host the registration machine used; only
        // the user:pass is the sticky slot, the address is deployment-specific.
        assert_eq!(
            effective_mint_proxy(
                "a@b.c",
                "http://mail-bob:sticky@172.20.0.1:18100",
                "127.0.0.1:18100",
            ),
            "http://mail-bob:sticky@127.0.0.1:18100"
        );
    }

    #[test]
    fn without_a_relay_the_supplied_proxy_is_left_alone() {
        assert_eq!(
            effective_mint_proxy("a@b.c", "http://u:p@host:1", ""),
            "http://u:p@host:1"
        );
        assert_eq!(effective_mint_proxy("a@b.c", "", ""), "");
    }

    #[test]
    fn a_rejected_cookie_is_terminal_for_remint() {
        assert!(sso_is_dead("sso rejected: https://accounts.x.ai/sign-in"));
        assert!(sso_is_dead("sso not accepted at the device page"));
        assert!(sso_is_dead("missing sso token"));
        assert!(!sso_is_dead(
            "egress blocked by the edge, not an account problem"
        ));
    }

    #[tokio::test]
    async fn remint_with_no_candidates_does_not_touch_the_network() {
        let store = Store::open_in_memory().unwrap();
        let report = remint_batch(&store, "127.0.0.1:18100", 10, 2)
            .await
            .unwrap();
        assert_eq!(report.attempted, 0);
        assert_eq!(report.remaining, 0);
    }
}
