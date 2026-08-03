//! Scheduling gate — shared with tnexus-api via `SCHEDULING_STATE_FILE` + live `ACCOUNTS_DB`.

use helper_client::PinAccount;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tnexus_accounts_db::AccountsBackend;
use tracing::warn;

const STATE_VERIFIED: &str = "verified_ready";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct SchedulingStateFile {
    #[serde(default)]
    by_email: HashMap<String, String>,
}

#[derive(Clone)]
pub struct SchedulingGate {
    scheduling_path: PathBuf,
    backend: AccountsBackend,
    /// 0 = unlimited concurrent inflight per account
    account_inflight_max: i64,
}

impl SchedulingGate {
    pub fn from_env() -> Self {
        Self::from_backend(AccountsBackend::from_env(None).unwrap_or_else(|e| {
            panic!("ACCOUNTS_DB required for scheduling gate: {e}");
        }))
    }

    pub fn from_backend(backend: AccountsBackend) -> Self {
        let gate = Self {
            scheduling_path: std::env::var("SCHEDULING_STATE_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/scheduling_state.json")),
            backend,
            account_inflight_max: std::env::var("IMAGE_ACCOUNT_INFLIGHT_MAX")
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0),
        };
        gate.reconcile_stale_inflight();
        gate
    }

    /// Clamp runaway inflight counters left behind by crashed workers or leaked guards.
    pub fn reconcile_stale_inflight(&self) {
        let ceiling = if self.account_inflight_max > 0 {
            self.account_inflight_max.saturating_mul(4)
        } else {
            8
        };
        match self.backend.reconcile_inflight_above(ceiling) {
            Ok(n) if n > 0 => {
                warn!(
                    count = n,
                    ceiling,
                    "reset stale image_inflight counters"
                );
            }
            Err(e) => warn!(error = %e, "reconcile_stale_inflight failed"),
            _ => {}
        }
    }

    fn load_scheduling(&self) -> HashMap<String, String> {
        if !self.scheduling_path.exists() {
            return HashMap::new();
        }
        let raw = fs::read_to_string(&self.scheduling_path).unwrap_or_default();
        if raw.trim().is_empty() {
            return HashMap::new();
        }
        serde_json::from_str::<SchedulingStateFile>(&raw)
            .map(|f| f.by_email)
            .unwrap_or_default()
    }

    fn save_scheduling(&self, map: &HashMap<String, String>) -> std::io::Result<()> {
        if let Some(parent) = self.scheduling_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = SchedulingStateFile {
            by_email: map.clone(),
        };
        let raw = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into());
        fs::write(&self.scheduling_path, raw)
    }

    fn load_accounts_by_email(&self) -> HashMap<String, Value> {
        self.backend.accounts_by_email().unwrap_or_else(|e| {
            warn!(error=%e, "load accounts from sqlite failed");
            HashMap::new()
        })
    }

    fn manual_scheduling_enabled(receive_state: &str) -> bool {
        let receive = receive_state.trim().to_lowercase();
        if receive.is_empty() {
            return true;
        }
        matches!(
            receive.as_str(),
            "verified_ready" | "verified" | "local_verified"
        )
    }

    pub fn is_email_schedulable(&self, email: &str, access_token: &str) -> bool {
        let key = email.trim().to_lowercase();
        if access_token.trim().is_empty() {
            return false;
        }
        let scheduling = self.load_scheduling();
        let receive = scheduling.get(&key).map(String::as_str).unwrap_or("");
        if !Self::manual_scheduling_enabled(receive) {
            return false;
        }
        let accounts = self.load_accounts_by_email();
        let Some(row) = accounts.get(&key) else {
            return true;
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("正常");
        if status != "正常" {
            return false;
        }
        if row
            .get("soft_band_percent")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0
        {
            return false;
        }
        let inflight = row
            .get("image_inflight")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if self.account_inflight_max > 0 && inflight >= self.account_inflight_max {
            return false;
        }
        let quota = row.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
        let unknown = row
            .get("image_quota_unknown")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if quota <= 0 && !unknown {
            return false;
        }
        true
    }

    pub fn set_bulk(&self, emails: &[String], enabled: bool) -> usize {
        let mut scheduling = self.load_scheduling();
        let next = if enabled {
            STATE_VERIFIED.to_string()
        } else {
            "identity_isolated".to_string()
        };
        let mut updated = 0usize;
        for email in emails {
            let key = email.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            scheduling.insert(key, next.clone());
            updated += 1;
        }
        let _ = self.save_scheduling(&scheduling);
        updated
    }

    pub fn touch_inflight(&self, email: &str, delta: i64) {
        let key = email.trim().to_lowercase();
        if key.is_empty() {
            return;
        }
        if let Err(e) = self.backend.touch_inflight(&key, delta) {
            warn!(error=%e, email = %key, "touch_inflight failed");
        }
    }

    pub fn begin_inflight<'a>(&'a self, email: &str) -> InflightGuard<'a> {
        self.touch_inflight(email, 1);
        InflightGuard {
            gate: self,
            email: email.to_string(),
        }
    }

    pub fn decrement_quota(&self, email: &str) -> anyhow::Result<Option<(i64, i64)>> {
        self.backend.decrement_quota(email, 1)
    }

    pub fn account_inflight_cap(&self) -> u64 {
        self.account_inflight_max.max(0) as u64
    }

    pub fn account_metrics(&self, email: &str) -> Option<(i64, bool, i64, i64)> {
        let key = email.trim().to_lowercase();
        let accounts = self.load_accounts_by_email();
        let row = accounts.get(&key)?;
        let quota = row.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
        let unknown = row
            .get("image_quota_unknown")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let inflight = row.get("image_inflight").and_then(|v| v.as_i64()).unwrap_or(0);
        let soft = row
            .get("soft_band_percent")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Some((quota, unknown, inflight, soft))
    }

    /// All pool rows as API-shaped JSON (postgres or sqlite backend).
    pub fn list_account_items_for_api(&self) -> Vec<Value> {
        let accounts = self.load_accounts_by_email();
        let mut items: Vec<Value> = accounts
            .values()
            .map(|row| self.row_to_api_item(row))
            .collect();
        items.sort_by(|a, b| {
            let ae = a.get("email").and_then(|v| v.as_str()).unwrap_or("");
            let be = b.get("email").and_then(|v| v.as_str()).unwrap_or("");
            ae.cmp(be)
        });
        items
    }

    pub fn pool_account_count(&self) -> usize {
        self.load_accounts_by_email().len()
    }

    pub fn schedulable_count(&self) -> usize {
        self.list_schedulable_pins().len()
    }

    /// Pin accounts eligible for image dispatch (humanlike pool).
    pub fn list_schedulable_pins(&self) -> Vec<PinAccount> {
        let accounts = self.load_accounts_by_email();
        let mut out: Vec<PinAccount> = accounts
            .values()
            .filter_map(|row| {
                let pin = value_to_pin(row)?;
                if self.is_email_schedulable(&pin.email, &pin.access_token) {
                    Some(pin)
                } else {
                    None
                }
            })
            .collect();
        out.sort_by(|a, b| a.email.cmp(&b.email));
        out
    }

    /// All pool rows as pins (admin list / reload).
    pub fn list_all_pins(&self) -> Vec<PinAccount> {
        let accounts = self.load_accounts_by_email();
        let mut out: Vec<PinAccount> = accounts.values().filter_map(value_to_pin).collect();
        out.sort_by(|a, b| a.email.cmp(&b.email));
        out
    }

    fn row_to_api_item(&self, row: &Value) -> Value {
        let email = row
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        let token = row
            .get("access_token")
            .or_else(|| row.get("accessToken"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or(if token.is_empty() { "异常" } else { "正常" });
        let schedulable = self.is_email_schedulable(email, token);
        let mut obj = match row.clone() {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        obj.insert("email".into(), Value::String(email.to_string()));
        obj.insert("access_token".into(), Value::String(token.to_string()));
        if !obj.contains_key("status") {
            obj.insert("status".into(), Value::String(status.to_string()));
        }
        obj.insert("image_schedulable".into(), Value::Bool(schedulable));
        Value::Object(obj)
    }
}

fn value_to_pin(row: &Value) -> Option<PinAccount> {
    let email = row
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)?;
    let access_token = row
        .get("access_token")
        .or_else(|| row.get("accessToken"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let device_id = row
        .get("device_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let proxy = row
        .get("proxy")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let user_agent = row
        .get("user_agent")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(PinAccount {
        email,
        access_token,
        device_id,
        proxy,
        user_agent,
    })
}

pub struct InflightGuard<'a> {
    gate: &'a SchedulingGate,
    email: String,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.gate.touch_inflight(&self.email, -1);
    }
}
