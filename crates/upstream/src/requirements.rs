use std::time::Duration;

use anyhow::{bail, Context, Result};
use rand::{rngs::StdRng, SeedableRng};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;
use wreq::Client;

use crate::account::PinAccount;
use crate::conversation::{
    build_image_prepare_body, build_image_start_body, image_model_slug, DEFAULT_TIMEZONE,
};
use crate::pow::{
    build_legacy_requirements_token, build_proof_token, parse_pow_resources, DEFAULT_POW_SCRIPT,
};
use crate::sentinel::{
    build_image_start_headers, PURE_HTTP_IMAGE_CLIENT_BUILD_NUMBER, PURE_HTTP_IMAGE_CLIENT_VERSION,
};
use crate::tls::{ChromeProfile, ClientPlatform, TlsClientBuilder};
use crate::turnstile::solve_turnstile_token;

pub const BASE_URL: &str = "https://chatgpt.com";
pub const DEFAULT_CLIENT_VERSION: &str = "prod-773467609da990104e0f78db96ed90bc4b199c3b";
pub const DEFAULT_CLIENT_BUILD_NUMBER: &str = "8448714";

#[derive(Debug, Clone)]
pub struct ChatRequirements {
    pub token: String,
    pub proof_token: String,
    pub turnstile_token: String,
    pub so_token: String,
    pub raw_finalize: Value,
}

#[derive(Debug, Clone)]
pub struct BootstrapResources {
    pub script_sources: Vec<String>,
    pub data_build: String,
}

pub struct RequirementsClient {
    client: Client,
    account: PinAccount,
    pow_script_sources: Vec<String>,
    pow_data_build: String,
    rng: StdRng,
}

impl RequirementsClient {
    pub fn new(account: PinAccount) -> Result<Self> {
        let profile = ChromeProfile::from_impersonate(&account.impersonate);
        let platform = if account.user_agent.contains("Macintosh") {
            ClientPlatform::MacOS
        } else {
            ClientPlatform::from_fp("windows")
        };
        let ua = if account.user_agent.trim().is_empty() {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string()
        } else {
            account.user_agent.clone()
        };
        let client = TlsClientBuilder::new()
            .profile(profile)
            .platform(platform)
            .proxy(account.proxy.clone())
            .user_agent(ua.clone())
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            client,
            account,
            pow_script_sources: Vec::new(),
            pow_data_build: String::new(),
            rng: StdRng::from_rng(&mut rand::rng()),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn account(&self) -> &PinAccount {
        &self.account
    }

    pub fn user_agent(&self) -> &str {
        if self.account.user_agent.trim().is_empty() {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
        } else {
            self.account.user_agent.as_str()
        }
    }

    pub fn device_id(&self) -> String {
        if self.account.device_id.trim().is_empty() {
            Uuid::new_v4().to_string()
        } else {
            self.account.device_id.clone()
        }
    }

    fn session_id(&self) -> String {
        Uuid::new_v4().to_string()
    }

    pub fn api_headers(&self, path: &str) -> Vec<(String, String)> {
        vec![
            ("User-Agent".into(), self.user_agent().to_string()),
            ("Origin".into(), BASE_URL.into()),
            ("Referer".into(), format!("{BASE_URL}/")),
            (
                "Accept-Language".into(),
                "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7".into(),
            ),
            ("Cache-Control".into(), "no-cache".into()),
            ("Pragma".into(), "no-cache".into()),
            ("Priority".into(), "u=1, i".into()),
            (
                "Sec-Ch-Ua".into(),
                "\"Chromium\";v=\"124\", \"Google Chrome\";v=\"124\", \"Not-A.Brand\";v=\"99\""
                    .into(),
            ),
            ("Sec-Ch-Ua-Mobile".into(), "?0".into()),
            ("Sec-Ch-Ua-Platform".into(), "\"Windows\"".into()),
            ("Sec-Fetch-Dest".into(), "empty".into()),
            ("Sec-Fetch-Mode".into(), "cors".into()),
            ("Sec-Fetch-Site".into(), "same-origin".into()),
            ("OAI-Device-Id".into(), self.device_id()),
            ("OAI-Session-Id".into(), self.session_id()),
            ("OAI-Language".into(), "zh-CN".into()),
            ("OAI-Client-Version".into(), DEFAULT_CLIENT_VERSION.into()),
            (
                "OAI-Client-Build-Number".into(),
                DEFAULT_CLIENT_BUILD_NUMBER.into(),
            ),
            (
                "Authorization".into(),
                format!("Bearer {}", self.account.access_token),
            ),
            ("X-OpenAI-Target-Path".into(), path.into()),
            ("X-OpenAI-Target-Route".into(), path.into()),
        ]
    }

    pub(crate) fn apply_headers(
        builder: wreq::RequestBuilder,
        headers: &[(String, String)],
    ) -> wreq::RequestBuilder {
        let mut req = builder;
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req
    }

    /// Homepage bootstrap — extract PoW scripts (`openai_backend_api.py::_bootstrap`).
    pub async fn bootstrap(&mut self, soft_fail: bool) -> Result<BootstrapResources> {
        let path = "/";
        let headers = self.api_headers(path);
        let resp = match Self::apply_headers(self.client.get(format!("{BASE_URL}{path}")), &headers)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) if soft_fail => {
                warn!(error = %e, "bootstrap soft-fail");
                self.pow_script_sources = vec![DEFAULT_POW_SCRIPT.to_string()];
                self.pow_data_build.clear();
                return Ok(BootstrapResources {
                    script_sources: self.pow_script_sources.clone(),
                    data_build: self.pow_data_build.clone(),
                });
            }
            Err(e) => return Err(e.into()),
        };
        let status = resp.status();
        let body = resp.text().await.context("bootstrap body")?;
        if !status.is_success() {
            if soft_fail {
                warn!(status = %status, "bootstrap soft-fail status");
                self.pow_script_sources = vec![DEFAULT_POW_SCRIPT.to_string()];
                self.pow_data_build.clear();
                return Ok(BootstrapResources {
                    script_sources: self.pow_script_sources.clone(),
                    data_build: self.pow_data_build.clone(),
                });
            }
            bail!("bootstrap HTTP {status}");
        }
        let (sources, build) = parse_pow_resources(&body);
        self.pow_script_sources = if sources.is_empty() {
            vec![DEFAULT_POW_SCRIPT.to_string()]
        } else {
            sources
        };
        self.pow_data_build = build;
        Ok(BootstrapResources {
            script_sources: self.pow_script_sources.clone(),
            data_build: self.pow_data_build.clone(),
        })
    }

    /// Sentinel prepare + finalize (`_get_chat_requirements_once`).
    pub async fn fetch_chat_requirements(&mut self) -> Result<ChatRequirements> {
        let base = "/backend-api/sentinel/chat-requirements";
        let ua = self.user_agent().to_string();
        let scripts = self.pow_script_sources.clone();
        let data_build = self.pow_data_build.clone();
        let p_token =
            build_legacy_requirements_token(&ua, Some(&scripts), &data_build, &mut self.rng);

        let prepare_path = format!("{base}/prepare");
        let mut hdrs = self.api_headers(&prepare_path);
        hdrs.push(("Content-Type".into(), "application/json".into()));
        let prepare_body = serde_json::to_string(&json!({"p": p_token}))?;
        let prepare_resp = Self::apply_headers(
            self.client
                .post(format!("{BASE_URL}{prepare_path}"))
                .header("Content-Type", "application/json")
                .body(prepare_body),
            &hdrs,
        )
        .send()
        .await
        .context("chat_requirements_prepare")?;
        let prepare_status = prepare_resp.status();
        let prepare_body = prepare_resp.text().await?;
        if !prepare_status.is_success() {
            bail!(
                "chat_requirements_prepare HTTP {prepare_status}: {}",
                &prepare_body[..prepare_body.len().min(240)]
            );
        }
        let prepare_data: PrepareResponse =
            serde_json::from_str(&prepare_body).context("parse chat_requirements prepare")?;

        if prepare_data
            .arkose
            .as_ref()
            .and_then(|a| a.required)
            .unwrap_or(false)
        {
            bail!("chat requirements requires arkose token, which is not implemented");
        }

        let mut proof_token = String::new();
        if prepare_data
            .proofofwork
            .as_ref()
            .and_then(|p| p.required)
            .unwrap_or(false)
        {
            let pow = prepare_data.proofofwork.as_ref().unwrap();
            proof_token = build_proof_token(
                pow.seed.as_deref().unwrap_or(""),
                pow.difficulty.as_deref().unwrap_or(""),
                &ua,
                Some(&scripts),
                &data_build,
                &mut self.rng,
            )?;
        }

        let mut turnstile_token = String::new();
        let turnstile_required = prepare_data
            .turnstile
            .as_ref()
            .and_then(|t| t.required)
            .unwrap_or(false);
        if turnstile_required {
            if let Some(dx) = prepare_data
                .turnstile
                .as_ref()
                .and_then(|t| t.dx.as_deref())
            {
                turnstile_token = solve_turnstile_token(dx, &p_token).unwrap_or_default();
            }
            if turnstile_token.is_empty() {
                bail!(
                    "chat_requirements_turnstile_required_but_unsolved: prepare demanded turnstile but local VM returned empty token"
                );
            }
        }

        let finalize_path = format!("{base}/finalize");
        let finalize_body = json!({
            "prepare_token": prepare_data.prepare_token.unwrap_or_default(),
            "proofofwork": proof_token,
            "turnstile": turnstile_token,
        });
        info!(
            turnstile_required,
            turnstile_solved_len = turnstile_token.len(),
            proof_solved_len = proof_token.len(),
            "chat_requirements_finalize"
        );
        let mut finalize_headers = self.api_headers(&finalize_path);
        finalize_headers.push(("Content-Type".into(), "application/json".into()));
        let finalize_body_str = serde_json::to_string(&finalize_body)?;
        let finalize_resp = Self::apply_headers(
            self.client
                .post(format!("{BASE_URL}{finalize_path}"))
                .header("Content-Type", "application/json")
                .body(finalize_body_str),
            &finalize_headers,
        )
        .send()
        .await
        .context("chat_requirements_finalize")?;
        let finalize_status = finalize_resp.status();
        let finalize_text = finalize_resp.text().await?;
        if !finalize_status.is_success() {
            bail!(
                "chat_requirements_finalize HTTP {finalize_status}: {}",
                &finalize_text[..finalize_text.len().min(240)]
            );
        }
        let finalize_data: FinalizeResponse =
            serde_json::from_str(&finalize_text).context("parse chat_requirements finalize")?;
        let token = finalize_data.token.unwrap_or_default();
        if token.is_empty() {
            bail!("missing auth chat requirements token: {finalize_text}");
        }
        Ok(ChatRequirements {
            token,
            proof_token,
            turnstile_token,
            so_token: finalize_data.so_token.unwrap_or_default(),
            raw_finalize: serde_json::from_str(&finalize_text).unwrap_or(Value::Null),
        })
    }

    fn oai_language_for_timezone(&self, tz_name: &str) -> String {
        match tz_name {
            "Asia/Shanghai" | "Asia/Chongqing" => "zh-CN".into(),
            "Asia/Tokyo" => "ja-JP".into(),
            t if t.starts_with("Europe/") => "en-GB".into(),
            _ => "en-US".into(),
        }
    }

    /// SPA image envelope headers (`openai_backend_api.py::_image_headers`, spa_tool_path=true).
    pub fn image_spa_headers(
        &self,
        path: &str,
        accept: &str,
        requirements: Option<&ChatRequirements>,
        conduit_token: &str,
        is_prepare: bool,
    ) -> Vec<(String, String)> {
        let mut headers = if is_prepare {
            vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), accept.to_string()),
            ]
        } else {
            let req = requirements.expect("requirements required for image start");
            build_image_start_headers(req, conduit_token, true)
        };
        if let Some((_, accept_hdr)) = headers.iter_mut().find(|(k, _)| k == "Accept") {
            if !accept.is_empty() {
                *accept_hdr = accept.to_string();
            }
        }
        let tz = DEFAULT_TIMEZONE;
        let mut built = vec![
            ("User-Agent".into(), self.user_agent().to_string()),
            (
                "Accept-Language".into(),
                "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7".into(),
            ),
            ("OAI-Device-Id".into(), self.device_id()),
            ("OAI-Session-Id".into(), self.session_id()),
            (
                "OAI-Client-Version".into(),
                PURE_HTTP_IMAGE_CLIENT_VERSION.into(),
            ),
            (
                "OAI-Client-Build-Number".into(),
                PURE_HTTP_IMAGE_CLIENT_BUILD_NUMBER.into(),
            ),
            ("OAI-Language".into(), self.oai_language_for_timezone(tz)),
            ("Origin".into(), BASE_URL.into()),
            ("Referer".into(), format!("{BASE_URL}/")),
            (
                "Authorization".into(),
                format!("Bearer {}", self.account.access_token),
            ),
        ];
        built.extend(headers);
        built.push(("X-OpenAI-Target-Path".into(), path.into()));
        built.push(("X-OpenAI-Target-Route".into(), path.into()));
        built
    }

    /// POST `/backend-api/f/conversation/prepare` for image (`_prepare_image_conversation`).
    pub async fn prepare_image_conversation(
        &self,
        prompt: &str,
        model: &str,
        _requirements: &ChatRequirements,
    ) -> Result<String> {
        let path = "/backend-api/f/conversation/prepare";
        let slug = image_model_slug(model);
        let body = build_image_prepare_body(prompt, slug, DEFAULT_TIMEZONE, true);
        let headers = self.image_spa_headers(path, "*/*", None, "", true);
        let body_str = serde_json::to_string(&body)?;
        let resp = Self::apply_headers(
            self.client.post(format!("{BASE_URL}{path}")).body(body_str),
            &headers,
        )
        .send()
        .await
        .context("image_prepare")?;
        let status = resp.status();
        let text = resp.text().await.context("image_prepare body")?;
        if !status.is_success() {
            bail!(
                "image_prepare HTTP {status}: {}",
                &text[..text.len().min(240)]
            );
        }
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(parsed
            .get("conduit_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string())
    }

    /// POST `/backend-api/f/conversation` for image start (returns HTTP response for SSE).
    pub async fn start_image_conversation(
        &self,
        prompt: &str,
        model: &str,
        requirements: &ChatRequirements,
        _conduit_token: &str,
    ) -> Result<wreq::Response> {
        let path = "/backend-api/f/conversation";
        let slug = image_model_slug(model);
        let body = build_image_start_body(prompt, slug, DEFAULT_TIMEZONE, &[], true);
        let headers =
            self.image_spa_headers(path, "text/event-stream", Some(requirements), "", false);
        let body_str = serde_json::to_string(&body)?;
        let resp = Self::apply_headers(
            self.client.post(format!("{BASE_URL}{path}")).body(body_str),
            &headers,
        )
        .send()
        .await
        .context("image_start")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "image_start HTTP {status}: {}",
                &text[..text.len().min(240)]
            );
        }
        Ok(resp)
    }
}

#[derive(Debug, Deserialize)]
struct PrepareResponse {
    prepare_token: Option<String>,
    arkose: Option<FlagField>,
    proofofwork: Option<PowField>,
    turnstile: Option<TurnstileField>,
}

#[derive(Debug, Deserialize)]
struct FlagField {
    required: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PowField {
    required: Option<bool>,
    seed: Option<String>,
    difficulty: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TurnstileField {
    required: Option<bool>,
    dx: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FinalizeResponse {
    token: Option<String>,
    so_token: Option<String>,
}
