//! Scheduling gate — shared with tnexus-api via `SCHEDULING_STATE_FILE` + `ACCOUNTS_FILE`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const STATE_VERIFIED: &str = "verified_ready";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct SchedulingStateFile {
    #[serde(default)]
    by_email: HashMap<String, String>,
}

pub struct SchedulingGate {
    scheduling_path: PathBuf,
    accounts_path: PathBuf,
}

impl SchedulingGate {
    pub fn from_env() -> Self {
        Self {
            scheduling_path: std::env::var("SCHEDULING_STATE_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/scheduling_state.json")),
            accounts_path: std::env::var("ACCOUNTS_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/accounts_pool.json")),
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
        let mut out = HashMap::new();
        if !self.accounts_path.exists() {
            return out;
        }
        let Ok(raw) = fs::read_to_string(&self.accounts_path) else {
            return out;
        };
        let Ok(items) = serde_json::from_str::<Vec<Value>>(&raw) else {
            return out;
        };
        for item in items {
            let email = item
                .get("email")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            if !email.is_empty() {
                out.insert(email, item);
            }
        }
        out
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
        if inflight > 0 {
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
        if key.is_empty() || !self.accounts_path.exists() {
            return;
        }
        let Ok(raw) = fs::read_to_string(&self.accounts_path) else {
            return;
        };
        let Ok(mut items) = serde_json::from_str::<Vec<Value>>(&raw) else {
            return;
        };
        let mut changed = false;
        for item in items.iter_mut() {
            let em = item
                .get("email")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            if em != key {
                continue;
            }
            let cur = item
                .get("image_inflight")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let next = (cur + delta).max(0);
            if let Some(obj) = item.as_object_mut() {
                obj.insert("image_inflight".into(), Value::from(next));
                changed = true;
            }
            break;
        }
        if changed {
            if let Some(parent) = self.accounts_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(serialized) = serde_json::to_string_pretty(&items) {
                let _ = fs::write(&self.accounts_path, serialized);
            }
        }
    }

    pub fn begin_inflight<'a>(&'a self, email: &str) -> InflightGuard<'a> {
        self.touch_inflight(email, 1);
        InflightGuard {
            gate: self,
            email: email.to_string(),
        }
    }
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
