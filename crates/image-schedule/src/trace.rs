//! Schedule trace — gptimage `image_schedule_trace` subset.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceEventKind {
    GateAdmission,
    AccountPick,
    SlotAcquire,
    UpstreamStart,
    UpstreamPoll,
    UpstreamDone,
    ReadyBufferAdmit,
    ReturnWindowAcquire,
    TaskQueued,
    TaskRunning,
    TaskDone,
    TaskFailed,
    CooldownApplied,
}

#[derive(Debug)]
pub struct ImageScheduleTrace {
    pub task_id: String,
    pub email: String,
    started: Instant,
    events: Vec<Value>,
}

impl ImageScheduleTrace {
    pub fn new(task_id: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            email: email.into(),
            started: Instant::now(),
            events: Vec::new(),
        }
    }

    pub fn emit(&mut self, kind: TraceEventKind, ok: bool, extra: Option<Value>) {
        let mut event = json!({
            "kind": kind,
            "ok": ok,
            "ms": self.started.elapsed().as_millis(),
        });
        if let Some(obj) = event.as_object_mut() {
            if let Some(ex) = extra {
                obj.insert("extra".into(), ex);
            }
        }
        self.events.push(event);
    }

    pub fn to_json(&self) -> Value {
        json!({
            "task_id": self.task_id,
            "email": self.email,
            "events": self.events,
            "wall_ms": self.started.elapsed().as_millis(),
        })
    }

    pub fn phases_ms(&self) -> BTreeMap<String, u128> {
        let mut out = BTreeMap::new();
        for ev in &self.events {
            let kind = ev
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let ms = ev.get("ms").and_then(|v| v.as_u64()).unwrap_or(0) as u128;
            out.insert(kind.to_string(), ms);
        }
        out
    }
}
