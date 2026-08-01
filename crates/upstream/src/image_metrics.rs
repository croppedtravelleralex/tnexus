use serde::Serialize;

/// Per-upstream-image wall-clock and bandwidth breakdown (milliseconds + bytes).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImageRunMetrics {
    pub bootstrap_ms: u64,
    pub requirements_ms: u64,
    pub prepare_ms: u64,
    pub sse_ms: u64,
    pub resolve_url_ms: u64,
    pub poll_tasks_ms: u64,
    pub download_ms: u64,
    pub wall_ms: u64,
    pub sse_bytes_in: u64,
    pub image_bytes: u64,
    pub sse_events: u32,
}

impl ImageRunMetrics {
    pub fn to_timings_json(&self) -> serde_json::Value {
        serde_json::json!({
            "bootstrap_ms": self.bootstrap_ms,
            "requirements_ms": self.requirements_ms,
            "prepare_ms": self.prepare_ms,
            "sse_ms": self.sse_ms,
            "resolve_url_ms": self.resolve_url_ms,
            "poll_tasks_ms": self.poll_tasks_ms,
            "download_ms": self.download_ms,
            "wall_ms": self.wall_ms,
        })
    }

    pub fn to_bytes_json(&self) -> serde_json::Value {
        serde_json::json!({
            "sse_in": self.sse_bytes_in,
            "image_download": self.image_bytes,
        })
    }
}
