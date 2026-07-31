use uuid::Uuid;

use crate::requirements::ChatRequirements;

pub const PURE_HTTP_IMAGE_CLIENT_VERSION: &str = "prod-a194cd50d4416d3c0b47c740f206b12ce60f5887";
pub const PURE_HTTP_IMAGE_CLIENT_BUILD_NUMBER: &str = "6708908";

/// Sentinel headers from `services/protocol/chatgpt_web_request.py::build_sentinel_headers`.
pub fn build_sentinel_headers(requirements: &ChatRequirements) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "OpenAI-Sentinel-Chat-Requirements-Token".into(),
            requirements.token.clone(),
        ),
        (
            "OpenAI-Sentinel-Chat-Requirements-Prepare-Token".into(),
            requirements.token.clone(),
        ),
    ];
    if !requirements.proof_token.is_empty() {
        headers.push((
            "OpenAI-Sentinel-Proof-Token".into(),
            requirements.proof_token.clone(),
        ));
    }
    if !requirements.turnstile_token.is_empty() {
        headers.push((
            "OpenAI-Sentinel-Turnstile-Token".into(),
            requirements.turnstile_token.clone(),
        ));
    }
    if !requirements.so_token.is_empty() {
        headers.push((
            "OpenAI-Sentinel-SO-Token".into(),
            requirements.so_token.clone(),
        ));
    }
    headers
}

pub fn build_chat_headers(requirements: &ChatRequirements) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Accept".into(), "text/event-stream".into()),
        ("Content-Type".into(), "application/json".into()),
    ];
    headers.extend(build_sentinel_headers(requirements));
    headers
}

pub fn build_image_prepare_headers(requirements: &ChatRequirements) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("Accept".into(), "*/*".into()),
    ];
    headers.extend(build_sentinel_headers(requirements));
    headers
}

/// Image start headers (`build_image_start_headers`).
pub fn build_image_start_headers(
    requirements: &ChatRequirements,
    conduit_token: &str,
    spa_tool_path: bool,
) -> Vec<(String, String)> {
    if spa_tool_path {
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "text/event-stream".into()),
        ];
        headers.extend(build_sentinel_headers(requirements));
        return headers;
    }
    let mut headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("Accept".into(), "text/event-stream".into()),
    ];
    headers.extend(build_sentinel_headers(requirements));
    if !conduit_token.trim().is_empty() {
        headers.push(("X-Conduit-Token".into(), conduit_token.trim().to_string()));
    }
    headers.push(("X-Oai-Turn-Trace-Id".into(), Uuid::new_v4().to_string()));
    headers
}
