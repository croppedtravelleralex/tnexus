//! Grok Web 养号：队列 + 按账号 chat（对齐 tnexus-account-ops nurture 语义）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use grok_domain::ChatRequest;
use grok_provider_web::ChatEngine;
use serde::Deserialize;
use serde_json::{json, Value};

const DEFAULT_PROMPT: &str = "Say hello in one short sentence.";

#[derive(Debug, Clone)]
pub struct NurtureJob {
    pub account_id: i64,
    pub email: Option<String>,
    pub prompt: String,
}

#[derive(Default)]
struct NurtureState {
    jobs: VecDeque<NurtureJob>,
    completed_today: u32,
    last_error: Option<String>,
}

/// 内存养号队列（单进程）。
pub struct GrokNurtureOps {
    enabled: AtomicBool,
    inner: Mutex<NurtureState>,
}

impl GrokNurtureOps {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            inner: Mutex::new(NurtureState::default()),
        }
    }

    pub fn status(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        json!({
            "running": self.enabled.load(Ordering::Relaxed),
            "queue_depth": inner.jobs.len(),
            "completed_in_day": inner.completed_today,
            "last_error": inner.last_error,
        })
    }

    pub fn set_enabled(&self, enabled: bool) -> Value {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.status()
    }

    pub fn enqueue(&self, account_ids: &[i64], prompt: Option<String>) -> Value {
        let prompt = prompt
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
        let mut inner = self.inner.lock().unwrap();
        for id in account_ids {
            inner.jobs.push_back(NurtureJob {
                account_id: *id,
                email: None,
                prompt: prompt.clone(),
            });
        }
        json!({ "enqueued": account_ids.len(), "queue_depth": inner.jobs.len() })
    }

    pub fn pop_job(&self) -> Option<NurtureJob> {
        self.inner.lock().unwrap().jobs.pop_front()
    }

    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.completed_today += 1;
        inner.last_error = None;
    }

    pub fn record_error(&self, err: String) {
        self.inner.lock().unwrap().last_error = Some(err);
    }

    pub fn is_running(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// 养号执行 + HTTP 路由。
pub struct GrokNurtureService {
    pub ops: Arc<GrokNurtureOps>,
    engine: Arc<ChatEngine>,
}

impl GrokNurtureService {
    pub fn new(ops: Arc<GrokNurtureOps>, engine: Arc<ChatEngine>) -> Self {
        Self { ops, engine }
    }

    pub async fn process_job(&self, job: &NurtureJob) -> Result<Value, String> {
        let req = ChatRequest {
            prompt: format!("[user]\n{}", job.prompt),
            images: vec![],
            ocr: false,
            system_prompt: None,
            request_id: format!("nurture-{}-{}", job.account_id, chrono::Utc::now().timestamp_millis()),
        };
        match self
            .engine
            .chat_for_account(job.account_id, &req)
            .await
        {
            Ok(outcome) => {
                self.ops.record_success();
                record_usage_event(job.account_id, job.email.as_deref());
                Ok(json!({
                    "ok": true,
                    "account_id": outcome.account_id,
                    "bytes": outcome.text.len(),
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                self.ops.record_error(msg.clone());
                Err(msg)
            }
        }
    }

    pub async fn process_one(&self, account_id: i64, prompt: Option<String>) -> Result<Value, String> {
        let job = NurtureJob {
            account_id,
            email: None,
            prompt: prompt
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_PROMPT.to_string()),
        };
        self.process_job(&job).await
    }

    pub async fn handle(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Option<(u16, Value)> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        match segments.as_slice() {
            ["admin", "nurture", "status"] if method.eq_ignore_ascii_case("GET") => {
                Some((200, self.ops.status()))
            }
            ["admin", "nurture", "enable"] if method.eq_ignore_ascii_case("POST") => {
                let enabled = parse_json::<EnableBody>(body)
                    .map(|b| b.enabled)
                    .unwrap_or(true);
                Some((200, self.ops.set_enabled(enabled)))
            }
            ["admin", "nurture", "enqueue"] if method.eq_ignore_ascii_case("POST") => {
                let body = parse_json::<EnqueueBody>(body)?;
                let ids = body.account_ids.unwrap_or_default();
                Some((200, self.ops.enqueue(&ids, body.prompt)))
            }
            ["admin", "nurture", "process-one"] if method.eq_ignore_ascii_case("POST") => {
                let body = parse_json::<ProcessOneBody>(body)?;
                let id = body.account_id?;
                match self.process_one(id, body.prompt).await {
                    Ok(v) => Some((200, v)),
                    Err(e) => Some((502, json!({ "ok": false, "error": e }))),
                }
            }
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct EnableBody {
    enabled: bool,
}

#[derive(Deserialize)]
struct EnqueueBody {
    account_ids: Option<Vec<i64>>,
    prompt: Option<String>,
}

#[derive(Deserialize)]
struct ProcessOneBody {
    account_id: Option<i64>,
    prompt: Option<String>,
}

fn parse_json<T: serde::de::DeserializeOwned>(body: Option<&str>) -> Option<T> {
    body.and_then(|raw| serde_json::from_str(raw).ok())
}

fn record_usage_event(account_id: i64, email: Option<&str>) {
    let path = std::env::var("USAGE_EVENTS_FILE")
        .unwrap_or_else(|_| "/data/pool/usage_events.ndjson".into());
    let email = email.unwrap_or("unknown@grok.local");
    let line = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "email": email,
        "metric": "grok_dialogues_nurture",
        "account_id": account_id,
        "binding": format!("grok:{account_id}"),
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(f, "{}", line);
    }
}
