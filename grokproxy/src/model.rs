//! Core domain types for the account pool.

use serde::{Deserialize, Serialize};

/// Which upstream an account can talk to.
///
/// `Build` accounts hold an OIDC refresh/access token pair and reach the free
/// promo model through `cli-chat-proxy`. `Web` accounts hold a grok.com SSO
/// cookie; they are stored and scheduled here but their chat path needs the
/// request signer, which this service does not implement yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Build,
    Web,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Build => "build",
            Provider::Web => "web",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "build" | "grok_build" => Some(Provider::Build),
            "web" | "grok_web" => Some(Provider::Web),
            _ => None,
        }
    }
}

/// Why an account is or is not schedulable.
///
/// The distinction that matters operationally: `NeedsReauth` is terminal until
/// a human or the register pipeline supplies new credentials, while `Cooling`
/// resolves on its own. Conflating them is what makes a pool look dead when it
/// is merely throttled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// Ready to serve.
    #[default]
    Active,
    /// Temporarily withheld (rate limit, quota window, transient upstream error).
    Cooling,
    /// Refresh token rejected; credentials must be replaced.
    NeedsReauth,
    /// Upstream refuses chat for this account (entitlement), retry is pointless.
    Forbidden,
    /// Operator switched it off.
    Disabled,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Active => "active",
            Health::Cooling => "cooling",
            Health::NeedsReauth => "needs_reauth",
            Health::Forbidden => "forbidden",
            Health::Disabled => "disabled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Health::Active,
            "cooling" => Health::Cooling,
            "needs_reauth" => Health::NeedsReauth,
            "forbidden" => Health::Forbidden,
            _ => Health::Disabled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Account {
    pub id: i64,
    pub provider: Provider,
    pub email: String,
    pub health: Health,
    /// Build: OIDC access token. Web: unused.
    pub access_token: String,
    /// Build: OIDC refresh token. Rotated on every refresh — always persisted.
    pub refresh_token: String,
    /// Web: grok.com `sso` cookie value.
    pub sso_token: String,
    /// Unix seconds when `access_token` expires; 0 when unknown.
    pub expires_at: i64,
    /// Per-account sticky egress, e.g. `http://user:pass@host:port`.
    pub proxy_url: String,
    /// Extra upstream headers stored verbatim from the mint archive.
    pub headers: serde_json::Value,
    /// Model id last advertised by the upstream for this account.
    pub last_model: String,
    pub last_used_at: i64,
    pub cooling_until: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub last_error: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    /// `cost_in_usd_ticks` accumulated; 1e7 ticks = 1 USD.
    pub cost_ticks: i64,
    /// Last time this account actually served a request; 0 = never proven.
    pub verified_at: i64,
    /// Quota from the upstream's x-ratelimit-* headers; -1 = never observed.
    pub limit_tokens: i64,
    pub remaining_tokens: i64,
    pub limit_requests: i64,
    pub remaining_requests: i64,
    pub quota_checked_at: i64,
}

impl Account {
    /// Eligible right now, taking a finished cooldown into account.
    pub fn is_available(&self, now: i64) -> bool {
        match self.health {
            Health::Active => true,
            Health::Cooling => self.cooling_until > 0 && now >= self.cooling_until,
            _ => false,
        }
    }

    /// Refresh before the token actually expires; a token that dies mid-request
    /// costs a retry and pollutes failure stats.
    pub fn needs_refresh(&self, now: i64, skew_secs: i64) -> bool {
        self.provider == Provider::Build
            && (self.access_token.is_empty() || self.expires_at <= now + skew_secs)
    }
}

/// One account as submitted by the register pipeline.
#[derive(Debug, Clone, Deserialize)]
pub struct AccountImport {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub sso_token: Option<String>,
    #[serde(default)]
    pub sso: Option<String>,
    #[serde(default)]
    pub expires_at: Option<serde_json::Value>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub headers: Option<serde_json::Value>,
}

impl AccountImport {
    pub fn resolved_email(&self) -> String {
        [self.email.as_deref(), self.name.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    }

    pub fn resolved_sso(&self) -> String {
        [self.sso_token.as_deref(), self.sso.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_default()
    }

    /// The register pipeline has historically sent `expires_at` as a number, a
    /// numeric string, or an RFC3339 stamp. Accept all three rather than
    /// dropping the field.
    pub fn resolved_expires_at(&self) -> i64 {
        match self.expires_at.as_ref() {
            Some(serde_json::Value::Number(number)) => number.as_i64().unwrap_or(0),
            Some(serde_json::Value::String(text)) => {
                let text = text.trim();
                if let Ok(value) = text.parse::<i64>() {
                    return value;
                }
                crate::jwt::parse_rfc3339_secs(text).unwrap_or(0)
            }
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportRequest {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub accounts: Vec<AccountImport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub inserted: usize,
    pub updated: usize,
    pub skipped: Vec<String>,
}

/// Public view of an account: never leaks tokens.
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: i64,
    pub provider: &'static str,
    pub email: String,
    pub health: &'static str,
    pub last_model: String,
    pub has_proxy: bool,
    pub expires_at: i64,
    pub last_used_at: i64,
    pub cooling_until: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub last_error: String,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    /// Accumulated spend in USD, derived from the upstream's cost ticks.
    pub cost_usd: f64,
    /// Quota last reported by the upstream; null = never observed.
    pub remaining_tokens: Option<i64>,
    pub limit_tokens: Option<i64>,
    pub remaining_requests: Option<i64>,
    pub limit_requests: Option<i64>,
    /// False when the account has never served a request — `active` alone only
    /// means "imported", not "known good".
    pub verified: bool,
}

/// The upstream reports cost in ten-millionths of a dollar.
const COST_TICKS_PER_USD: f64 = 1e7;

impl From<&Account> for AccountView {
    fn from(account: &Account) -> Self {
        AccountView {
            id: account.id,
            provider: account.provider.as_str(),
            email: account.email.clone(),
            health: account.health.as_str(),
            last_model: account.last_model.clone(),
            has_proxy: !account.proxy_url.is_empty(),
            expires_at: account.expires_at,
            last_used_at: account.last_used_at,
            cooling_until: account.cooling_until,
            success_count: account.success_count,
            failure_count: account.failure_count,
            last_error: account.last_error.clone(),
            total_tokens: account.total_tokens,
            prompt_tokens: account.prompt_tokens,
            completion_tokens: account.completion_tokens,
            cost_usd: (account.cost_ticks as f64 / COST_TICKS_PER_USD * 1e6).round() / 1e6,
            verified: account.verified_at > 0,
            remaining_tokens: (account.remaining_tokens >= 0).then_some(account.remaining_tokens),
            limit_tokens: (account.limit_tokens >= 0).then_some(account.limit_tokens),
            remaining_requests: (account.remaining_requests >= 0)
                .then_some(account.remaining_requests),
            limit_requests: (account.limit_requests >= 0).then_some(account.limit_requests),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(health: Health, cooling_until: i64) -> Account {
        Account {
            id: 1,
            provider: Provider::Build,
            email: "a@b.c".into(),
            health,
            access_token: "token".into(),
            refresh_token: "refresh".into(),
            sso_token: String::new(),
            expires_at: 10_000,
            proxy_url: String::new(),
            headers: serde_json::Value::Null,
            last_model: String::new(),
            last_used_at: 0,
            cooling_until,
            success_count: 0,
            failure_count: 0,
            last_error: String::new(),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        }
    }

    #[test]
    fn cooling_account_returns_after_its_window() {
        let acct = account(Health::Cooling, 500);
        assert!(!acct.is_available(499));
        assert!(acct.is_available(500));
    }

    #[test]
    fn terminal_states_never_become_available() {
        for health in [Health::NeedsReauth, Health::Forbidden, Health::Disabled] {
            assert!(!account(health, 0).is_available(i64::MAX));
        }
    }

    #[test]
    fn refresh_happens_before_expiry_not_after() {
        let acct = account(Health::Active, 0);
        assert!(!acct.needs_refresh(0, 60));
        assert!(acct.needs_refresh(9_941, 60));
    }

    #[test]
    fn web_accounts_never_ask_for_oidc_refresh() {
        let mut acct = account(Health::Active, 0);
        acct.provider = Provider::Web;
        acct.access_token = String::new();
        assert!(!acct.needs_refresh(i64::MAX / 2, 60));
    }

    #[test]
    fn provider_accepts_pipeline_spelling() {
        assert_eq!(Provider::parse("grok_build"), Some(Provider::Build));
        assert_eq!(Provider::parse("grok_web"), Some(Provider::Web));
        assert_eq!(Provider::parse("nope"), None);
    }

    #[test]
    fn import_accepts_every_expires_at_shape() {
        let numeric: AccountImport = serde_json::from_str(r#"{"expires_at": 1786000000}"#).unwrap();
        assert_eq!(numeric.resolved_expires_at(), 1786000000);

        let stringy: AccountImport =
            serde_json::from_str(r#"{"expires_at": "1786000000"}"#).unwrap();
        assert_eq!(stringy.resolved_expires_at(), 1786000000);

        let rfc: AccountImport =
            serde_json::from_str(r#"{"expires_at": "2026-08-13T02:00:00Z"}"#).unwrap();
        assert!(rfc.resolved_expires_at() > 1_700_000_000);

        let missing: AccountImport = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.resolved_expires_at(), 0);
    }

    #[test]
    fn import_falls_back_from_email_to_name() {
        let named: AccountImport = serde_json::from_str(r#"{"name":"A@B.C"}"#).unwrap();
        assert_eq!(named.resolved_email(), "a@b.c");
    }
}
