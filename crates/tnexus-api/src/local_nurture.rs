//! Local IP nurture presets/bindings (no gptimage proxy).

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

fn bindings_path() -> PathBuf {
    std::env::var("IP_NURTURE_BINDINGS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/ip_nurture_bindings.json"))
}

fn default_presets() -> Value {
    json!({
        "presets": [
            {
                "id": "balanced",
                "label": "均衡",
                "weights": [
                    [1,1,1,1,1,1,1,1,1,1,1,1],
                    [1,1,1,1,1,1,1,1,1,1,1,1],
                    [1,1,1,1,1,1,1,1,1,1,1,1],
                    [1,1,1,1,1,1,1,1,1,1,1,1],
                    [1,1,1,1,1,1,1,1,1,1,1,1],
                    [1,1,1,1,1,1,1,1,1,1,1,1],
                    [1,1,1,1,1,1,1,1,1,1,1,1]
                ]
            },
            {
                "id": "weekday_peak",
                "label": "工作日高峰",
                "weights": [
                    [0,0,0,1,2,2,2,2,2,1,0,0],
                    [0,0,0,1,2,2,2,2,2,1,0,0],
                    [0,0,0,1,2,2,2,2,2,1,0,0],
                    [0,0,0,1,2,2,2,2,2,1,0,0],
                    [0,0,0,1,2,2,2,2,2,1,0,0],
                    [0,0,1,1,1,1,1,1,1,1,1,0],
                    [0,0,1,1,1,1,1,1,1,1,1,0]
                ]
            }
        ],
        "source": "tnexus-local"
    })
}

#[derive(Clone, Default)]
pub struct LocalNurtureStore {
    bindings: Arc<RwLock<HashMap<String, Value>>>,
}

impl LocalNurtureStore {
    pub fn new() -> Self {
        let bindings = load_bindings_file().unwrap_or_default();
        Self {
            bindings: Arc::new(RwLock::new(bindings)),
        }
    }

    pub fn presets(&self) -> Value {
        default_presets()
    }

    pub async fn bindings(&self) -> Value {
        let guard = self.bindings.read().await;
        json!({ "bindings": guard.clone(), "source": "tnexus-local" })
    }

    pub async fn save_binding(
        &self,
        binding_key: &str,
        preset_id: &str,
        custom_matrix: Option<Value>,
    ) -> Result<Value> {
        let mut entry = json!({
            "binding_key": binding_key,
            "preset_id": preset_id,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        if let Some(matrix) = custom_matrix {
            entry["weights"] = matrix;
        }
        {
            let mut guard = self.bindings.write().await;
            guard.insert(binding_key.to_string(), entry.clone());
            save_bindings_file(&guard)?;
        }
        Ok(json!({ "ok": true, "binding": entry, "source": "tnexus-local" }))
    }
}

fn load_bindings_file() -> Result<HashMap<String, Value>> {
    let path = bindings_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let parsed: Value = serde_json::from_str(&raw).context("parse bindings")?;
    let mut out = HashMap::new();
    if let Some(obj) = parsed.get("bindings").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            out.insert(k.clone(), v.clone());
        }
    } else if let Some(obj) = parsed.as_object() {
        for (k, v) in obj {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(out)
}

fn save_bindings_file(map: &HashMap<String, Value>) -> Result<()> {
    let path = bindings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    let payload = json!({ "bindings": map });
    let raw = serde_json::to_string_pretty(&payload).context("serialize bindings")?;
    fs::write(&path, raw).with_context(|| format!("write {:?}", path))?;
    Ok(())
}

#[derive(Clone, Default)]
pub struct OutlookRecoveryStore {
    enabled: Arc<RwLock<bool>>,
}

impl OutlookRecoveryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn status(&self) -> Value {
        let enabled = *self.enabled.read().await;
        json!({
            "enabled": enabled,
            "available": false,
            "running": false,
            "last_run_at": null,
            "last_error": if enabled { json!("Outlook 自动恢复尚未在本环境实现") } else { Value::Null },
            "source": "tnexus-local",
            "message": "暂无（TNexus 独立实现中，不依赖生产 gptimage）",
        })
    }

    pub async fn set_enabled(&self, enabled: bool) -> Value {
        *self.enabled.write().await = enabled;
        self.status().await
    }
}
