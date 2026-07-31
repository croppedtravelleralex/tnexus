use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use wreq::Client;
use wreq_util::{Emulation, Platform, Profile};

/// Default JA3/JA4 reflection endpoint (overridable via `FP_ENDPOINT`).
pub fn fp_endpoint() -> String {
    std::env::var("FP_ENDPOINT").unwrap_or_else(|_| "https://tls.browserleaks.com/json".to_string())
}

#[derive(Debug, Clone, Copy)]
pub enum ChromeProfile {
    Chrome120,
    Chrome124,
    Chrome131,
}

impl ChromeProfile {
    pub fn from_impersonate(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "chrome131" => Self::Chrome131,
            "chrome124" => Self::Chrome124,
            _ => Self::Chrome120,
        }
    }

    fn wreq_profile(self) -> Profile {
        match self {
            Self::Chrome120 => Profile::Chrome120,
            Self::Chrome124 => Profile::Chrome124,
            Self::Chrome131 => Profile::Chrome131,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ClientPlatform {
    Windows,
    MacOS,
}

impl ClientPlatform {
    pub fn from_fp(platform: &str) -> Self {
        if platform.eq_ignore_ascii_case("macos") || platform.eq_ignore_ascii_case("mac") {
            Self::MacOS
        } else {
            Self::Windows
        }
    }

    fn wreq_platform(self) -> Platform {
        match self {
            Self::Windows => Platform::Windows,
            Self::MacOS => Platform::MacOS,
        }
    }
}

pub struct TlsClientBuilder {
    profile: ChromeProfile,
    platform: ClientPlatform,
    proxy: Option<String>,
    timeout: Duration,
    user_agent: Option<String>,
}

impl Default for TlsClientBuilder {
    fn default() -> Self {
        Self {
            profile: ChromeProfile::Chrome124,
            platform: ClientPlatform::Windows,
            proxy: None,
            timeout: Duration::from_secs(60),
            user_agent: None,
        }
    }
}

impl TlsClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn profile(mut self, profile: ChromeProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn platform(mut self, platform: ClientPlatform) -> Self {
        self.platform = platform;
        self
    }

    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    pub fn build(self) -> Result<Client> {
        let emulation = Emulation::builder()
            .profile(self.profile.wreq_profile())
            .platform(self.platform.wreq_platform())
            .http2(true)
            .build();

        let mut builder = Client::builder().emulation(emulation).timeout(self.timeout);

        if let Some(proxy) = self.proxy.filter(|p| !p.trim().is_empty()) {
            builder = builder.proxy(wreq::Proxy::all(proxy)?);
        }

        let client = builder.build()?;
        Ok(client)
    }
}

/// Probe TLS fingerprint via reflection service.
pub async fn probe_tls_fingerprint(client: &Client, endpoint: &str) -> Result<Value> {
    let resp = client
        .get(endpoint)
        .header("accept", "application/json")
        .send()
        .await
        .with_context(|| format!("GET {endpoint}"))?;
    let status = resp.status();
    let body = resp.text().await.context("read tls probe body")?;
    if !status.is_success() {
        anyhow::bail!("tls probe HTTP {status}: {}", &body[..body.len().min(240)]);
    }
    serde_json::from_str(&body).context("parse tls probe json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_from_impersonate() {
        assert!(matches!(
            ChromeProfile::from_impersonate("chrome131"),
            ChromeProfile::Chrome131
        ));
    }
}
