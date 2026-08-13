//! Grok 纯 HTTP 探针：对齐 `scripts/grok_pure_http_client.py --gate`。
//!
//! 用法（本机）：
//! ```text
//! GROK_LOCAL_PROXY=http://127.0.0.1:7897 \
//!   cargo run -p grok-pure-http -- \
//!   --keys /path/to/pure_http_keys/email_at_domain.json \
//!   --image /path/to/probe.png \
//!   --gate
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use base64::Engine;
use clap::Parser;
use grok_provider_web::{DirectConfig, HttpDirectClient, SessionKeys};
use serde_json::json;
use tracing::info;

const DEFAULT_IMAGE: &str = r"C:\Users\Lenovo\Downloads\image-1785287126849-88e3a45901dc98-1785287699703-649ee24e9542d8.png";

#[derive(Parser, Debug)]
#[command(name = "grok-pure-http")]
struct Args {
    /// pure_http_keys/*.json（含 meta_b64、fingerprint、sso）
    #[arg(long)]
    keys: PathBuf,
    /// OCR 探针图片（默认用户指定 PNG）
    #[arg(long, default_value = DEFAULT_IMAGE)]
    image: PathBuf,
    /// 跑完整 gate（upload + 多轮 chat + OCR）
    #[arg(long)]
    gate: bool,
    /// 单条 chat 消息（非 gate 模式）
    #[arg(long, default_value = "Reply with exactly: PONG")]
    message: String,
    /// 本地出口代理（缺省读 GROK_LOCAL_PROXY）
    #[arg(long, env = "GROK_LOCAL_PROXY")]
    local_proxy: Option<String>,
    /// 上游账号出口代理（webshare/udeal；缺省读 GROK_UPSTREAM_PROXY）
    #[arg(long, env = "GROK_UPSTREAM_PROXY")]
    upstream_proxy: Option<String>,
    /// 代理模式标签（写入报告）
    #[arg(long, default_value = "local")]
    proxy_label: String,
}

#[derive(Debug, serde::Serialize)]
struct Step {
    name: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct GateReport {
    proxy_label: String,
    keys: String,
    image: String,
    steps: Vec<Step>,
    ok: bool,
    followup_ok: bool,
    ocr_ok: bool,
    upload_ok: bool,
}

fn load_keys(path: &PathBuf) -> Result<(serde_json::Value, String)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let sso = value
        .get("sso")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("keys missing sso")?;
    Ok((value, sso))
}

fn build_client(args: &Args, session: SessionKeys) -> HttpDirectClient {
    let mut proxy_pool = grok_provider_web::proxy::ProxyPool::empty();
    if let Some(up) = args.upstream_proxy.as_deref() {
        if !up.trim().is_empty() {
            proxy_pool = grok_provider_web::proxy::ProxyPool::from_text(up);
        }
    }
    let cfg = DirectConfig {
        local_proxy: args.local_proxy.clone(),
        proxy: Arc::new(proxy_pool),
        session: Some(session),
        ..DirectConfig::default()
    };
    HttpDirectClient::new(cfg)
}

fn chat_payload(message: &str) -> serde_json::Value {
    json!({
        "collectionIds": [],
        "disabledConnectorIds": [],
        "deviceEnvInfo": {
            "darkModeEnabled": false,
            "devicePixelRatio": 2,
            "screenHeight": 1328,
            "screenWidth": 2056,
            "viewportHeight": 1083,
            "viewportWidth": 2056,
        },
        "disableMemory": true,
        "disableSearch": false,
        "disableSelfHarmShortCircuit": false,
        "disableTextFollowUps": false,
        "enableImageGeneration": false,
        "enableImageStreaming": false,
        "enableSideBySide": true,
        "fileAttachments": [],
        "forceConcise": false,
        "forceSideBySide": false,
        "imageAttachments": [],
        "imageGenerationCount": 0,
        "isAsyncChat": false,
        "message": message,
        "modeId": "fast",
        "responseMetadata": {},
        "returnImageBytes": false,
        "returnRawGrokInXaiRequest": false,
        "sendFinalMetadata": true,
        "temporary": true,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let (keys_json, sso) = load_keys(&args.keys)?;
    let session = SessionKeys::from_json(&keys_json).map_err(|e| anyhow::anyhow!("{e}"))?;
    let client = build_client(&args, session);

    if !args.gate {
        let payload = chat_payload(&args.message);
        let turn = client
            .fetch_chat_turn(
                "/rest/app-chat/conversations/new",
                &payload,
                Some(&sso),
                None,
            )
            .await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": !turn.text.is_empty(),
                "reply": turn.text,
                "conversation_id": turn.conversation_id,
                "response_id": turn.response_id,
            }))?
        );
        return Ok(());
    }

    let mut steps = Vec::new();
    let mut file_id: Option<String> = None;

    // upload
    if args.image.exists() {
        let bytes = std::fs::read(&args.image)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let mime = if args.image.extension().and_then(|s| s.to_str()) == Some("png") {
            "image/png"
        } else {
            "application/octet-stream"
        };
        match client
            .upload_file_b64(
                args.image
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("probe.png"),
                mime,
                &b64,
                &sso,
            )
            .await
        {
            Ok(id) => {
                info!(%id, "upload ok");
                file_id = Some(id.clone());
                steps.push(Step {
                    name: "upload_file".into(),
                    ok: true,
                    error: None,
                    reply: None,
                    file_id: Some(id),
                });
            }
            Err(e) => steps.push(Step {
                name: "upload_file".into(),
                ok: false,
                error: Some(e.to_string()),
                reply: None,
                file_id: None,
            }),
        }
    } else {
        steps.push(Step {
            name: "upload_file".into(),
            ok: false,
            error: Some(format!("image missing: {}", args.image.display())),
            reply: None,
            file_id: None,
        });
    }

    // chat round 1
    let r1 = client
        .fetch_chat_turn(
            "/rest/app-chat/conversations/new",
            &chat_payload("Reply with exactly: PONG"),
            Some(&sso),
            None,
        )
        .await;
    let (conv_id, resp_id) = match &r1 {
        Ok(t) => {
            steps.push(Step {
                name: "chat_new_text".into(),
                ok: !t.text.is_empty(),
                error: None,
                reply: Some(t.text.clone()),
                file_id: None,
            });
            (t.conversation_id.clone(), t.response_id.clone())
        }
        Err(e) => {
            steps.push(Step {
                name: "chat_new_text".into(),
                ok: false,
                error: Some(e.to_string()),
                reply: None,
                file_id: None,
            });
            (None, None)
        }
    };

    // followup 1
    let mut resp2 = None;
    if let (Some(conv), Some(parent)) = (conv_id.clone(), resp_id.clone()) {
        match client
            .fetch_chat_followup(
                &conv,
                &parent,
                &chat_payload("Reply with exactly: PONG2"),
                Some(&sso),
                None,
            )
            .await
        {
            Ok(t) => {
                resp2 = t.response_id.clone();
                steps.push(Step {
                    name: "chat_followup".into(),
                    ok: !t.text.is_empty(),
                    error: None,
                    reply: Some(t.text),
                    file_id: None,
                });
            }
            Err(e) => steps.push(Step {
                name: "chat_followup".into(),
                ok: false,
                error: Some(e.to_string()),
                reply: None,
                file_id: None,
            }),
        }
    }

    // followup 2
    if let (Some(conv), Some(parent)) = (conv_id, resp2) {
        match client
            .fetch_chat_followup(
                &conv,
                &parent,
                &chat_payload("What were my previous two replies? One short sentence."),
                Some(&sso),
                None,
            )
            .await
        {
            Ok(t) => steps.push(Step {
                name: "chat_followup_2".into(),
                ok: !t.text.is_empty(),
                error: None,
                reply: Some(t.text),
                file_id: None,
            }),
            Err(e) => steps.push(Step {
                name: "chat_followup_2".into(),
                ok: false,
                error: Some(e.to_string()),
                reply: None,
                file_id: None,
            }),
        }
    }

    // OCR with uploaded file（对齐 Python canary.chat_payload + fileAttachments）
    if let Some(fid) = file_id {
        let mut ocr_payload = chat_payload("提取图中全部可见文字，若无文字则描述画面。");
        ocr_payload["fileAttachments"] = json!([fid]);
        match client
            .fetch_chat_turn(
                "/rest/app-chat/conversations/new",
                &ocr_payload,
                Some(&sso),
                None,
            )
            .await
        {
            Ok(t) => steps.push(Step {
                name: "chat_ocr_with_file".into(),
                ok: !t.text.is_empty(),
                error: None,
                reply: Some(t.text),
                file_id: None,
            }),
            Err(e) => steps.push(Step {
                name: "chat_ocr_with_file".into(),
                ok: false,
                error: Some(e.to_string()),
                reply: None,
                file_id: None,
            }),
        }
    }

    let report = GateReport {
        proxy_label: args.proxy_label.clone(),
        keys: args.keys.display().to_string(),
        image: args.image.display().to_string(),
        ok: steps.iter().any(|s| s.name == "chat_new_text" && s.ok),
        followup_ok: steps
            .iter()
            .any(|s| s.name.starts_with("chat_followup") && s.ok),
        ocr_ok: steps.iter().any(|s| s.name == "chat_ocr_with_file" && s.ok),
        upload_ok: steps.iter().any(|s| s.name == "upload_file" && s.ok),
        steps,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.ok {
        bail!("gate failed");
    }
    Ok(())
}
