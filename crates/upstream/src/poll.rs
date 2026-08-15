//! Poll `/backend-api/tasks` and conversation documents for async image generation results.

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tracing::{info, warn};
use wreq::Client;

use crate::requirements::{RequirementsClient, BASE_URL};
use crate::sse::{extract_conversation_ids, file_service_re, real_image_file_re, sediment_re};

// Align with gptimage `image_generation_poll_timeout_secs` (300s on Panda).
const DEFAULT_POLL_TIMEOUT_SECS: u64 = 300;
const DEFAULT_POLL_INITIAL_WAIT_SECS: f64 = 5.0;
const DEFAULT_POLL_INTERVAL_SECS: f64 = 3.0;
const DEFAULT_POLL_SETTLE_SECS: f64 = 2.0;
const DEFAULT_POLL_TASKS_EVERY_N: u32 = 2;
const GET_BUDGET_OVERSHOOT_FACTOR: u32 = 2;
const GET_BUDGET_SLACK_ATTEMPTS: u32 = 8;
const SKIP_FILE_IDS: &[&str] = &["file_upload"];

/// `/backend-api/tasks` 的终态取值。任务全部终结却没有图片 id，说明这次生成已经
/// 结束且不会再产出，继续轮询只是在空耗整个 wall budget。
const TERMINAL_TASK_STATUSES: &[&str] = &[
    "completed",
    "failed",
    "cancelled",
    "canceled",
    "expired",
    "error",
];

/// 连续多少轮观察到「任务全终结且无图」才判定失败，避开任务列表短暂领先于
/// 会话文档的竞态。
const TERMINAL_TASK_CONFIRMATIONS: u32 = 2;

/// 连续多少轮 `/backend-api/tasks` 查询失败后，若会话已有 image_gen 活动却始终无图，
/// 提前放弃。旧逻辑用 `if let Ok(tasks)` 丢掉 Err，任务查询失败时既不推进终态判定、
/// 也不计入任何 streak，轮询会空转到 wall budget（线上见过 420s）。
const DEFAULT_TASK_QUERY_FAIL_MAX: u32 = 8;
///
/// 线上实测：SSE 走完 22 个事件即结束且无 file_id，随后 94 轮全是 `no ids yet`，
/// 直到 420s 用尽。那种会话根本没有创建异步任务，因此 `/backend-api/tasks` 返回空、
/// `tasks_all_terminal` 判为「无法判定」，早停永远不触发。
///
/// 该判定只在 image_gen 节点**始终不存在**时生效：工具一旦被调用（哪怕生成很慢）
/// 就恒为 true，从而不会误杀慢生成。对照一次成功请求在 attempt=8（约 55s）拿到图。
const DEFAULT_NO_ACTIVITY_MAX_ATTEMPTS: u32 = 20;

const CONTENT_POLICY_KEYWORDS: &[&str] = &[
    "内容政策",
    "防护限制",
    "违反",
    "moderation",
    "policy",
    "blocked",
    "不能生成",
    "无法生成",
    "不能帮助",
    "无法帮助",
    "裸体",
    "裸露",
    "色情",
    "性内容",
    "未成年",
    "抱歉，我不能",
];

/// Wall-clock poll budget for post-SSE image resolution (aligned with gptimage `image_task_queue`).
#[derive(Debug, Clone)]
pub struct ImagePollConfig {
    pub timeout: Duration,
    pub initial_wait: Duration,
    pub interval: Duration,
    pub settle: Duration,
    pub check_before_hit: bool,
    pub max_tasks_gets: u32,
    pub tasks_every_n_attempts: u32,
}

impl ImagePollConfig {
    pub fn from_env() -> Self {
        let timeout_secs = env_u64(
            "UPSTREAM_IMAGE_POLL_TIMEOUT_SECS",
            DEFAULT_POLL_TIMEOUT_SECS,
        );
        let initial_secs = env_f64(
            "UPSTREAM_IMAGE_POLL_INITIAL_WAIT_SECS",
            DEFAULT_POLL_INITIAL_WAIT_SECS,
        );
        let interval_secs = env_f64(
            "UPSTREAM_IMAGE_POLL_INTERVAL_SECS",
            DEFAULT_POLL_INTERVAL_SECS,
        );
        let interval = Duration::from_secs_f64(interval_secs.max(0.5));
        let timeout = Duration::from_secs(timeout_secs.max(30));
        let tasks_every_n = env_u32(
            "UPSTREAM_IMAGE_POLL_TASKS_EVERY_N",
            DEFAULT_POLL_TASKS_EVERY_N,
        )
        .max(1);
        let max_tasks_gets = env_u32("UPSTREAM_IMAGE_POLL_MAX_TASKS_GETS", 0);
        let max_tasks_gets = if max_tasks_gets == 0 {
            derive_max_tasks_gets(timeout, interval, tasks_every_n)
        } else {
            max_tasks_gets
        };
        let settle_secs =
            env_f64("UPSTREAM_IMAGE_POLL_SETTLE_SECS", DEFAULT_POLL_SETTLE_SECS).max(0.0);
        Self {
            timeout,
            initial_wait: Duration::from_secs_f64(initial_secs.max(0.0)),
            interval,
            settle: Duration::from_secs_f64(settle_secs),
            check_before_hit: env_bool("UPSTREAM_IMAGE_POLL_CHECK_BEFORE_HIT", true),
            max_tasks_gets,
            tasks_every_n_attempts: tasks_every_n,
        }
    }
}

/// Headroom for tasks GETs across the full wall budget (gptimage `image_poll_budget`).
fn derive_max_tasks_gets(timeout: Duration, interval: Duration, tasks_every_n: u32) -> u32 {
    let interval_secs = interval.as_secs_f64().max(0.5);
    let wall_secs = timeout.as_secs_f64().max(0.1);
    let nominal_attempts = (wall_secs / interval_secs).ceil() as u32;
    let cap = nominal_attempts
        .saturating_mul(GET_BUDGET_OVERSHOOT_FACTOR)
        .saturating_add(GET_BUDGET_SLACK_ATTEMPTS);
    cap.saturating_div(tasks_every_n.max(1)).max(8)
}

#[derive(Debug, Clone, Default)]
pub struct ImagePollOutcome {
    pub file_ids: Vec<String>,
    pub sediment_ids: Vec<String>,
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key).ok().map(|s| s.to_ascii_lowercase()) {
        Some(v) if matches!(v.as_str(), "1" | "true" | "yes" | "on") => true,
        Some(v) if matches!(v.as_str(), "0" | "false" | "no" | "off") => false,
        Some(_) => default,
        None => default,
    }
}

fn with_accept_json(mut headers: Vec<(String, String)>) -> Vec<(String, String)> {
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Accept"))
    {
        headers.push(("Accept".into(), "application/json".into()));
    }
    headers
}

fn add_unique_file_ids(file_ids: &mut Vec<String>, candidates: &[String]) {
    for candidate in candidates {
        if candidate.is_empty()
            || SKIP_FILE_IDS.contains(&candidate.as_str())
            || file_ids.contains(candidate)
        {
            continue;
        }
        file_ids.push(candidate.clone());
    }
}

fn add_unique_sediment_ids(sediment_ids: &mut Vec<String>, candidates: &[String]) {
    for candidate in candidates {
        if candidate.is_empty() || sediment_ids.contains(candidate) {
            continue;
        }
        sediment_ids.push(candidate.clone());
    }
}

fn task_matches_conversation(task: &Value, conversation_id: &str) -> bool {
    if conversation_id.is_empty() {
        return true;
    }
    task.get("conversation_id")
        .and_then(|v| v.as_str())
        .map(|c| c == conversation_id)
        .unwrap_or(false)
        || task
            .get("original_conversation_id")
            .and_then(|v| v.as_str())
            .map(|c| c == conversation_id)
            .unwrap_or(false)
}

fn message_is_structured_image_error(message: &serde_json::Map<String, Value>) -> bool {
    let metadata = message.get("metadata").and_then(|v| v.as_object());
    let content = message.get("content").and_then(|v| v.as_object());
    let author = message.get("author").and_then(|v| v.as_object());
    let is_error = metadata
        .and_then(|m| m.get("is_error"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_text_only = content
        .and_then(|c| c.get("content_type"))
        .and_then(|v| v.as_str())
        == Some("text");
    let is_assistant_role =
        author.and_then(|a| a.get("role")).and_then(|v| v.as_str()) == Some("assistant");
    is_error && is_text_only && is_assistant_role
}

fn structured_image_error_text(message: &serde_json::Map<String, Value>) -> Option<String> {
    if !message_is_structured_image_error(message) {
        return None;
    }
    let content = message.get("content")?;
    let parts = content.get("parts")?.as_array()?;
    let text: String = parts
        .iter()
        .filter_map(|p| p.as_str())
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() {
        Some("upstream image generation failed".into())
    } else {
        Some(text)
    }
}

fn task_is_structured_error(task: &Value) -> bool {
    let img_msg = task.get("image_gen_message").and_then(|v| v.as_object());
    if img_msg.is_none() {
        return false;
    }
    let img_msg = img_msg.unwrap();
    message_is_structured_image_error(img_msg)
}

/// Detect terminal upstream image failures in a conversation document.
pub fn detect_image_gen_failure_from_conversation(data: &Value) -> Option<String> {
    let mapping = data.get("mapping")?.as_object()?;
    for node in mapping.values() {
        let message = node.get("message")?.as_object()?;
        if let Some(text) = structured_image_error_text(message) {
            return Some(text);
        }
    }
    None
}

fn last_task_error_from_tasks(tasks: &[Value]) -> Option<String> {
    let mut last = None;
    for task in tasks {
        if !task_is_structured_error(task) {
            continue;
        }
        let img_msg = task.get("image_gen_message")?.as_object()?;
        if let Some(text) = structured_image_error_text(img_msg) {
            last = Some(text);
        }
    }
    last
}

fn message_text(message: &serde_json::Map<String, Value>) -> String {
    let content = message.get("content").unwrap_or(&Value::Null);
    let mut parts = Vec::new();
    if let Some(content_obj) = content.as_object() {
        if let Some(msg_parts) = content_obj.get("parts").and_then(|v| v.as_array()) {
            for part in msg_parts {
                if let Some(text) = part.as_str() {
                    if !text.trim().is_empty() {
                        parts.push(text.trim());
                    }
                }
            }
        }
        if let Some(text) = content_obj.get("text").and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                parts.push(text.trim());
            }
        }
    } else if let Some(text) = content.as_str() {
        if !text.trim().is_empty() {
            parts.push(text.trim());
        }
    }
    parts.join("\n")
}

fn is_content_policy_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    CONTENT_POLICY_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(&keyword.to_ascii_lowercase()))
}

/// Classify assistant/task text into a terminal upstream error code.
pub fn classify_terminal_upstream_text(text: &str) -> Option<(String, String)> {
    let clipped = text.trim();
    if clipped.is_empty() {
        return None;
    }
    let clipped = clipped.chars().take(500).collect::<String>();
    if is_content_policy_error(&clipped) {
        return Some(("content_policy_violation".into(), clipped));
    }
    let lower = clipped.to_ascii_lowercase();
    if lower.contains("image creation limit")
        || lower.contains("instant limit")
        || lower.contains("limit resets")
    {
        return Some(("image_instant_limit".into(), clipped));
    }
    if clipped.contains("请上传")
        || clipped.contains("参考图")
        || clipped.contains("请先上传")
        || lower.contains("please upload")
        || lower.contains("reference image")
        || lower.contains("no reference image")
    {
        return Some(("missing_reference_image".into(), clipped));
    }
    None
}

pub fn conversation_has_image_gen_activity(data: &Value) -> bool {
    let mapping = match data.get("mapping").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return false,
    };
    for node in mapping.values() {
        let metadata = node.get("message").and_then(|v| v.get("metadata"));
        if metadata
            .and_then(|m| m.get("async_task_type"))
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("image_gen"))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

pub fn find_terminal_upstream_block_in_conversation(data: &Value) -> Option<(String, String)> {
    let title = data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !title.is_empty() && title.to_ascii_lowercase().contains("image creation limit") {
        return Some((
            "image_instant_limit".into(),
            title.chars().take(500).collect(),
        ));
    }
    let mapping = data.get("mapping")?.as_object()?;
    for node in mapping.values() {
        let message = node.get("message")?.as_object()?;
        let role = message
            .get("author")
            .and_then(|a| a.get("role"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if role != "assistant" && role != "tool" {
            continue;
        }
        let text = message_text(message);
        if let Some(hit) = classify_terminal_upstream_text(&text) {
            return Some(hit);
        }
    }
    None
}

fn walk_image_reference_ids(
    value: &Value,
    file_ids: &mut Vec<String>,
    sediment_ids: &mut Vec<String>,
) {
    match value {
        Value::String(s) => {
            for cap in file_service_re().captures_iter(s) {
                add_unique_file_ids(file_ids, &[cap[1].to_string()]);
            }
            for m in real_image_file_re().find_iter(s) {
                add_unique_file_ids(file_ids, &[m.as_str().to_string()]);
            }
            for cap in sediment_re().captures_iter(s) {
                add_unique_sediment_ids(sediment_ids, &[cap[1].to_string()]);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_image_reference_ids(item, file_ids, sediment_ids);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                walk_image_reference_ids(item, file_ids, sediment_ids);
            }
        }
        _ => {}
    }
}

fn has_image_asset_pointer(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if map.get("content_type").and_then(|v| v.as_str()) == Some("image_asset_pointer") {
                return true;
            }
            let asset_pointer = map
                .get("asset_pointer")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if asset_pointer.starts_with("file-service://")
                || asset_pointer.starts_with("sediment://")
            {
                return true;
            }
            map.values().any(has_image_asset_pointer)
        }
        Value::Array(items) => items.iter().any(has_image_asset_pointer),
        _ => false,
    }
}

/// Extract image file/sediment ids from a conversation document (`mapping` tree).
pub fn extract_image_ids_from_conversation(data: &Value) -> (Vec<String>, Vec<String>) {
    let mut file_ids = Vec::new();
    let mut sediment_ids = Vec::new();
    let mapping = data.get("mapping").and_then(|v| v.as_object());
    if mapping.is_none() {
        return (file_ids, sediment_ids);
    }
    for node in mapping.unwrap().values() {
        let message = node.get("message").and_then(|v| v.as_object());
        if message.is_none() {
            continue;
        }
        let message = message.unwrap();
        let author = message.get("author").and_then(|v| v.as_object());
        let role = author
            .and_then(|a| a.get("role"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if role != "tool" && role != "assistant" {
            continue;
        }
        let metadata = message.get("metadata").unwrap_or(&Value::Null);
        let content = message.get("content").unwrap_or(&Value::Null);
        let is_image_gen =
            metadata.get("async_task_type").and_then(|v| v.as_str()) == Some("image_gen");
        let has_asset = has_image_asset_pointer(content) || has_image_asset_pointer(metadata);
        if role == "assistant" && !is_image_gen && !has_asset {
            continue;
        }
        let mut msg_file_ids = Vec::new();
        let mut msg_sediment_ids = Vec::new();
        walk_image_reference_ids(content, &mut msg_file_ids, &mut msg_sediment_ids);
        walk_image_reference_ids(metadata, &mut msg_file_ids, &mut msg_sediment_ids);
        if !is_image_gen && !has_asset && msg_file_ids.is_empty() && msg_sediment_ids.is_empty() {
            continue;
        }
        add_unique_file_ids(&mut file_ids, &msg_file_ids);
        add_unique_sediment_ids(&mut sediment_ids, &msg_sediment_ids);
    }
    (file_ids, sediment_ids)
}

/// GET `/backend-api/conversation/{id}`.
pub async fn get_conversation<F>(
    client: &Client,
    headers_fn: F,
    conversation_id: &str,
) -> Result<Value>
where
    F: Fn(&str) -> Vec<(String, String)>,
{
    let path = format!("/backend-api/conversation/{conversation_id}");
    let url = format!("{BASE_URL}{path}");
    let headers = with_accept_json(headers_fn(&path));
    let resp = RequirementsClient::apply_headers(client.get(url), &headers)
        .send()
        .await
        .context("GET /backend-api/conversation")?;
    let status = resp.status();
    let text = resp.text().await.context("conversation body")?;
    if !status.is_success() {
        bail!(
            "conversation HTTP {status}: {}",
            &text[..text.len().min(240)]
        );
    }
    serde_json::from_str(&text).context("parse conversation json")
}

/// GET `/backend-api/tasks?conversation_id=...` and return the task list.
pub async fn query_tasks<F>(
    client: &Client,
    headers_fn: F,
    conversation_id: &str,
) -> Result<Vec<Value>>
where
    F: Fn(&str) -> Vec<(String, String)>,
{
    let path = "/backend-api/tasks";
    let url = if conversation_id.is_empty() {
        format!("{BASE_URL}{path}")
    } else {
        format!("{BASE_URL}{path}?conversation_id={conversation_id}")
    };
    let headers = with_accept_json(headers_fn(path));
    let resp = RequirementsClient::apply_headers(client.get(url), &headers)
        .send()
        .await
        .context("GET /backend-api/tasks")?;
    let status = resp.status();
    let text = resp.text().await.context("tasks body")?;
    if !status.is_success() {
        bail!("tasks HTTP {status}: {}", &text[..text.len().min(240)]);
    }
    let data: Value = serde_json::from_str(&text).context("parse tasks json")?;
    let tasks = data
        .get("tasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if conversation_id.is_empty() {
        return Ok(tasks);
    }
    Ok(tasks
        .into_iter()
        .filter(|task| task_matches_conversation(task, conversation_id))
        .collect())
}

/// 任务是否全部进入终态。`None` = 无任务或上游未给 status，不足以判定。
pub fn tasks_all_terminal(tasks: &[Value]) -> Option<bool> {
    if tasks.is_empty() {
        return None;
    }
    let mut all_terminal = true;
    for task in tasks {
        let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status.is_empty() {
            return None;
        }
        if !TERMINAL_TASK_STATUSES
            .iter()
            .any(|s| status.eq_ignore_ascii_case(s))
        {
            all_terminal = false;
        }
    }
    Some(all_terminal)
}

/// 任务查询连续失败时是否该提前放弃。
///
/// 只在「已有 image_gen 活动、却始终拿不到图」时生效：此时终态判定依赖 tasks，
/// 查询失败会被旧的 `if let Ok` 吞掉，轮询会空转到 timeout。
pub fn should_stop_after_task_query_failures(
    consecutive_failures: u32,
    fail_max: u32,
    no_ids: bool,
    has_activity: bool,
) -> bool {
    no_ids && has_activity && consecutive_failures >= fail_max
}

/// Extract image `file_ids` from polled tasks when generation has completed.
pub fn poll_image_ready_from_tasks(tasks: &[Value]) -> Option<Vec<String>> {
    let mut file_ids = Vec::new();
    for task in tasks {
        if task_is_structured_error(task) {
            continue;
        }
        if let Some(ids) = task.get("file_ids").and_then(|v| v.as_array()) {
            let parsed = ids
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            add_unique_file_ids(&mut file_ids, &parsed);
        }
        if let Some(img_msg) = task.get("image_gen_message") {
            let payload = serde_json::to_string(img_msg).unwrap_or_default();
            let (_, extracted, _) = extract_conversation_ids(&payload);
            add_unique_file_ids(&mut file_ids, &extracted);
        }
    }
    if file_ids.is_empty() {
        None
    } else {
        Some(file_ids)
    }
}

/// Back off after a failed conversation GET.
///
/// A flat 2s retry is *shorter* than the normal poll interval, so an upstream 429
/// made us poll faster than usual and kept the rate limit engaged — one request
/// once burned 141 attempts over its whole budget without ever recovering.
fn poll_error_backoff(err: &str, consecutive: u32, interval: Duration) -> Duration {
    const MAX_BACKOFF: Duration = Duration::from_secs(30);
    let base = if err.contains("429") {
        interval.max(Duration::from_secs(5))
    } else {
        Duration::from_secs(2)
    };
    let shift = consecutive.saturating_sub(1).min(4);
    base.saturating_mul(1u32 << shift).min(MAX_BACKOFF)
}

async fn cancel_aware_sleep(deadline: Instant, duration: Duration) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let sleep_for = duration.min(remaining);
    if sleep_for > Duration::ZERO {
        tokio::time::sleep(sleep_for).await;
    }
}

/// Poll conversation document (primary) and tasks (secondary) until image ids appear or wall budget expires.
pub async fn poll_image_conversation<F>(
    client: &Client,
    headers_fn: F,
    conversation_id: &str,
    config: &ImagePollConfig,
    initial_file_ids: &[String],
    initial_sediment_ids: &[String],
) -> Result<ImagePollOutcome>
where
    F: Fn(&str) -> Vec<(String, String)>,
{
    if conversation_id.is_empty() {
        bail!("poll_image_conversation requires conversation_id");
    }

    let started = Instant::now();
    let deadline = started + config.timeout;
    let mut file_ids: Vec<String> = initial_file_ids.to_vec();
    let mut sediment_ids: Vec<String> = initial_sediment_ids.to_vec();
    let mut attempt: u32 = 0;
    let mut tasks_gets: u32 = 0;
    let mut last_task_error = String::new();
    let mut last_hit_key: Option<(Vec<String>, Vec<String>)> = None;
    let mut consecutive_errors: u32 = 0;
    let mut terminal_task_streak: u32 = 0;
    let mut no_activity_streak: u32 = 0;
    let mut consecutive_task_query_errors: u32 = 0;
    let mut tasks_seen_max: usize = 0;
    let no_activity_max = env_u32(
        "UPSTREAM_IMAGE_POLL_NO_ACTIVITY_MAX",
        DEFAULT_NO_ACTIVITY_MAX_ATTEMPTS,
    )
    .max(5);
    let task_query_fail_max = env_u32(
        "UPSTREAM_IMAGE_POLL_TASK_QUERY_FAIL_MAX",
        DEFAULT_TASK_QUERY_FAIL_MAX,
    )
    .max(3);

    if file_ids.is_empty() && sediment_ids.is_empty() && config.initial_wait > Duration::ZERO {
        info!(
            conversation_id = %conversation_id,
            initial_wait_secs = config.initial_wait.as_secs_f64(),
            timeout_secs = config.timeout.as_secs_f64(),
            "image poll start (post-SSE)"
        );
        cancel_aware_sleep(deadline, config.initial_wait).await;
    }

    while Instant::now() < deadline {
        attempt += 1;

        if tasks_gets < config.max_tasks_gets && attempt % config.tasks_every_n_attempts == 0 {
            tasks_gets += 1;
            match query_tasks(client, &headers_fn, conversation_id).await {
                Ok(tasks) => {
                    consecutive_task_query_errors = 0;
                    if let Some(err) = last_task_error_from_tasks(&tasks) {
                        last_task_error = err;
                    }
                    tasks_seen_max = tasks_seen_max.max(tasks.len());
                    match tasks_all_terminal(&tasks) {
                        Some(true) => terminal_task_streak += 1,
                        Some(false) => terminal_task_streak = 0,
                        None => {}
                    }
                    if let Some(ids) = poll_image_ready_from_tasks(&tasks) {
                        add_unique_file_ids(&mut file_ids, &ids);
                    }
                }
                Err(e) => {
                    consecutive_task_query_errors += 1;
                    last_task_error = e.to_string();
                    warn!(
                        conversation_id = %conversation_id,
                        attempt,
                        consecutive_task_query_errors,
                        error = %e,
                        "image poll query_tasks failed"
                    );
                }
            }
        }

        match get_conversation(client, &headers_fn, conversation_id).await {
            Ok(conversation) => {
                consecutive_errors = 0;
                if let Some(reason) = detect_image_gen_failure_from_conversation(&conversation) {
                    bail!("upstream image generation failed: {reason}");
                }
                let (conv_files, conv_sediments) =
                    extract_image_ids_from_conversation(&conversation);
                add_unique_file_ids(&mut file_ids, &conv_files);
                add_unique_sediment_ids(&mut sediment_ids, &conv_sediments);

                // 任务已全部终结时也要放行终止判定：`conversation_has_image_gen_activity`
                // 只看 image_gen 节点是否存在，任务一旦启动它就恒为 true，会把下面的
                // 失败识别永久屏蔽掉，让失败的生成空转到 wall budget 用尽。
                let generation_settled = terminal_task_streak >= TERMINAL_TASK_CONFIRMATIONS;
                let has_activity = conversation_has_image_gen_activity(&conversation);
                let no_ids = file_ids.is_empty() && sediment_ids.is_empty();
                if no_ids && !has_activity {
                    no_activity_streak += 1;
                } else {
                    no_activity_streak = 0;
                }
                if no_ids && (generation_settled || !has_activity) {
                    if let Some((code, msg)) =
                        find_terminal_upstream_block_in_conversation(&conversation)
                    {
                        bail!("upstream image generation failed ({code}): {msg}");
                    }
                    if !last_task_error.is_empty() {
                        if let Some((code, msg)) = classify_terminal_upstream_text(&last_task_error)
                        {
                            bail!("upstream image generation failed ({code}): {msg}");
                        }
                    }
                    if generation_settled {
                        let detail = if last_task_error.is_empty() {
                            String::new()
                        } else {
                            format!(": {last_task_error}")
                        };
                        bail!(
                            "upstream image generation finished without an image \
                             (conversation_id={conversation_id}, attempts={attempt}){detail}"
                        );
                    }
                    // 生图工具自始至终没被调用：会话里没有 image_gen 节点，也就没有异步
                    // 任务可查，继续轮询到 420s 也不会凭空出现图片。
                    if no_activity_streak >= no_activity_max {
                        bail!(
                            "upstream image generation was never started \
                             (conversation_id={conversation_id}, attempts={attempt}, \
                             no_image_gen_activity_for={no_activity_streak} polls, \
                             tasks_seen={tasks_seen_max})"
                        );
                    }
                }
                if should_stop_after_task_query_failures(
                    consecutive_task_query_errors,
                    task_query_fail_max,
                    no_ids,
                    has_activity,
                ) {
                    bail!(
                        "upstream image generation stalled: task query failed \
                         {consecutive_task_query_errors} times \
                         (conversation_id={conversation_id}, attempts={attempt}, \
                         last_error={last_task_error})"
                    );
                }
            }
            Err(err) => {
                consecutive_errors += 1;
                let backoff =
                    poll_error_backoff(&err.to_string(), consecutive_errors, config.interval);
                info!(
                    conversation_id = %conversation_id,
                    attempt,
                    consecutive_errors,
                    backoff_secs = backoff.as_secs_f64(),
                    error = %err,
                    "image poll conversation GET failed"
                );
                cancel_aware_sleep(deadline, backoff).await;
                continue;
            }
        }

        if !file_ids.is_empty() || !sediment_ids.is_empty() {
            let hit_key = (file_ids.clone(), sediment_ids.clone());
            if config.check_before_hit {
                if last_hit_key.as_ref() == Some(&hit_key) {
                    info!(
                        conversation_id = %conversation_id,
                        attempt,
                        file_ids = file_ids.len(),
                        sediment_ids = sediment_ids.len(),
                        elapsed_secs = started.elapsed().as_secs_f64(),
                        "image poll succeeded"
                    );
                    return Ok(ImagePollOutcome {
                        file_ids,
                        sediment_ids,
                    });
                }
                last_hit_key = Some(hit_key);
                if config.settle > Duration::ZERO {
                    cancel_aware_sleep(deadline, config.settle).await;
                    continue;
                }
            }
            info!(
                conversation_id = %conversation_id,
                attempt,
                file_ids = file_ids.len(),
                sediment_ids = sediment_ids.len(),
                elapsed_secs = started.elapsed().as_secs_f64(),
                "image poll succeeded"
            );
            return Ok(ImagePollOutcome {
                file_ids,
                sediment_ids,
            });
        }

        // 这几个字段是判定为何没能早停的唯一依据：线上出现过 94 轮空转而早停从未
        // 触发，事后无法从日志区分「任务列表为空」「任务永不终结」「工具没被调用」。
        info!(
            conversation_id = %conversation_id,
            attempt,
            remaining_secs = deadline.saturating_duration_since(Instant::now()).as_secs_f64(),
            tasks_seen_max,
            terminal_task_streak,
            no_activity_streak,
            consecutive_task_query_errors,
            "image poll check: no ids yet"
        );
        cancel_aware_sleep(deadline, config.interval).await;
    }

    bail!(
        "image poll timeout after {:.1}s (conversation_id={}, attempts={})",
        started.elapsed().as_secs_f64(),
        conversation_id,
        attempt
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASKS_FIXTURE: &str = r#"{
        "tasks": [
            {
                "conversation_id": "conv-abc",
                "task_id": "task-1",
                "status": "completed",
                "file_ids": ["file_00000000a1b2c3d4e5f678901234"],
                "image_gen_message": {
                    "author": { "role": "tool" },
                    "content": {
                        "content_type": "multimodal_text",
                        "parts": [
                            {
                                "content_type": "image_asset_pointer",
                                "asset_pointer": "file-service://file_00000000a1b2c3d4e5f678901234"
                            }
                        ]
                    },
                    "metadata": { "async_task_type": "image_gen" }
                }
            },
            {
                "conversation_id": "conv-abc",
                "task_id": "task-2",
                "status": "failed",
                "image_gen_message": {
                    "author": { "role": "assistant" },
                    "content": {
                        "content_type": "text",
                        "parts": ["moderation blocked"]
                    },
                    "metadata": { "is_error": true }
                }
            }
        ]
    }"#;

    #[test]
    fn poll_image_ready_from_tasks_fixture() {
        let data: Value = serde_json::from_str(TASKS_FIXTURE).unwrap();
        let tasks = data
            .get("tasks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let file_ids = poll_image_ready_from_tasks(&tasks).expect("file_ids");
        assert_eq!(file_ids, ["file_00000000a1b2c3d4e5f678901234"]);
    }

    #[test]
    fn poll_image_ready_skips_structured_errors() {
        let tasks = serde_json::from_str::<Value>(
            r#"{
                "tasks": [{
                    "image_gen_message": {
                        "author": { "role": "assistant" },
                        "content": { "content_type": "text", "parts": ["blocked"] },
                        "metadata": { "is_error": true }
                    }
                }]
            }"#,
        )
        .unwrap()
        .get("tasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
        assert!(poll_image_ready_from_tasks(&tasks).is_none());
    }

    #[test]
    fn extract_image_ids_from_conversation_mapping() {
        let data: Value = serde_json::from_str(
            r#"{
                "mapping": {
                    "m1": {
                        "message": {
                            "author": { "role": "tool" },
                            "metadata": { "async_task_type": "image_gen" },
                            "content": {
                                "content_type": "multimodal_text",
                                "parts": [{
                                    "content_type": "image_asset_pointer",
                                    "asset_pointer": "file-service://file_00000000a1b2c3d4e5f678901234"
                                }]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let (file_ids, sediment_ids) = extract_image_ids_from_conversation(&data);
        assert_eq!(file_ids, ["file_00000000a1b2c3d4e5f678901234"]);
        assert!(sediment_ids.is_empty());
    }

    #[test]
    fn rate_limited_backoff_exceeds_poll_interval() {
        let interval = Duration::from_secs(3);
        let err = "conversation HTTP 429 Too Many Requests: {\"detail\":\"Too many requests\"}";
        let first = poll_error_backoff(err, 1, interval);
        assert!(
            first > interval,
            "a 429 must slow polling down, got {first:?} vs interval {interval:?}"
        );
        // grows with consecutive failures, but stays bounded
        assert!(poll_error_backoff(err, 3, interval) > first);
        assert_eq!(
            poll_error_backoff(err, 99, interval),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn transient_error_backoff_stays_short() {
        let interval = Duration::from_secs(3);
        assert_eq!(
            poll_error_backoff("connection reset", 1, interval),
            Duration::from_secs(2)
        );
    }

    fn tasks_of(json: &str) -> Vec<Value> {
        serde_json::from_str::<Value>(json)
            .unwrap()
            .get("tasks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn tasks_all_terminal_detects_finished_run() {
        let tasks = tasks_of(r#"{"tasks":[{"status":"completed"},{"status":"failed"}]}"#);
        assert_eq!(tasks_all_terminal(&tasks), Some(true));
    }

    #[test]
    fn tasks_all_terminal_false_while_running() {
        let tasks = tasks_of(r#"{"tasks":[{"status":"completed"},{"status":"running"}]}"#);
        assert_eq!(tasks_all_terminal(&tasks), Some(false));
    }

    #[test]
    fn tasks_all_terminal_undecidable_without_status() {
        assert_eq!(tasks_all_terminal(&[]), None);
        let tasks = tasks_of(r#"{"tasks":[{"file_ids":[]}]}"#);
        assert_eq!(
            tasks_all_terminal(&tasks),
            None,
            "上游没给 status 时不得据此判失败"
        );
    }

    #[test]
    fn conversation_without_image_gen_node_never_started_generation() {
        // 复现线上 420s 空转：SSE 结束无 file_id，会话里也没有 image_gen 节点，
        // 说明生图工具压根没被调用——此时 tasks 为空，旧逻辑无任何出口。
        let data: Value = serde_json::from_str(
            r#"{
                "mapping": {
                    "m1": {
                        "message": {
                            "author": { "role": "assistant" },
                            "content": { "content_type": "text", "parts": ["好的"] }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        assert!(!conversation_has_image_gen_activity(&data));
        let (files, sediments) = extract_image_ids_from_conversation(&data);
        assert!(files.is_empty() && sediments.is_empty());
        // 已知文案匹配不到，所以只能靠 no_activity_streak 收尾。
        assert!(find_terminal_upstream_block_in_conversation(&data).is_none());
        assert_eq!(tasks_all_terminal(&[]), None, "空任务列表不可判定");
    }

    #[test]
    fn task_query_failures_stop_when_activity_has_no_ids() {
        assert!(should_stop_after_task_query_failures(8, 8, true, true));
        assert!(
            !should_stop_after_task_query_failures(7, 8, true, true),
            "未达阈值不得提前放弃"
        );
        assert!(
            !should_stop_after_task_query_failures(8, 8, false, true),
            "已经拿到图就继续走完，不要误杀"
        );
        assert!(
            !should_stop_after_task_query_failures(8, 8, true, false),
            "没有 image_gen 活动时走 no_activity_streak，不走这条"
        );
    }

    #[test]
    fn image_gen_activity_alone_must_not_gate_termination() {
        // 复现线上 420s 空转：image_gen 节点存在 → 旧逻辑永久跳过终止判定。
        let data: Value = serde_json::from_str(
            r#"{
                "mapping": {
                    "m1": {
                        "message": {
                            "author": { "role": "assistant" },
                            "metadata": { "async_task_type": "image_gen" },
                            "content": { "content_type": "text", "parts": ["working"] }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        assert!(conversation_has_image_gen_activity(&data));
        let (files, sediments) = extract_image_ids_from_conversation(&data);
        assert!(files.is_empty() && sediments.is_empty());
        // 任务侧已终结 → 必须能判定收尾，而不是继续等满 wall budget。
        let tasks = tasks_of(r#"{"tasks":[{"status":"failed"}]}"#);
        assert_eq!(tasks_all_terminal(&tasks), Some(true));
    }

    #[test]
    fn classify_terminal_content_policy() {
        let (code, _) = classify_terminal_upstream_text("抱歉，我不能生成这类图片").unwrap();
        assert_eq!(code, "content_policy_violation");
    }

    #[test]
    fn terminal_block_skipped_while_image_gen_active() {
        let data: Value = serde_json::from_str(
            r#"{
                "mapping": {
                    "m1": {
                        "message": {
                            "author": { "role": "assistant" },
                            "metadata": { "async_task_type": "image_gen" },
                            "content": { "content_type": "text", "parts": ["working"] }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        assert!(conversation_has_image_gen_activity(&data));
        assert!(find_terminal_upstream_block_in_conversation(&data).is_none());
    }
}
