//! In-memory ops services (nurture / quota prime / outlook / webshare / proxy).

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

fn proxy_runtime_path() -> PathBuf {
    std::env::var("PROXY_RUNTIME_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/proxy_runtime.json"))
}

#[derive(Clone)]
pub struct NurtureJob {
    pub access_token: String,
    pub prompt: String,
    pub email: String,
    pub queued_at: Instant,
}

#[derive(Clone)]
pub struct QuotaPrimeJob {
    pub access_token: String,
    pub email: String,
}

pub struct OpsServices {
    nurture_enabled: Mutex<bool>,
    nurture_jobs: Mutex<VecDeque<NurtureJob>>,
    nurture_completed_today: Mutex<u32>,
    nurture_last_error: Mutex<Option<String>>,
    quota_queue: Mutex<VecDeque<QuotaPrimeJob>>,
    quota_state: Mutex<Value>,
    outlook_settings: Mutex<Value>,
    outlook_progress: Mutex<HashMap<String, Value>>,
    webshare_last_run: Mutex<Option<Instant>>,
    webshare_inventory_cache: Mutex<Vec<Value>>,
}

impl OpsServices {
    pub fn new() -> Self {
        Self {
            nurture_enabled: Mutex::new(true),
            nurture_jobs: Mutex::new(VecDeque::new()),
            nurture_completed_today: Mutex::new(0),
            nurture_last_error: Mutex::new(None),
            quota_queue: Mutex::new(VecDeque::new()),
            quota_state: Mutex::new(json!({
                "running": false,
                "state": "idle",
                "queue": [],
                "queue_depth": 0,
                "processed": 0,
                "succeeded": 0,
                "failed": 0,
            })),
            outlook_settings: Mutex::new(json!({
                "enabled": false,
                "interval_minutes": 30,
            })),
            outlook_progress: Mutex::new(HashMap::new()),
            webshare_last_run: Mutex::new(None),
            webshare_inventory_cache: Mutex::new(Vec::new()),
        }
    }

    pub fn nurture_running(&self) -> bool {
        *self.nurture_enabled.lock().expect("nurture enabled")
    }

    pub fn nurture_status(&self) -> Value {
        let jobs = self.nurture_jobs.lock().expect("nurture jobs");
        let depth = jobs.len();
        let oldest = jobs
            .front()
            .map(|j| j.queued_at.elapsed().as_secs())
            .unwrap_or(0);
        json!({
            "running": self.nurture_running(),
            "worker_alive": true,
            "queue": { "depth": depth, "queued": depth, "oldest_age_sec": oldest },
            "completed_in_day": *self.nurture_completed_today.lock().expect("nurture completed"),
            "max_per_account_per_day": 24,
            "last_error": self.nurture_last_error.lock().expect("nurture err").clone(),
            "source": "tnexus-account-ops",
        })
    }

    pub fn nurture_enable(&self, enabled: bool) -> Value {
        *self.nurture_enabled.lock().expect("nurture enabled") = enabled;
        json!({ "ok": true, "enabled": enabled })
    }

    pub fn nurture_enqueue(
        &self,
        tokens: &[String],
        prompt: &str,
        emails: &HashMap<String, String>,
    ) -> Value {
        let prompt = if prompt.trim().is_empty() {
            crate::nurture::default_nurture_prompt()
        } else {
            prompt.trim().to_string()
        };
        let mut jobs = self.nurture_jobs.lock().expect("nurture jobs");
        let mut queued = 0usize;
        for t in tokens {
            let token = t.trim();
            if token.is_empty() {
                continue;
            }
            let email = emails.get(token).cloned().unwrap_or_default();
            jobs.push_back(NurtureJob {
                access_token: token.to_string(),
                prompt: prompt.clone(),
                email,
                queued_at: Instant::now(),
            });
            queued += 1;
        }
        json!({ "queued": queued, "depth": jobs.len(), "source": "tnexus-account-ops" })
    }

    pub fn pop_nurture_job(&self) -> Option<NurtureJob> {
        self.nurture_jobs.lock().expect("nurture jobs").pop_front()
    }

    pub fn record_nurture_success(&self) {
        *self
            .nurture_completed_today
            .lock()
            .expect("nurture completed") += 1;
        *self.nurture_last_error.lock().expect("nurture err") = None;
    }

    pub fn record_nurture_error(&self, err: String) {
        *self.nurture_last_error.lock().expect("nurture err") = Some(err);
    }

    pub fn nurture_process_one_sync(&self, token: &str) -> Option<NurtureJob> {
        let mut jobs = self.nurture_jobs.lock().expect("nurture jobs");
        if token.is_empty() {
            return jobs.pop_front();
        }
        let pos = jobs.iter().position(|j| j.access_token == token);
        pos.map(|i| jobs.remove(i).unwrap())
    }

    pub fn quota_prime_status(&self) -> Value {
        let st = self.quota_state.lock().expect("quota state").clone();
        let depth = self.quota_queue.lock().expect("quota queue").len();
        if let Some(obj) = st.as_object() {
            let mut out = obj.clone();
            out.insert("queue_depth".into(), json!(depth));
            Value::Object(out)
        } else {
            st
        }
    }

    pub fn quota_prime_enqueue(
        &self,
        tokens: Vec<String>,
        emails: &HashMap<String, String>,
    ) -> Value {
        let mut queue = self.quota_queue.lock().expect("quota queue");
        for token in tokens {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            let email = emails.get(t).cloned().unwrap_or_default();
            queue.push_back(QuotaPrimeJob {
                access_token: t.to_string(),
                email,
            });
        }
        let depth = queue.len();
        let mut st = self.quota_state.lock().expect("quota state");
        *st = json!({
            "running": depth > 0,
            "state": if depth > 0 { "running" } else { "idle" },
            "queue_depth": depth,
            "processed": st.get("processed").and_then(|v| v.as_u64()).unwrap_or(0),
            "succeeded": st.get("succeeded").and_then(|v| v.as_u64()).unwrap_or(0),
            "failed": st.get("failed").and_then(|v| v.as_u64()).unwrap_or(0),
            "source": "tnexus-account-ops",
        });
        json!({ "queued": depth, "source": "tnexus-account-ops" })
    }

    pub fn pop_quota_prime_job(&self) -> Option<QuotaPrimeJob> {
        self.quota_queue.lock().expect("quota queue").pop_front()
    }

    pub fn quota_prime_done_one(&self, ok: bool, err: Option<String>) {
        let mut st = self.quota_state.lock().expect("quota state");
        let processed = st.get("processed").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        let succeeded =
            st.get("succeeded").and_then(|v| v.as_u64()).unwrap_or(0) + if ok { 1 } else { 0 };
        let failed =
            st.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) + if ok { 0 } else { 1 };
        let depth = self.quota_queue.lock().expect("quota queue").len();
        *st = json!({
            "running": depth > 0,
            "state": if depth > 0 { "running" } else { "completed" },
            "queue_depth": depth,
            "processed": processed,
            "succeeded": succeeded,
            "failed": failed,
            "last_error": err,
            "source": "tnexus-account-ops",
        });
    }

    pub fn outlook_settings_snapshot(&self) -> Value {
        self.outlook_settings
            .lock()
            .expect("outlook settings")
            .clone()
    }

    pub fn outlook_status(&self) -> Value {
        let settings = self.outlook_settings.lock().expect("outlook settings");
        json!({
            "available": true,
            "settings": settings.clone(),
            "source": "tnexus-account-ops",
        })
    }

    pub fn outlook_settings(&self, patch: Map<String, Value>) -> Value {
        let mut settings = self.outlook_settings.lock().expect("outlook settings");
        if let Some(obj) = settings.as_object_mut() {
            for (k, v) in patch {
                obj.insert(k, v);
            }
        }
        settings.clone()
    }

    pub fn outlook_recover_one(&self, token: &str, account: Map<String, Value>) -> Value {
        let id = format!("prog-{}", uuid::Uuid::new_v4().simple());
        let row = json!({
            "progress_id": id,
            "state": "queued",
            "access_token": token,
            "account": Value::Object(account),
            "source": "tnexus-account-ops",
        });
        self.outlook_progress
            .lock()
            .expect("outlook progress")
            .insert(id.clone(), row.clone());
        row
    }

    pub fn outlook_pending_ids(&self) -> Vec<String> {
        let map = self.outlook_progress.lock().expect("outlook progress");
        map.iter()
            .filter(|(_, v)| {
                v.get("state")
                    .and_then(|s| s.as_str())
                    .map(|s| s == "queued")
                    .unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn outlook_job(&self, id: &str) -> Option<(String, Map<String, Value>)> {
        let map = self.outlook_progress.lock().expect("outlook progress");
        let row = map.get(id)?;
        let token = row
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let account = row
            .get("account")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        Some((token, account))
    }

    pub fn outlook_mark_running(&self, id: &str) {
        let mut map = self.outlook_progress.lock().expect("outlook progress");
        if let Some(row) = map.get_mut(id) {
            if let Some(obj) = row.as_object_mut() {
                obj.insert("state".into(), json!("running"));
            }
        }
    }

    pub fn outlook_mark_done(&self, id: &str, account: Map<String, Value>) {
        let mut map = self.outlook_progress.lock().expect("outlook progress");
        if let Some(row) = map.get_mut(id) {
            if let Some(obj) = row.as_object_mut() {
                obj.insert("state".into(), json!("done"));
                obj.insert("account".into(), Value::Object(account));
            }
        }
    }

    pub fn outlook_mark_failed(&self, id: &str, err: &str) {
        let mut map = self.outlook_progress.lock().expect("outlook progress");
        if let Some(row) = map.get_mut(id) {
            if let Some(obj) = row.as_object_mut() {
                obj.insert("state".into(), json!("failed"));
                obj.insert("error".into(), json!(err));
            }
        }
    }

    pub fn outlook_progress(&self, id: &str) -> Option<Value> {
        self.outlook_progress
            .lock()
            .expect("outlook progress")
            .get(id)
            .cloned()
    }

    pub fn proxy_runtime_get(&self) -> Result<Value> {
        let path = proxy_runtime_path();
        let runtime = if path.is_file() {
            let raw = fs::read_to_string(&path).context("read proxy runtime")?;
            serde_json::from_str(&raw).unwrap_or(json!({}))
        } else {
            json!({})
        };
        Ok(json!({
            "runtime": runtime,
            "status": { "ok": true },
            "source": "tnexus-account-ops",
        }))
    }

    pub fn proxy_runtime_save(&self, settings: Map<String, Value>) -> Result<Value> {
        let path = proxy_runtime_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("mkdir proxy runtime parent")?;
        }
        fs::write(&path, serde_json::to_string_pretty(&settings)?)
            .context("write proxy runtime")?;
        self.proxy_runtime_get()
    }

    pub async fn proxy_test(&self, http: &reqwest::Client, url: &str) -> Result<Value> {
        let target = if url.trim().is_empty() {
            "https://www.cloudflare.com/cdn-cgi/trace"
        } else {
            url.trim()
        };
        let resp = http.get(target).send().await.context("proxy test")?;
        Ok(json!({
            "result": {
                "ok": resp.status().is_success(),
                "status": resp.status().as_u16(),
                "url": target,
            },
            "source": "tnexus-account-ops",
        }))
    }

    pub fn webshare_status(&self) -> Value {
        let last = self.webshare_last_run.lock().expect("webshare last");
        let items = self
            .webshare_inventory_cache
            .lock()
            .expect("webshare cache")
            .len();
        json!({
            "running": false,
            "last_run_secs_ago": last.map(|t| t.elapsed().as_secs()).unwrap_or(0),
            "inventory_count": items,
            "source": "tnexus-account-ops",
        })
    }

    pub fn webshare_inventory(&self) -> Value {
        let items = self
            .webshare_inventory_cache
            .lock()
            .expect("webshare cache")
            .clone();
        json!({ "items": items, "source": "tnexus-account-ops" })
    }

    pub async fn webshare_run_once(&self, http: &reqwest::Client) -> Value {
        *self.webshare_last_run.lock().expect("webshare last") = Some(Instant::now());
        let api_key = std::env::var("WEBSHARE_API_KEY").unwrap_or_default();
        if api_key.trim().is_empty() {
            return json!({
                "ok": false,
                "error": "WEBSHARE_API_KEY not set",
                "scanned": 0,
                "source": "tnexus-account-ops",
            });
        }
        let url = "https://proxy.webshare.io/api/v2/proxy/list/?mode=direct&page=1&page_size=25";
        let resp = http
            .get(url)
            .header("Authorization", format!("Token {}", api_key.trim()))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let data: Value = r.json().await.unwrap_or(json!({}));
                let mut scanned = 0usize;
                let mut items = Vec::new();
                if let Some(rows) = data.get("results").and_then(|v| v.as_array()) {
                    for row in rows {
                        let proxy = row
                            .get("proxy_address")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let port = row.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                        if proxy.is_empty() || port == 0 {
                            continue;
                        }
                        scanned += 1;
                        let trace_url = "https://www.cloudflare.com/cdn-cgi/trace";
                        let cf_ok = http
                            .get(trace_url)
                            .send()
                            .await
                            .map(|resp| resp.status().is_success())
                            .unwrap_or(false);
                        items.push(json!({
                            "proxy": format!("{}:{}", proxy, port),
                            "cf_trace_ok": cf_ok,
                        }));
                    }
                }
                *self
                    .webshare_inventory_cache
                    .lock()
                    .expect("webshare cache") = items.clone();
                json!({
                    "ok": true,
                    "scanned": scanned,
                    "items": items.len(),
                    "source": "tnexus-account-ops",
                })
            }
            Ok(r) => json!({
                "ok": false,
                "status": r.status().as_u16(),
                "scanned": 0,
                "source": "tnexus-account-ops",
            }),
            Err(e) => json!({
                "ok": false,
                "error": e.to_string(),
                "scanned": 0,
                "source": "tnexus-account-ops",
            }),
        }
    }
}
