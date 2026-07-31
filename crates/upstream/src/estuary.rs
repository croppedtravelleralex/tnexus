use std::time::Duration;

use anyhow::{bail, Context, Result};
use protocol::{build_estuary_download_headers, validate_estuary_headers};
use serde_json::Value;
use tracing::warn;
use wreq::Client;

use crate::requirements::{RequirementsClient, BASE_URL};

fn download_url_from_payload(payload: &Value) -> String {
    payload
        .get("download_url")
        .or_else(|| payload.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn estuary_request_headers(access_token: &str) -> Result<Vec<(String, String)>> {
    let headers = build_estuary_download_headers(access_token);
    validate_estuary_headers(&headers).map_err(|e| anyhow::anyhow!(e))?;
    Ok(headers
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect())
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

/// GET `/backend-api/files/{id}/download` and parse `download_url` (`openai_backend_api.py::_get_file_download_url`).
pub async fn get_file_download_url(
    client: &Client,
    base_headers: &[(String, String)],
    file_id: &str,
) -> Result<String> {
    let path = format!("/backend-api/files/{file_id}/download");
    let headers = with_accept_json(base_headers.to_vec());
    let resp = RequirementsClient::apply_headers(client.get(format!("{BASE_URL}{path}")), &headers)
        .send()
        .await
        .context("file_download_url")?;
    let status = resp.status();
    let text = resp.text().await.context("file_download_url body")?;
    if !status.is_success() {
        bail!(
            "file_download_url HTTP {status}: {}",
            &text[..text.len().min(240)]
        );
    }
    let data: Value = serde_json::from_str(&text).context("parse file_download_url")?;
    let url = download_url_from_payload(&data);
    if url.is_empty() {
        bail!("empty download_url for file {file_id}");
    }
    Ok(url)
}

/// GET `/backend-api/conversation/{cid}/attachment/{id}/download` (`_get_attachment_download_url`).
pub async fn get_attachment_download_url(
    client: &Client,
    base_headers: &[(String, String)],
    conversation_id: &str,
    attachment_id: &str,
) -> Result<String> {
    let path =
        format!("/backend-api/conversation/{conversation_id}/attachment/{attachment_id}/download");
    let headers = with_accept_json(base_headers.to_vec());
    let resp = RequirementsClient::apply_headers(client.get(format!("{BASE_URL}{path}")), &headers)
        .send()
        .await
        .context("attachment_download_url")?;
    let status = resp.status();
    let text = resp.text().await.context("attachment_download_url body")?;
    if !status.is_success() {
        bail!(
            "attachment_download_url HTTP {status}: {}",
            &text[..text.len().min(240)]
        );
    }
    let data: Value = serde_json::from_str(&text).context("parse attachment_download_url")?;
    let url = download_url_from_payload(&data);
    if url.is_empty() {
        bail!("empty download_url for attachment {attachment_id}");
    }
    Ok(url)
}

/// Download image bytes from estuary URL with Bearer auth; retry 403/404 up to 3 times.
pub async fn download_image_bytes(
    client: &Client,
    url: &str,
    access_token: &str,
) -> Result<Vec<u8>> {
    let headers = estuary_request_headers(access_token)?;
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=3 {
        let resp = RequirementsClient::apply_headers(client.get(url), &headers)
            .send()
            .await;
        match resp {
            Ok(response) => {
                let status = response.status();
                let code = status.as_u16();
                if (code == 403 || code == 404) && attempt < 3 {
                    warn!(attempt, status = code, "image_download retryable status");
                    tokio::time::sleep(Duration::from_millis((1500 * attempt) as u64)).await;
                    continue;
                }
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    bail!(
                        "image_download HTTP {status}: {}",
                        &body[..body.len().min(240)]
                    );
                }
                let bytes = response.bytes().await.context("image_download body")?;
                if bytes.is_empty() {
                    bail!("image_download empty body");
                }
                return Ok(bytes.to_vec());
            }
            Err(err) => {
                last_err = Some(err.into());
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis((1500 * attempt) as u64)).await;
                    continue;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("image_download failed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_download_url_fields() {
        assert_eq!(
            download_url_from_payload(&json!({"download_url": "https://example/a"})),
            "https://example/a"
        );
        assert_eq!(
            download_url_from_payload(&json!({"url": "https://example/b"})),
            "https://example/b"
        );
        assert!(download_url_from_payload(&json!({})).is_empty());
    }

    #[test]
    fn estuary_headers_validate_via_protocol() {
        let headers = build_estuary_download_headers("test-token");
        assert!(validate_estuary_headers(&headers).is_ok());
        let built = estuary_request_headers("test-token").expect("headers");
        assert!(built
            .iter()
            .any(|(k, v)| k == "Authorization" && v.contains("test-token")));
    }
}
