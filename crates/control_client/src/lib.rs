//! HTTP client for gptimage control-plane admission APIs (Phase C stub).

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct ControlClient {
    base: String,
    http: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub intent: String,
    #[serde(default)]
    pub min_image_quota: Option<i64>,
    #[serde(default)]
    pub preferred_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionResponse {
    pub ok: bool,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub fault: Option<String>,
}

impl ControlClient {
    pub fn new(base: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build control http client")?;
        Ok(Self {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        })
    }

    /// POST /api/accounts/admission — stub against gptimage control plane.
    pub async fn request_admission(&self, req: &AdmissionRequest) -> Result<AdmissionResponse> {
        let url = format!("{}/api/accounts/admission", self.base);
        let resp = self
            .http
            .post(&url)
            .json(req)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let parsed: AdmissionResponse = resp.json().await.context("decode admission response")?;
        if !status.is_success() && parsed.reason.is_none() {
            return Err(anyhow!("admission status={status}"));
        }
        Ok(parsed)
    }

    /// Health probe for control plane (optional).
    pub async fn health(&self) -> Result<bool> {
        let url = format!("{}/health", self.base);
        let resp = self.http.get(url).send().await.context("control health")?;
        Ok(resp.status().is_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admission_mock_roundtrip() {
        let body = serde_json::json!({
            "ok": true,
            "email": "test@example.com",
            "reason": "schedulable"
        });
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/accounts/admission")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body.to_string())
            .create_async()
            .await;
        let client = ControlClient::new(server.url()).unwrap();
        let resp = client
            .request_admission(&AdmissionRequest {
                intent: "image".into(),
                min_image_quota: Some(1),
                preferred_email: None,
            })
            .await
            .unwrap();
        mock.assert_async().await;
        assert!(resp.ok);
        assert_eq!(resp.email.as_deref(), Some("test@example.com"));
    }
}
