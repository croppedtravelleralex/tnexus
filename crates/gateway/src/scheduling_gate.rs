//! Scheduling gate — shared with tnexus-api via `SCHEDULING_STATE_FILE` + live `ACCOUNTS_DB`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tnexus_accounts_db::AccountsDb;
use tracing::warn;

const STATE_VERIFIED: &str = "verified_ready";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct SchedulingStateFile {
    #[serde(default)]
    by_email: HashMap<String, String>,
}

pub struct SchedulingGate {
    scheduling_path: PathBuf,
    db: AccountsDb,
}

impl SchedulingGate {
    pub fn from_env() -> Self {
        let db = AccountsDb::from_env().unwrap_or_else(|e| {
            panic!("ACCOUNTS_DB required for scheduling gate: {e}");
        });
        Self {
            scheduling_path: std::env::var("SCHEDULING_STATE_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/scheduling_state.json")),
            db,
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
        self.db.accounts_by_email().unwrap_or_else(|e| {
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
        if key.is_empty() {
            return;
        }
        if let Err(e) = self.db.touch_inflight(&key, delta) {
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
