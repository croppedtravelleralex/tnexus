//! Environment-driven configuration.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub database_path: PathBuf,
    pub base_url: String,
    /// Bearer key for `/v1/*`. Empty disables the check (local use only).
    pub api_key: String,
    /// Bearer key for `/api/v1/*` and the admin page.
    pub admin_key: String,
    pub upstream_timeout_secs: u64,
    /// How many accounts one request may burn before giving up.
    pub max_attempts: usize,
    /// Egress used when an account carries no sticky `proxy_url`.
    pub default_proxy: String,
    /// `host:port` where the sticky relay really listens; rewrites the address
    /// baked into imported credentials while keeping their sticky user:pass.
    pub sticky_relay: String,
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            listen: env_or("GROKPROXY_ADDR", "0.0.0.0:8110"),
            database_path: PathBuf::from(env_or("GROKPROXY_DB", "/data/grokproxy.db")),
            base_url: env_or("GROKPROXY_UPSTREAM", crate::upstream::DEFAULT_BASE_URL),
            api_key: env_or("GROKPROXY_API_KEY", ""),
            admin_key: env_or("GROKPROXY_ADMIN_KEY", ""),
            upstream_timeout_secs: env_or("GROKPROXY_TIMEOUT_SECS", "120")
                .parse()
                .unwrap_or(120),
            max_attempts: env_or("GROKPROXY_MAX_ATTEMPTS", "3").parse().unwrap_or(3),
            default_proxy: env_or("GROKPROXY_PROXY", ""),
            sticky_relay: env_or("GROKPROXY_STICKY_RELAY", ""),
        }
    }

    /// Constant-time-ish bearer check. Empty configured key means "open".
    pub fn authorizes(expected: &str, header: Option<&str>) -> bool {
        if expected.is_empty() {
            return true;
        }
        let Some(header) = header else { return false };
        let presented = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
            .unwrap_or(header)
            .trim();
        // Length check first avoids comparing wildly different strings.
        presented.len() == expected.len()
            && presented
                .bytes()
                .zip(expected.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_expected_key_allows_anyone() {
        assert!(Config::authorizes("", None));
        assert!(Config::authorizes("", Some("Bearer whatever")));
    }

    #[test]
    fn missing_header_is_rejected_when_a_key_is_set() {
        assert!(!Config::authorizes("secret", None));
    }

    #[test]
    fn bearer_prefix_is_optional_and_case_tolerant() {
        assert!(Config::authorizes("secret", Some("Bearer secret")));
        assert!(Config::authorizes("secret", Some("bearer secret")));
        assert!(Config::authorizes("secret", Some("secret")));
    }

    #[test]
    fn wrong_key_is_rejected() {
        assert!(!Config::authorizes("secret", Some("Bearer nope")));
        assert!(!Config::authorizes("secret", Some("Bearer secrets")));
    }
}
