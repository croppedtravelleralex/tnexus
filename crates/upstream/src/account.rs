use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Minimal pin account shape (`deploy/pin_account.json.example`).
#[derive(Debug, Clone, Deserialize)]
pub struct PinAccount {
    pub email: String,
    pub access_token: String,
    #[serde(default)]
    pub device_id: String,
    pub proxy: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub impersonate: String,
}

impl PinAccount {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path.as_ref())
            .with_context(|| format!("read pin account {}", path.as_ref().display()))?;
        serde_json::from_str(&raw).context("parse pin account json")
    }

    pub fn redacted_email(&self) -> String {
        let email = self.email.trim();
        if let Some((local, domain)) = email.split_once('@') {
            let prefix = local.chars().take(2).collect::<String>();
            format!("{prefix}***@{domain}")
        } else if email.is_empty() {
            "(empty)".into()
        } else {
            "***".into()
        }
    }
}
