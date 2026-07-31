//! Panda/WSL upstream probe — Phase 1 acceptance:
//! TLS (optional) → bootstrap → chat-requirements → text SSE ready / image file_id.
//!
//! Env:
//!   PIN_ACCOUNT_FILE   path to pin account json (required)
//!   PROBE_STEPS        comma list: tls,bootstrap,requirements,sse,image (default all except image)
//!   FP_ENDPOINT        TLS reflection URL
//!   PROBE_PROMPT       text chat prompt (default short ping)
//!   PROBE_IMAGE_PROMPT image prompt (default simple scene)
//!   PROBE_IMAGE_MODEL  default gpt-image-2
//!   PROBE_SSE_TIMEOUT_SECS  default 120
//!   PROBE_IMAGE_TIMEOUT_SECS default 300

use std::env;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tracing::{info, warn};
use upstream::conversation::build_text_chat_body;
use upstream::requirements::{RequirementsClient, BASE_URL};
use upstream::sentinel::build_chat_headers;
use upstream::sse::{consume_sse_until, SseConsumeMode};
use upstream::tls::{fp_endpoint, probe_tls_fingerprint};
use upstream::PinAccount;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let pin_path = env::var("PIN_ACCOUNT_FILE").context("PIN_ACCOUNT_FILE required")?;
    let account = PinAccount::load(&pin_path)?;
    info!(email = %account.redacted_email(), "loaded pin account");

    let default_steps = "tls,bootstrap,requirements,sse";
    let steps: Vec<String> = env::var("PROBE_STEPS")
        .unwrap_or_else(|_| default_steps.into())
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let mut client = RequirementsClient::new(account)?;
    let mut requirements = None;

    if steps.iter().any(|s| s == "tls") {
        let endpoint = fp_endpoint();
        info!(%endpoint, "tls probe start");
        let fp = probe_tls_fingerprint(client.client(), &endpoint).await?;
        let ja3 = fp
            .get("ja3n_hash")
            .or_else(|| fp.get("ja3_hash"))
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".into());
        info!(%ja3, "tls probe ok");
        println!("TLS_OK ja3={ja3}");
    }

    if steps.iter().any(|s| s == "bootstrap") {
        let boot = client.bootstrap(true).await?;
        info!(
            scripts = boot.script_sources.len(),
            data_build = %boot.data_build,
            "bootstrap ok"
        );
        println!(
            "BOOTSTRAP_OK scripts={} build={}",
            boot.script_sources.len(),
            boot.data_build
        );
    } else if steps
        .iter()
        .any(|s| s == "requirements" || s == "sse" || s == "image")
    {
        let _ = client.bootstrap(true).await?;
    }

    if steps.iter().any(|s| s == "requirements")
        || steps.iter().any(|s| s == "sse")
        || steps.iter().any(|s| s == "image")
    {
        let req = client.fetch_chat_requirements().await?;
        info!(
            token_len = req.token.len(),
            proof_len = req.proof_token.len(),
            turnstile_len = req.turnstile_token.len(),
            "chat requirements ok"
        );
        println!(
            "REQUIREMENTS_OK token_len={} proof_len={} turnstile_len={}",
            req.token.len(),
            req.proof_token.len(),
            req.turnstile_token.len()
        );
        requirements = Some(req);
    }

    if steps.iter().any(|s| s == "sse") {
        let requirements = requirements
            .as_ref()
            .context("requirements step required before sse")?;
        let prompt =
            env::var("PROBE_PROMPT").unwrap_or_else(|_| "Reply with one short word.".into());
        let timeout_secs: u64 = env::var("PROBE_SSE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        let path = "/backend-api/f/conversation";
        let body = build_text_chat_body(&prompt, "auto", "Asia/Shanghai");
        let mut headers = client.api_headers(path);
        headers.extend(build_chat_headers(requirements));
        let body_str = serde_json::to_string(&body)?;
        let http = client
            .client()
            .post(format!("{BASE_URL}{path}"))
            .body(body_str);
        let mut req = http;
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.context("POST /f/conversation")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "conversation HTTP {status}: {}",
                &body[..body.len().min(240)]
            );
        }
        let consumed = consume_sse_until(
            resp,
            SseConsumeMode::Text,
            Duration::from_secs(timeout_secs),
        )
        .await
        .context("text sse")?;
        if let Some(ready) = consumed.parser.text_ready() {
            info!(
                conversation_id = %ready.conversation_id,
                saw_delta = ready.saw_delta,
                events = ready.event_count,
                "text sse ready"
            );
            println!(
                "SSE_READY conversation_id={} saw_delta={} events={}",
                ready.conversation_id, ready.saw_delta, ready.event_count
            );
        }
        if !steps.iter().any(|s| s == "image") {
            return Ok(());
        }
    }

    if steps.iter().any(|s| s == "image") {
        let requirements = requirements
            .as_ref()
            .context("requirements step required before image")?;
        let prompt = env::var("PROBE_IMAGE_PROMPT")
            .unwrap_or_else(|_| "a red cube on a white background, product photo".into());
        let model = env::var("PROBE_IMAGE_MODEL").unwrap_or_else(|_| "gpt-image-2".into());
        let timeout_secs: u64 = env::var("PROBE_IMAGE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        let conduit = client
            .prepare_image_conversation(&prompt, &model, requirements)
            .await?;
        info!(conduit_len = conduit.len(), "image prepare ok");
        println!("IMAGE_PREPARE_OK conduit_len={}", conduit.len());

        let resp = client
            .start_image_conversation(&prompt, &model, requirements, &conduit)
            .await?;
        let consumed = consume_sse_until(
            resp,
            SseConsumeMode::Image,
            Duration::from_secs(timeout_secs),
        )
        .await
        .context("image sse")?;
        if let Some(ready) = consumed.parser.image_ready() {
            info!(
                conversation_id = %ready.conversation_id,
                file_ids = ?ready.file_ids,
                sediment_ids = ?ready.sediment_ids,
                events = ready.event_count,
                "image sse ready"
            );
            println!(
                "IMAGE_READY conversation_id={} file_ids={} sediment_ids={} events={}",
                ready.conversation_id,
                ready.file_ids.join(","),
                ready.sediment_ids.join(","),
                ready.event_count
            );
        } else {
            warn!("image sse ended without image_ready predicate");
        }
        return Ok(());
    }

    println!("PROBE_OK");
    Ok(())
}
