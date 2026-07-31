//! Poll `/backend-api/tasks` for async image generation results.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use wreq::Client;

use crate::requirements::{RequirementsClient, BASE_URL};
use crate::sse::extract_conversation_ids;

const SKIP_FILE_IDS: &[&str] = &["file_upload"];

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

fn task_is_structured_error(task: &Value) -> bool {
    let img_msg = task.get("image_gen_message").and_then(|v| v.as_object());
    if img_msg.is_none() {
        return false;
    }
    let img_msg = img_msg.unwrap();
    let metadata = img_msg.get("metadata").and_then(|v| v.as_object());
    let content = img_msg.get("content").and_then(|v| v.as_object());
    let author = img_msg.get("author").and_then(|v| v.as_object());
    let is_error = metadata
        .and_then(|m| m.get("is_error"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_text_only = content
        .and_then(|c| c.get("content_type"))
        .and_then(|v| v.as_str())
        == Some("text");
    let is_assistant_role = author
        .and_then(|a| a.get("role"))
        .and_then(|v| v.as_str())
        == Some("assistant");
    is_error && is_text_only && is_assistant_role
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
        bail!(
            "tasks HTTP {status}: {}",
            &text[..text.len().min(240)]
        );
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
}
