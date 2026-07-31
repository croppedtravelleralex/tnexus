use anyhow::{bail, Context, Result};
use helper_client::PinAccount;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataPlane {
    Helper,
    Upstream,
}

impl DataPlane {
    pub fn from_env() -> Self {
        match env::var("DATA_PLANE")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
        {
            Some(ref s) if s == "helper" => Self::Helper,
            Some(ref s) if s == "upstream" => Self::Upstream,
            None => Self::Upstream,
            Some(_) => Self::Upstream,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Helper => "helper",
            Self::Upstream => "upstream",
        }
    }
}

#[derive(Debug, Deserialize)]
struct AccountFile {
    email: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    proxy: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
}

pub struct Config {
    pub listen: String,
    pub helper_url: String,
    pub data_plane: DataPlane,
    pub account: PinAccount,
    pub accounts: HashMap<String, PinAccount>,
    pub account_email_log: String,
    pub min_image_quota: i64,
    pub image_global_concurrency: usize,
    pub image_sem: Arc<Semaphore>,
    pub image_enabled: bool,
    pub public_base_url: String,
}

pub fn load() -> Result<Config> {
    let listen = env::var("GATEWAY_LISTEN").unwrap_or_else(|_| "0.0.0.0:8013".into());
    let helper_url = env::var("HELPER_URL").unwrap_or_else(|_| "http://127.0.0.1:19001".into());
    let min_image_quota = env::var("MVP_MIN_IMAGE_QUOTA")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1)
        .max(0);
    let image_global_concurrency = env::var("IMAGE_GLOBAL_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1);
    let image_enabled = env::var("IMAGE_ENABLED")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let data_plane = DataPlane::from_env();

    let account = if let Ok(path) = env::var("PIN_ACCOUNT_FILE") {
        load_account_file(PathBuf::from(path))?
    } else if let Ok(raw) = env::var("PIN_ACCOUNT_JSON") {
        let af: AccountFile = serde_json::from_str(&raw).context("parse PIN_ACCOUNT_JSON")?;
        to_pin(af)
    } else {
        bail!("set PIN_ACCOUNT_FILE or PIN_ACCOUNT_JSON");
    };

    let mut accounts = HashMap::new();
    accounts.insert(account.email.to_lowercase(), account.clone());
    if let Ok(path) = env::var("ACCOUNTS_FILE") {
        for af in load_accounts_file(PathBuf::from(path))? {
            let pin = to_pin(af);
            accounts.insert(pin.email.to_lowercase(), pin);
        }
    }

    let account_email_log = account.email.clone();
    let image_sem = Arc::new(Semaphore::new(image_global_concurrency));
    let public_base_url = env::var("GATEWAY_PUBLIC_BASE_URL")
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/')
        .to_string();
    Ok(Config {
        listen,
        helper_url,
        data_plane,
        account,
        accounts,
        account_email_log,
        min_image_quota,
        image_global_concurrency,
        image_sem,
        image_enabled,
        public_base_url,
    })
}

fn load_account_file(path: PathBuf) -> Result<PinAccount> {
    let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    let af: AccountFile = serde_json::from_str(&raw).context("parse account file")?;
    Ok(to_pin(af))
}

fn load_accounts_file(path: PathBuf) -> Result<Vec<AccountFile>> {
    let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    let parsed: Vec<AccountFile> =
        serde_json::from_str(&raw).context("parse ACCOUNTS_FILE as array")?;
    Ok(parsed)
}

fn to_pin(af: AccountFile) -> PinAccount {
    PinAccount {
        email: af.email,
        access_token: af.access_token,
        device_id: af.device_id,
        proxy: af.proxy,
        user_agent: af.user_agent,
    }
}
