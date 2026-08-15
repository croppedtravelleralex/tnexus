//! Simple backend capability surface for the web UI.

use crate::config::DataPlane;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;

async fn helper_liveness(st: &AppState) -> bool {
    if st.data_plane == DataPlane::Upstream && st.image_enabled {
        return true;
    }
    st.helper.health().await.is_ok()
}

pub async fn capabilities(State(st): State<Arc<AppState>>) -> Json<Value> {
    let helper_ok = helper_liveness(&st).await;
    let upstream_mode = st.data_plane == DataPlane::Upstream;
    let image_edits_enabled = st.image_enabled && upstream_mode;
    let mut deferred = Vec::new();
    if !image_edits_enabled {
        deferred.push("image_edits");
    }
    if !upstream_mode {
        deferred.push("estuary_download");
    }
    if !st.image_enabled {
        deferred.insert(0, "image_generations");
    }
    Json(json!({
        "ok": true,
        "service": "gptimage-gateway-rs",
        "wave": "local-full",
        "data_plane": st.data_plane.as_str(),
        "helper_ok": helper_ok,
        "features": {
            "auth": !st.auth.config().auth_disabled(),
            "auth_mode": st.auth.config().mode.as_str(),
            "chat": true,
            "chat_stream": true,
            "stream_chat": upstream_mode,
            "poll_tasks": upstream_mode,
            "models": true,
            "quota_probe": !upstream_mode,
            "account_candidates": !upstream_mode,
            "image_generations": st.image_generation_allowed(),
            "image_edits": image_edits_enabled && st.image_generation_allowed(),
            "image_tasks": st.image_generation_allowed(),
            "image_async": st.image_generation_allowed(),
            "image_runtime_hot_reload": st.image_enabled,
            "estuary_download": upstream_mode,
            "static_ui": st.static_dir.is_some(),
        },
        "deferred": deferred,
        "notes": {
            "image": if st.image_enabled {
                if upstream_mode {
                    "IMAGE_ENABLED=1 — generations routed to upstream (Rust data plane)"
                } else {
                    "IMAGE_ENABLED=1 — generations routed to helper"
                }
            } else {
                "Set IMAGE_ENABLED=1 to enable /v1/images/generations"
            },
            "data_plane": format!(
                "DATA_PLANE={} (default upstream; set helper for legacy bridge)",
                st.data_plane.as_str()
            ),
            "local": "bash scripts/local_bringup_wsl.sh (LOCAL_MODE=full default)"
        }
    }))
}

/// Full runtime detail. Admin-only: exposes pool identities and tuning that
/// the unauthenticated `/health` deliberately omits.
pub async fn admin_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let accounts = st.accounts.lock().await;
    let emails: Vec<&str> = accounts.keys().map(String::as_str).collect();
    Json(json!({
        "ok": true,
        "listen": st.listen,
        "pin_email": st.pin.email,
        "accounts": emails,
        "image_global_concurrency": st.image_global_concurrency,
        "effective_global_concurrency": st.effective_global_concurrency(),
        "image_sem_available": st.image_sem.available_permits(),
        "min_image_quota": st.min_image_quota,
        "image_enabled": st.image_enabled,
        "image_generation_allowed": st.image_generation_allowed(),
        "image_runtime_paused": st.image_runtime.is_paused(),
        "image_pipeline": st.pipeline_snapshot(),
        "auth_disabled": st.auth.config().auth_disabled(),
        "auth_mode": st.auth.config().mode.as_str(),
        "static_ui": st.static_dir.is_some(),
    }))
}
