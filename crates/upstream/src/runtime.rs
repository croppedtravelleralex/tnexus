use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tracing::info;

use crate::account::PinAccount;
use crate::conversation::{build_text_chat_body, ImageReference, DEFAULT_TIMEZONE};
use crate::estuary::{download_image_bytes, get_attachment_download_url, get_file_download_url};
use crate::image_metrics::ImageRunMetrics;
use crate::poll::{poll_image_ready_from_tasks, query_tasks};
use crate::requirements::{RequirementsClient, BASE_URL};
use crate::sentinel::build_chat_headers;
use crate::sse::{consume_sse_until, ImageSseReady, SseConsumeMode};
use crate::upload::{upload_image_bytes, uploaded_to_reference};

const DEFAULT_TEXT_SSE_TIMEOUT_SECS: u64 = 120;
const DEFAULT_IMAGE_SSE_TIMEOUT_SECS: u64 = 300;

/// End-to-end upstream runtime: requirements → conversation SSE → estuary download.
pub struct UpstreamRuntime {
    client: RequirementsClient,
    text_sse_timeout: Duration,
    image_sse_timeout: Duration,
}

impl UpstreamRuntime {
    pub fn new(account: PinAccount) -> Result<Self> {
        Ok(Self {
            client: RequirementsClient::new(account)?,
            text_sse_timeout: Duration::from_secs(DEFAULT_TEXT_SSE_TIMEOUT_SECS),
            image_sse_timeout: Duration::from_secs(DEFAULT_IMAGE_SSE_TIMEOUT_SECS),
        })
    }

    pub fn client(&self) -> &RequirementsClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut RequirementsClient {
        &mut self.client
    }

    /// Bootstrap → chat-requirements → text SSE response (caller consumes stream).
    pub async fn start_text_stream(&mut self, prompt: &str, model: &str) -> Result<wreq::Response> {
        self.client.bootstrap(true).await?;
        let requirements = self.client.fetch_chat_requirements().await?;

        let path = "/backend-api/f/conversation";
        let body = build_text_chat_body(prompt, model, DEFAULT_TIMEZONE);
        let mut headers = self.client.api_headers(path);
        headers.extend(build_chat_headers(&requirements));
        let body_str = serde_json::to_string(&body)?;

        let http = RequirementsClient::apply_headers(
            self.client
                .client()
                .post(format!("{BASE_URL}{path}"))
                .body(body_str),
            &headers,
        );
        let resp = http.send().await.context("POST /f/conversation")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "conversation HTTP {status}: {}",
                &body[..body.len().min(240)]
            );
        }
        Ok(resp)
    }

    /// Bootstrap → chat-requirements → text SSE → collected assistant text.
    pub async fn run_text(&mut self, prompt: &str, model: &str) -> Result<String> {
        let resp = self.start_text_stream(prompt, model).await?;
        let consumed = consume_sse_until(resp, SseConsumeMode::Text, self.text_sse_timeout).await?;
        let text = consumed.parser.state().text.clone();
        info!(
            conversation_id = %consumed.parser.state().conversation_id,
            text_len = text.len(),
            events = consumed.parser.event_count(),
            "text conversation complete"
        );
        Ok(text)
    }

    /// Bootstrap → requirements → image prepare/start → SSE → estuary download.
    pub async fn run_image(&mut self, prompt: &str, model: &str) -> Result<Vec<u8>> {
        self.run_image_with_metrics(prompt, model)
            .await
            .map(|(bytes, _)| bytes)
    }

    pub async fn run_image_with_metrics(
        &mut self,
        prompt: &str,
        model: &str,
    ) -> Result<(Vec<u8>, ImageRunMetrics)> {
        self.run_image_with_references(prompt, model, &[]).await
    }

    pub async fn run_image_edit_with_metrics(
        &mut self,
        prompt: &str,
        model: &str,
        image_bytes: &[u8],
        file_name: &str,
    ) -> Result<(Vec<u8>, ImageRunMetrics)> {
        let uploaded = upload_image_bytes(self.client(), image_bytes, file_name).await?;
        let reference = uploaded_to_reference(&uploaded);
        self.run_image_with_references(prompt, model, &[reference])
            .await
    }

    async fn run_image_with_references(
        &mut self,
        prompt: &str,
        model: &str,
        references: &[ImageReference],
    ) -> Result<(Vec<u8>, ImageRunMetrics)> {
        let wall_start = Instant::now();
        let mut metrics = ImageRunMetrics::default();

        let t0 = Instant::now();
        self.client.bootstrap(true).await?;
        metrics.bootstrap_ms = t0.elapsed().as_millis() as u64;

        let t0 = Instant::now();
        let requirements = self.client.fetch_chat_requirements().await?;
        metrics.requirements_ms = t0.elapsed().as_millis() as u64;

        let t0 = Instant::now();
        let conduit = self
            .client
            .prepare_image_conversation(prompt, model, &requirements)
            .await?;
        metrics.prepare_ms = t0.elapsed().as_millis() as u64;
        info!(conduit_len = conduit.len(), "image prepare complete");

        let t0 = Instant::now();
        let resp = self
            .client
            .start_image_conversation(prompt, model, &requirements, &conduit, references)
            .await?;

        let consumed =
            consume_sse_until(resp, SseConsumeMode::Image, self.image_sse_timeout).await?;
        metrics.sse_ms = t0.elapsed().as_millis() as u64;
        metrics.sse_bytes_in = consumed.bytes_in;
        metrics.sse_events = consumed.parser.event_count() as u32;
        let ready = consumed
            .parser
            .image_ready()
            .context("image SSE ended without file_id")?;

        let mut ready_for_download = ready;
        let t0 = Instant::now();
        let download_url = match self.resolve_image_download_url(&ready_for_download).await {
            Ok(url) => url,
            Err(initial_err) => {
                if ready_for_download.conversation_id.is_empty() {
                    metrics.resolve_url_ms = t0.elapsed().as_millis() as u64;
                    metrics.wall_ms = wall_start.elapsed().as_millis() as u64;
                    return Err(initial_err);
                }
                let mut last_err = initial_err;
                let mut resolved_url = None;
                let poll_start = Instant::now();
                for attempt in 0..3 {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    let tasks = query_tasks(
                        self.client.client(),
                        |path| self.client.api_headers(path),
                        &ready_for_download.conversation_id,
                    )
                    .await
                    .unwrap_or_default();
                    if let Some(file_ids) = poll_image_ready_from_tasks(&tasks) {
                        for file_id in file_ids {
                            if !ready_for_download.file_ids.contains(&file_id) {
                                ready_for_download.file_ids.push(file_id);
                            }
                        }
                        match self.resolve_image_download_url(&ready_for_download).await {
                            Ok(url) => {
                                resolved_url = Some(url);
                                break;
                            }
                            Err(err) => {
                                info!(attempt, error = %err, "tasks poll download url still missing");
                                last_err = err;
                            }
                        }
                    } else {
                        info!(attempt, "tasks poll returned no file_ids");
                    }
                }
                metrics.poll_tasks_ms = poll_start.elapsed().as_millis() as u64;
                match resolved_url {
                    Some(url) => url,
                    None => {
                        metrics.resolve_url_ms = t0.elapsed().as_millis() as u64;
                        metrics.wall_ms = wall_start.elapsed().as_millis() as u64;
                        return Err(last_err);
                    }
                }
            }
        };
        metrics.resolve_url_ms = t0.elapsed().as_millis() as u64;

        let t0 = Instant::now();
        let access_token = self.client.account().access_token.clone();
        let bytes = download_image_bytes(self.client.client(), &download_url, &access_token).await?;
        metrics.download_ms = t0.elapsed().as_millis() as u64;
        metrics.image_bytes = bytes.len() as u64;
        metrics.wall_ms = wall_start.elapsed().as_millis() as u64;
        Ok((bytes, metrics))
    }

    async fn resolve_image_download_url(&self, ready: &ImageSseReady) -> Result<String> {
        const SKIP_FILE_IDS: &[&str] = &["file_upload"];

        for file_id in &ready.file_ids {
            if SKIP_FILE_IDS.contains(&file_id.as_str()) {
                continue;
            }
            let path = format!("/backend-api/files/{file_id}/download");
            let headers = self.client.api_headers(&path);
            match get_file_download_url(self.client.client(), &headers, file_id).await {
                Ok(url) if !url.is_empty() => return Ok(url),
                Ok(_) => continue,
                Err(err) => {
                    info!(%file_id, error = %err, "file download url lookup failed");
                }
            }
        }

        if !ready.conversation_id.is_empty() {
            for sediment_id in &ready.sediment_ids {
                let path = format!(
                    "/backend-api/conversation/{}/attachment/{}/download",
                    ready.conversation_id, sediment_id
                );
                let headers = self.client.api_headers(&path);
                match get_attachment_download_url(
                    self.client.client(),
                    &headers,
                    &ready.conversation_id,
                    sediment_id,
                )
                .await
                {
                    Ok(url) if !url.is_empty() => return Ok(url),
                    Ok(_) => continue,
                    Err(err) => {
                        info!(%sediment_id, error = %err, "attachment download url lookup failed");
                    }
                }
            }
        }

        bail!(
            "no image download url resolved (file_ids={:?}, sediment_ids={:?})",
            ready.file_ids,
            ready.sediment_ids
        )
    }
}
