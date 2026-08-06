//! OpenAI platform OAuth PKCE (mirrors `helper/oauth_login.py`).

use crate::pkce::generate_pkce;
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use urlencoding::encode;

const AUTH_BASE: &str = "https://auth.openai.com";
const PLATFORM_BASE: &str = "https://platform.openai.com";
const PLATFORM_OAUTH_CLIENT_ID: &str = "app_2SKx67EdpoN0G6j64rFvigXD";
const PLATFORM_OAUTH_REDIRECT_URI: &str = "https://platform.openai.com/auth/callback";
const PLATFORM_OAUTH_AUDIENCE: &str = "https://api.openai.com/v1";
const PLATFORM_AUTH0_CLIENT: &str = "eyJuYW1lIjoiYXV0aDAtc3BhLWpzIiwidmVyc2lvbiI6IjEuMjEuMCJ9";
const USER_AGENT_STR: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
const SEC_CH_UA: &str =
    "\"Google Chrome\";v=\"145\", \"Not?A_Brand\";v=\"8\", \"Chromium\";v=\"145\"";

const SESSION_TTL: Duration = Duration::from_secs(600);
const MAX_SESSIONS: usize = 64;

#[derive(Clone)]
struct OAuthSession {
    code_verifier: String,
    state: String,
    redirect_uri: String,
    created_at: Instant,
}

pub struct OAuthLoginService {
    sessions: Mutex<HashMap<String, OAuthSession>>,
    http: reqwest::Client,
}

impl OAuthLoginService {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            http,
        }
    }

    fn purge_expired(&self, sessions: &mut HashMap<String, OAuthSession>) {
        sessions.retain(|_, s| s.created_at.elapsed() < SESSION_TTL);
        if sessions.len() > MAX_SESSIONS {
            let mut ordered: Vec<(String, Instant)> = sessions
                .iter()
                .map(|(k, v)| (k.clone(), v.created_at))
                .collect();
            ordered.sort_by_key(|(_, t)| *t);
            for (sid, _) in ordered
                .into_iter()
                .take(sessions.len().saturating_sub(MAX_SESSIONS))
            {
                sessions.remove(&sid);
            }
        }
    }

    pub fn start(&self, email_hint: &str) -> Result<Value> {
        let (verifier, challenge) = generate_pkce();
        let session_id = uuid::Uuid::new_v4().simple().to_string();
        let state_suffix = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>());
        let state = format!("{session_id}.{state_suffix}");
        let device_id = uuid::Uuid::new_v4().to_string();
        let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 24]>());

        let mut params: Vec<(&str, String)> = vec![
            ("issuer", AUTH_BASE.to_string()),
            ("client_id", PLATFORM_OAUTH_CLIENT_ID.to_string()),
            ("audience", PLATFORM_OAUTH_AUDIENCE.to_string()),
            ("redirect_uri", PLATFORM_OAUTH_REDIRECT_URI.to_string()),
            ("device_id", device_id),
            ("screen_hint", "login_or_signup".to_string()),
            ("max_age", "0".to_string()),
            ("scope", "openid profile email offline_access".to_string()),
            ("response_type", "code".to_string()),
            ("response_mode", "query".to_string()),
            ("state", state.clone()),
            ("nonce", nonce),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256".to_string()),
            ("auth0Client", PLATFORM_AUTH0_CLIENT.to_string()),
        ];
        let email_hint = email_hint.trim();
        if !email_hint.is_empty() {
            params.push(("login_hint", email_hint.to_string()));
        }
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let authorize_url = format!("{AUTH_BASE}/api/accounts/authorize?{query}");

        let mut sessions = self.sessions.lock().expect("oauth sessions lock");
        self.purge_expired(&mut sessions);
        sessions.insert(
            session_id.clone(),
            OAuthSession {
                code_verifier: verifier,
                state,
                redirect_uri: PLATFORM_OAUTH_REDIRECT_URI.to_string(),
                created_at: Instant::now(),
            },
        );

        Ok(json!({
            "session_id": session_id,
            "authorize_url": authorize_url,
            "expires_in": SESSION_TTL.as_secs().to_string(),
            "redirect_uri_prefix": PLATFORM_OAUTH_REDIRECT_URI,
        }))
    }

    fn extract_code_from_callback(value: &str) -> Result<(String, String)> {
        let raw = value.trim();
        if raw.is_empty() {
            return Ok((String::new(), String::new()));
        }
        if raw.starts_with("http://") || raw.starts_with("https://") {
            let parsed = url::Url::parse(raw).context("parse callback URL")?;
            let mut code = String::new();
            let mut state = String::new();
            let mut err = String::new();
            for (k, v) in parsed.query_pairs() {
                match k.as_ref() {
                    "code" => code = v.to_string(),
                    "state" => state = v.to_string(),
                    "error" | "error_description" => {
                        if err.is_empty() {
                            err = v.to_string();
                        }
                    }
                    _ => {}
                }
            }
            if code.is_empty() {
                return Err(anyhow!(
                    "{}",
                    if err.is_empty() {
                        "callback URL 中没有 code 参数".to_string()
                    } else {
                        err
                    }
                ));
            }
            return Ok((code, state));
        }
        Ok((raw.to_string(), String::new()))
    }

    pub async fn finish(&self, session_id: &str, callback: &str) -> Result<Value> {
        let (code, state) = Self::extract_code_from_callback(callback)?;
        if code.is_empty() {
            return Err(anyhow!("缺少 code 或 callback URL"));
        }
        let body_sid = session_id.trim();
        let state_sid = state.split('.').next().unwrap_or("");
        let candidates: Vec<&str> = [state_sid, body_sid]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        if candidates.is_empty() {
            return Err(anyhow!(
                "既未提供 session_id，callback URL 中也未携带 state"
            ));
        }

        let session = {
            let mut sessions = self.sessions.lock().expect("oauth sessions lock");
            self.purge_expired(&mut sessions);
            let mut picked = None;
            let mut picked_sid = String::new();
            for sid in candidates {
                if let Some(s) = sessions.get(sid) {
                    picked = Some(s.clone());
                    picked_sid = sid.to_string();
                    break;
                }
            }
            if picked.is_none() {
                return Err(anyhow!("OAuth 会话已过期或不存在，请重新生成授权链接"));
            }
            let s = picked.unwrap();
            if !state.is_empty() && !s.state.is_empty() && state != s.state {
                return Err(anyhow!("state 不匹配，请点「重新生成」后再走一次授权"));
            }
            sessions.remove(&picked_sid);
            s
        };

        self.exchange_code(&code, &session.code_verifier, &session.redirect_uri)
            .await
    }

    async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<Value> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(ORIGIN, HeaderValue::from_static(PLATFORM_BASE));
        headers.insert(REFERER, HeaderValue::from_static(PLATFORM_BASE));
        headers.insert(
            "auth0-client",
            HeaderValue::from_static(PLATFORM_AUTH0_CLIENT),
        );
        headers.insert("sec-ch-ua", HeaderValue::from_static(SEC_CH_UA));
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));

        let body = json!({
            "client_id": PLATFORM_OAUTH_CLIENT_ID,
            "code_verifier": code_verifier,
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
        });

        let response = self
            .http
            .post(format!("{AUTH_BASE}/api/accounts/oauth/token"))
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("oauth token exchange")?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let data: Value = serde_json::from_str(&text).unwrap_or(json!({}));

        if !status.is_success() {
            let detail = data
                .get("error_description")
                .or_else(|| data.get("error"))
                .or_else(|| data.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return Err(anyhow!(
                "OpenAI 拒绝换 token (HTTP {status}){detail}",
                detail = if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let access_token = data
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let refresh_token = data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if access_token.is_empty() {
            return Err(anyhow!("OpenAI 返回的 access_token 为空"));
        }
        if refresh_token.is_empty() {
            return Err(anyhow!("OpenAI 没有返回 refresh_token"));
        }
        Ok(json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
            "id_token": data.get("id_token").and_then(|v| v.as_str()).unwrap_or(""),
        }))
    }
}
