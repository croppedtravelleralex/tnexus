//! Approving the device on `accounts.x.ai`.
//!
//! This host refuses reqwest's TLS signature with 403 and answers a browser
//! signature, so every call here goes through wreq with a Chrome emulation
//! profile. See the module docs in `mod.rs` for the measurements.
//!
//! The page is a Next.js app, and it exposes approval two ways depending on the
//! deploy: a plain HTML form, or a server action invoked with a `next-action`
//! header. Both are handled, because which one is live has changed before.

use std::time::Duration;

use anyhow::{bail, Result};
use tracing::debug;

use super::scrape::{self, PageKind};

pub const ACCOUNTS_BASE_URL: &str = "https://accounts.x.ai";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36";

pub struct Consent {
    client: wreq::Client,
    sso: String,
}

/// A page the flow landed on, reduced to what the next step needs.
struct Landing {
    kind: PageKind,
    url: String,
    html: String,
}

impl Consent {
    pub fn new(sso: &str, proxy_url: &str, timeout_secs: u64) -> Result<Self> {
        let mut builder = wreq::Client::builder()
            // Chrome137 is the newest profile wreq-util ships; the signature it
            // presents is what gets past the edge, not the version string.
            .emulation(wreq_util::Emulation::Chrome137)
            .timeout(Duration::from_secs(timeout_secs))
            .redirect(wreq::redirect::Policy::limited(10))
            .cookie_store(true);
        if !proxy_url.is_empty() {
            builder = builder.proxy(wreq::Proxy::all(proxy_url)?);
        }
        Ok(Consent {
            client: builder.build()?,
            sso: scrape::normalize_sso(sso),
        })
    }

    fn cookie_header(&self) -> String {
        // Both names are set: which one the edge honours has varied.
        format!("sso={0}; sso-rw={0}", self.sso)
    }

    async fn get(&self, url: &str, referer: &str) -> Result<Landing> {
        let mut request = self
            .client
            .get(url)
            .header("user-agent", UA)
            .header(
                "accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("accept-language", "en-US,en;q=0.9")
            .header("cookie", self.cookie_header());
        if !referer.is_empty() {
            request = request.header("referer", referer);
        }
        let response = request.send().await?;
        let final_url = response.url().to_string();
        let html = response.text().await.unwrap_or_default();
        Ok(Landing {
            kind: scrape::classify_page(&final_url, &html),
            url: final_url,
            html,
        })
    }

    /// Confirm the SSO cookie is accepted before spending a device code on it.
    pub async fn validate_sso(&self) -> Result<()> {
        if self.sso.is_empty() {
            bail!("missing sso token");
        }
        let landing = self.get(ACCOUNTS_BASE_URL, "").await?;
        match landing.kind {
            PageKind::Blocked => bail!("egress blocked by the edge, not an account problem"),
            PageKind::SignIn => bail!("sso rejected: {}", short(&landing.url)),
            _ => Ok(()),
        }
    }

    /// Approve the device, leaving it ready for the token poll.
    ///
    /// `verification_uri` is the address the upstream handed out with the code;
    /// following it rather than a locally built one keeps working if the path
    /// changes.
    pub async fn approve(
        &self,
        user_code: &str,
        verification_uri: &str,
        email: &str,
    ) -> Result<()> {
        let code = scrape::normalize_user_code(user_code);
        let device_url = if verification_uri.is_empty() {
            format!("{ACCOUNTS_BASE_URL}/oauth2/device?user_code={code}")
        } else {
            verification_uri.to_string()
        };
        let landing = self
            .get(&device_url, &format!("{ACCOUNTS_BASE_URL}/"))
            .await?;
        debug!(kind = ?landing.kind, url = %short(&landing.url), "device page");

        match landing.kind {
            PageKind::Blocked => bail!("egress blocked at the device page"),
            PageKind::SignIn => bail!("sso not accepted at the device page"),
            // Already approved, nothing to submit.
            PageKind::Done => return Ok(()),
            _ => {}
        }

        let after = self.submit_allow(&landing, &code, email).await?;
        match after {
            PageKind::Done | PageKind::Consent => Ok(()),
            PageKind::Blocked => bail!("egress blocked while approving"),
            other => bail!("approval landed on an unexpected page: {other:?}"),
        }
    }

    async fn submit_allow(&self, page: &Landing, code: &str, email: &str) -> Result<PageKind> {
        if let Some(form) = scrape::find_post_form(&page.html, &page.url) {
            if form.method == "post" {
                return self.submit_html_form(page, form, code, email).await;
            }
        }
        self.submit_server_action(page, code).await
    }

    async fn submit_html_form(
        &self,
        page: &Landing,
        form: scrape::HtmlForm,
        code: &str,
        email: &str,
    ) -> Result<PageKind> {
        let mut fields = form.fields;
        fields.insert("user_code".into(), code.to_string());
        fields
            .entry("action".into())
            .and_modify(|value| {
                if value.is_empty() {
                    *value = "allow".into();
                }
            })
            .or_insert_with(|| "allow".into());

        // The rendered form ships principal_id empty and React fills it.
        // Submitting it empty is accepted and then the token poll fails with
        // "Access denied", which reads as a dead credential rather than a
        // missing field — so refuse rather than approve a doomed device.
        if fields
            .get("principal_id")
            .map(String::is_empty)
            .unwrap_or(true)
        {
            let Some(principal) = scrape::find_principal_id(&page.html, Some(email)) else {
                bail!("consent page has no principal id for {email}");
            };
            fields.insert("principal_id".into(), principal);
            fields
                .entry("principal_type".into())
                .or_insert_with(|| "User".into());
        }

        debug!(action = %short(&form.action), "approving via html form");
        let pairs: Vec<(String, String)> = fields.into_iter().collect();
        let response = self
            .client
            .post(&form.action)
            .header("user-agent", UA)
            .header("referer", &page.url)
            .header("origin", scrape::origin_of(&form.action))
            .header("cookie", self.cookie_header())
            .form(&pairs)
            .send()
            .await?;
        let url = response.url().to_string();
        let html = response.text().await.unwrap_or_default();
        Ok(scrape::classify_page(&url, &html))
    }

    async fn submit_server_action(&self, page: &Landing, code: &str) -> Result<PageKind> {
        let Some(action_id) = scrape::find_server_action_id(&page.html) else {
            bail!("consent page exposes neither a form nor a server action");
        };
        let post_url = page.url.split('?').next().unwrap_or(&page.url).to_string();
        let body = serde_json::to_string(&serde_json::json!([{
            "action": "allow",
            "userCode": code,
        }]))?;

        debug!(action = %&action_id[..12.min(action_id.len())], "approving via server action");
        let response = self
            .client
            .post(&post_url)
            .header("user-agent", UA)
            .header("accept", "text/x-component")
            .header("content-type", "text/plain;charset=UTF-8")
            .header("origin", ACCOUNTS_BASE_URL)
            .header("referer", &page.url)
            .header("next-action", action_id)
            .header("cookie", self.cookie_header())
            .body(body)
            .send()
            .await?;
        let status = response.status().as_u16();
        let url = response.url().to_string();
        let html = response.text().await.unwrap_or_default();
        let kind = scrape::classify_page(&url, &html);
        if kind == PageKind::Unknown && (200..400).contains(&status) {
            // A server action answers with a flight payload, not a page, so a
            // 2xx with no recognisable page is the success shape here.
            return Ok(PageKind::Done);
        }
        Ok(kind)
    }
}

fn short(url: &str) -> String {
    crate::upstream::truncate(url, 120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cookie_carries_both_sso_names() {
        let consent = Consent::new("sso=abc; Path=/", "", 30).unwrap();
        assert_eq!(consent.cookie_header(), "sso=abc; sso-rw=abc");
    }

    #[test]
    fn an_empty_sso_is_refused_before_any_request() {
        let consent = Consent::new("   ", "", 30).unwrap();
        assert!(consent.sso.is_empty());
    }

    #[tokio::test]
    async fn validating_an_empty_sso_fails_without_touching_the_network() {
        let consent = Consent::new("", "", 30).unwrap();
        let err = consent.validate_sso().await.unwrap_err();
        assert!(err.to_string().contains("missing sso"));
    }
}
