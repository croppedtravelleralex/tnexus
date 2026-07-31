//! Local refresh-all slow job (no gptimage proxy).

use crate::account_ops::{self, ProgressStore};
use crate::state::AppState;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct RefreshAllStore {
    inner: Arc<RwLock<Option<RefreshAllJob>>>,
}

struct RefreshAllJob {
    job_id: String,
    cancel: Arc<AtomicBool>,
    state: Arc<RwLock<Value>>,
}

impl RefreshAllStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn status(&self) -> Value {
        let guard = self.inner.read().await;
        if let Some(job) = guard.as_ref() {
            return job.state.read().await.clone();
        }
        json!({
            "state": "idle",
            "running": false,
            "source": "tnexus-local",
            "total": 0,
            "processed": 0,
            "refreshed": 0,
            "available": 0,
            "became_available": 0,
            "failed": 0,
            "skipped": 0,
        })
    }

    pub async fn start(&self, state: Arc<AppState>, options: Value) -> Result<Value, String> {
        {
            let guard = self.inner.read().await;
            if let Some(job) = guard.as_ref() {
                let st = job.state.read().await;
                if st.get("running").and_then(|v| v.as_bool()).unwrap_or(false) {
                    return Err("refresh-all 已在运行".into());
                }
            }
        }

        let list = state.accounts.list(0, usize::MAX).await;
        let tokens: Vec<String> = list
            .get("items")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|row| row.get("access_token").and_then(|v| v.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let concurrency = options
            .get("concurrency")
            .and_then(|v| v.as_u64())
            .unwrap_or(4)
            .clamp(1, 16) as usize;
        let delay_ms = options
            .get("delay_sec")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.2)
            .max(0.0);
        let delay_ms = (delay_ms * 1000.0) as u64;

        let job_id = Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let job_state = Arc::new(RwLock::new(json!({
            "job_id": job_id,
            "state": "running",
            "running": true,
            "source": "tnexus-local",
            "started_at": chrono::Utc::now().to_rfc3339(),
            "total": tokens.len(),
            "processed": 0,
            "refreshed": 0,
            "available": 0,
            "became_available": 0,
            "failed": 0,
            "skipped": 0,
            "recent": [],
            "options": options,
        })));

        {
            let mut guard = self.inner.write().await;
            *guard = Some(RefreshAllJob {
                job_id: job_id.clone(),
                cancel: cancel.clone(),
                state: job_state.clone(),
            });
        }

        let store = self.clone();
        tokio::spawn(async move {
            run_refresh_all(state, tokens, concurrency, delay_ms, cancel, job_state.clone()).await;
            let mut st = job_state.write().await;
            if st.get("state").and_then(|v| v.as_str()) == Some("running") {
                st["state"] = json!("completed");
                st["running"] = json!(false);
                st["finished_at"] = json!(chrono::Utc::now().to_rfc3339());
            }
            let _ = store;
        });

        Ok(json!({ "job_id": job_id, "started": true, "source": "tnexus-local" }))
    }

    pub async fn stop(&self) -> Value {
        let guard = self.inner.read().await;
        if let Some(job) = guard.as_ref() {
            job.cancel.store(true, Ordering::SeqCst);
            let mut st = job.state.write().await;
            st["state"] = json!("stopping");
            return st.clone();
        }
        json!({ "state": "idle", "running": false, "source": "tnexus-local" })
    }
}

async fn run_refresh_all(
    state: Arc<AppState>,
    tokens: Vec<String>,
    concurrency: usize,
    delay_ms: u64,
    cancel: Arc<AtomicBool>,
    job_state: Arc<RwLock<Value>>,
) {
    let progress = ProgressStore::new();
    let mut refreshed = 0usize;
    let mut failed = 0usize;
    let mut processed = 0usize;
    let mut recent: Vec<Value> = Vec::new();

    for chunk in tokens.chunks(concurrency) {
        if cancel.load(Ordering::SeqCst) {
            let mut st = job_state.write().await;
            st["state"] = json!("stopped");
            st["running"] = json!(false);
            return;
        }
        let mut handles = Vec::new();
        for token in chunk {
            let st = state.clone();
            let token = token.clone();
            handles.push(tokio::spawn(async move {
                let account = st.accounts.export_account_for_token(&token).await;
                let Some(account) = account else {
                    return Err("account not found".to_string());
                };
                account_ops::refresh_one(&st, account).await
            }));
        }
        for handle in handles {
            processed += 1;
            match handle.await {
                Ok(Ok(updated)) => {
                    let status = updated
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("正常");
                    let quota = updated.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
                    if state.accounts.merge_remote_items(&[updated.clone()]).await.is_ok() {
                        refreshed += 1;
                    }
                    recent.push(json!({
                        "token": token_preview(&updated),
                        "status": status,
                        "quota": quota,
                    }));
                    if recent.len() > 20 {
                        recent.remove(0);
                    }
                }
                _ => failed += 1,
            }
            let mut st = job_state.write().await;
            st["processed"] = json!(processed);
            st["refreshed"] = json!(refreshed);
            st["failed"] = json!(failed);
            st["recent"] = json!(recent);
        }
        if delay_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        }
        let _ = progress;
    }
}

fn token_preview(row: &Value) -> String {
    row.get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(8).collect())
        .unwrap_or_default()
}
