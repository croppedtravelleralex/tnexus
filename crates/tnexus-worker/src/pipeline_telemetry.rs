//! NDJSON pipeline telemetry (worker-side job aggregation).

use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;

fn events_path() -> PathBuf {
    std::env::var("PIPELINE_EVENTS_FILE")
        .or_else(|_| std::env::var("USAGE_EVENTS_FILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/pool/pipeline_events.ndjson"))
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineEvent {
    pub ts: String,
    pub kind: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_index: Option<i32>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_before: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_after: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timings_ms: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

pub fn append_event(event: &PipelineEvent) {
    let path = events_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(line) = serde_json::to_string(event) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
