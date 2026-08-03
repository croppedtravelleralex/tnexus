//! Append gptimage-compatible usage events (shared `USAGE_EVENTS_FILE`).

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static APPEND_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize)]
pub struct UsageEvent {
    pub ts: String,
    pub email: String,
    pub binding: String,
    pub metric: String,
    pub ok: bool,
}

fn events_path() -> PathBuf {
    std::env::var("USAGE_EVENTS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/data/pool/usage_events.ndjson"))
}

pub fn record_dialogues_nurture(email: &str, binding: &str) -> Result<()> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(());
    }
    let binding = if binding.trim().is_empty() {
        "default".to_string()
    } else {
        binding.trim().to_string()
    };
    let event = UsageEvent {
        ts: Utc::now().to_rfc3339(),
        email,
        binding,
        metric: "dialogues_nurture".into(),
        ok: true,
    };
    let path = events_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    let line = serde_json::to_string(&event).context("serialize usage event")?;
    let _guard = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open usage events {:?}", path))?;
    writeln!(file, "{line}").context("append usage event")?;
    Ok(())
}
