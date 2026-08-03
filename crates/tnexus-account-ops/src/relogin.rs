//! Password relogin via auth.openai.com (best-effort; OTP accounts need OAuth).

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::{STANDARD as B64_STD, URL_SAFE_NO_PAD};
use base64::Engine;
use reqwest::cookie::{CookieStore, Jar};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

use crate::pkce::generate_pkce;
use crate::user_info;

const AUTH_BASE: &str = "https://auth.openai.com";
const PLATFORM_BASE: &str = "https://platform.openai.com";
const PLATFORM_OAUTH_CLIENT_ID: &str = "app_2SKx67EdpoN0G6j64rFvigXD";
const PLATFORM_OAUTH_REDIRECT_URI: &str = "https://platform.openai.com/auth/callback";
const PLATFORM_OAUTH_AUDIENCE: &str = "https://api.openai.com/v1";
const PLATFORM_AUTH0_CLIENT: &str = "eyJuYW1lIjoiYXV0aDAtc3BhLWpzIiwidmVyc2lvbiI6IjEuMjEuMCJ9";
const USER_AGENT_STR: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
const SEC_CH_UA: &str = "\"Google Chrome\";v=\"145\", \"Not?A_Brand\";v=\"8\", \"Chromium\";v=\"145\"";

pub async fn relogin_account(account: &Map<String, Value>) -> Result<Map<String, Value>> {
    let email = account
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let password = account
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if email.is_empty() || password.is_empty() {
        anyhow::bail!("账号缺少 email/password，无法密码重登");
    }

    let jar = Arc::new(Jar::default());
    let http = reqwest::Client::builder()
        .cookie_provider(jar.clone())
        .redirect(reqwest::redirect::Policy::limited(15))
        .timeout(Duration::from_secs(120))
        .build()
        .context("relogin http client")?;

    let (verifier, challenge) = generate_pkce();
    let state_suffix = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>());
    let state = format!("relogin.{}", state_suffix);
    let device_id = uuid::Uuid::new_v4().to_string();

    let authorize_q = format!(
        "issuer={}&client_id={}&audience={}&redirect_uri={}&device_id={}&screen_hint=login&max_age=0&scope=openid%20profile%20email%20offline_access&response_type=code&response_mode=query&state={}&code_challenge={}&code_challenge_method=S256&auth0Client={}",
        urlencoding::encode(AUTH_BASE),
        urlencoding::encode(PLATFORM_OAUTH_CLIENT_ID),
        urlencoding::encode(PLATFORM_OAUTH_AUDIENCE),
        urlencoding::encode(PLATFORM_OAUTH_REDIRECT_URI),
        urlencoding::encode(&device_id),
        urlencoding::encode(&state),
        urlencoding::encode(&challenge),
        urlencoding::encode(PLATFORM_AUTH0_CLIENT),
    );
    let authorize_url = format!("{AUTH_BASE}/api/accounts/authorize?{authorize_q}");
    http.get(&authorize_url)
        .header("User-Agent", USER_AGENT_STR)
        .send()
        .await
        .context("authorize bootstrap")?;

    let sentinel_header = fetch_sentinel_header(&http, &device_id).await?;
    let continue_body = json!({
        "username": { "value": email, "kind": "email" },
        "screen_hint": "login",
    });
    let continue_resp = http
        .post(format!("{AUTH_BASE}/api/accounts/authorize/continue"))
        .header("User-Agent", USER_AGENT_STR)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Referer", "https://auth.openai.com/log-in")
        .header("openai-sentinel-token", &sentinel_header)
        .json(&continue_body)
        .send()
        .await
        .context("authorize/continue")?;
    let continue_status = continue_resp.status();
    let continue_text = continue_resp.text().await.unwrap_or_default();
    if !continue_status.is_success() {
        anyhow::bail!(
            "authorize/continue HTTP {continue_status}: {}",
            continue_text.chars().take(300).collect::<String>()
        );
    }
    let continue_json: Value = serde_json::from_str(&continue_text).unwrap_or(json!({}));
    let page_type = continue_json
        .get("page")
        .and_then(|p| p.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if page_type.contains("otp") || page_type == "email_otp_verification" {
        anyhow::bail!("账号需要邮箱 OTP 验证，请使用 OAuth 导入");
    }
    if !page_type.is_empty() && !page_type.contains("password") {
        anyhow::bail!("不支持的登录页面类型: {page_type}，请使用 OAuth");
    }

    let pwd_resp = http
        .post(format!("{AUTH_BASE}/api/accounts/password/verify"))
        .header("User-Agent", USER_AGENT_STR)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Referer", "https://auth.openai.com/log-in/password")
        .header("openai-sentinel-token", &sentinel_header)
        .json(&json!({ "password": password }))
        .send()
        .await
        .context("password/verify")?;
    let pwd_status = pwd_resp.status();
    let pwd_text = pwd_resp.text().await.unwrap_or_default();
    if !pwd_status.is_success() {
        anyhow::bail!(
            "password/verify HTTP {pwd_status}: {}",
            pwd_text.chars().take(300).collect::<String>()
        );
    }
    let pwd_json: Value = serde_json::from_str(&pwd_text).unwrap_or(json!({}));
    let pwd_page = pwd_json
        .get("page")
        .and_then(|p| p.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if pwd_page.contains("otp") {
        anyhow::bail!("密码校验后需要 OTP，请使用 OAuth 导入");
    }

    let tokens = complete_token_exchange(&http, &jar, &verifier, &state).await?;
    let mut acc = account.clone();
    for key in ["access_token", "refresh_token", "id_token"] {
        if let Some(v) = tokens.get(key).and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                acc.insert(key.into(), json!(v.trim()));
            }
        }
    }
    acc.insert("status".into(), json!("正常"));
    acc.insert(
        "source_type".into(),
        json!(acc.get("source_type").and_then(|v| v.as_str()).unwrap_or("password")),
    );
    Ok(user_info::merge_user_info(&http, &acc).await)
}

async fn fetch_sentinel_header(http: &reqwest::Client, device_id: &str) -> Result<String> {
    if let Ok(prefixed) = std::env::var("OPENAI_SENTINEL_HEADER") {
        if !prefixed.trim().is_empty() {
            return Ok(prefixed.trim().to_string());
        }
    }
    let body = json!({ "p": "", "id": device_id, "flow": "authorize_continue" }).to_string();
    let resp = http
        .post("https://sentinel.openai.com/backend-api/sentinel/req")
        .header("User-Agent", USER_AGENT_STR)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Origin", "https://sentinel.openai.com")
        .header(
            "Referer",
            "https://sentinel.openai.com/backend-api/sentinel/frame.html",
        )
        .body(body)
        .send()
        .await
        .context("sentinel req")?;
    if !resp.status().is_success() {
        anyhow::bail!("sentinel HTTP {}", resp.status());
    }
    let data: Value = resp.json().await.unwrap_or(json!({}));
    let token = data
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        anyhow::bail!(
            "sentinel returned empty token (set OPENAI_SENTINEL_HEADER if PoW required)"
        );
    }
    Ok(json!({
        "p": "",
        "t": "",
        "c": token,
        "id": device_id,
        "flow": "authorize_continue",
    })
    .to_string())
}

async fn complete_token_exchange(
    http: &reqwest::Client,
    jar: &Jar,
    code_verifier: &str,
    expected_state: &str,
) -> Result<Map<String, Value>> {
    let auth_cookie = cookie_value(jar, "oai-client-auth-session")
        .or_else(|_| std::env::var("OAI_CLIENT_AUTH_SESSION").map_err(|_| anyhow!("missing cookie")))
        .context("missing oai-client-auth-session after login")?;
    let workspace_id = workspace_id_from_cookie(&auth_cookie)?;
    let select_resp = http
        .post(format!("{AUTH_BASE}/api/accounts/workspace/select"))
        .header("User-Agent", USER_AGENT_STR)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Referer", "https://auth.openai.com/sign-in-with-chatgpt/codex/consent")
        .json(&json!({ "workspace_id": workspace_id }))
        .send()
        .await
        .context("workspace/select")?;
    let select_status = select_resp.status();
    let select_text = select_resp.text().await.unwrap_or_default();
    if !select_status.is_success() {
        anyhow::bail!(
            "workspace/select HTTP {select_status}: {}",
            select_text.chars().take(300).collect::<String>()
        );
    }
    let select_json: Value = serde_json::from_str(&select_text).unwrap_or(json!({}));
    let continue_url = select_json
        .get("continue_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if continue_url.is_empty() {
        anyhow::bail!("workspace/select missing continue_url");
    }

    let redirect_resp = http
        .get(continue_url)
        .header("User-Agent", USER_AGENT_STR)
        .send()
        .await
        .context("follow continue_url")?;
    let callback_url = redirect_resp.url().to_string();
    let (code, returned_state) = parse_callback_code(&callback_url)?;
    if returned_state != expected_state {
        anyhow::bail!("OAuth state mismatch after redirect");
    }

    let token_resp = http
        .post(format!("{AUTH_BASE}/api/accounts/oauth/token"))
        .header("User-Agent", USER_AGENT_STR)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Origin", PLATFORM_BASE)
        .header("Referer", format!("{PLATFORM_BASE}/"))
        .header("auth0-client", PLATFORM_AUTH0_CLIENT)
        .header("sec-ch-ua", SEC_CH_UA)
        .json(&json!({
            "client_id": PLATFORM_OAUTH_CLIENT_ID,
            "code_verifier": code_verifier,
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": PLATFORM_OAUTH_REDIRECT_URI,
        }))
        .send()
        .await
        .context("oauth/token")?;
    let token_status = token_resp.status();
    let token_text = token_resp.text().await.unwrap_or_default();
    if !token_status.is_success() {
        anyhow::bail!(
            "token exchange HTTP {token_status}: {}",
            token_text.chars().take(300).collect::<String>()
        );
    }
    let data: Value = serde_json::from_str(&token_text).unwrap_or(json!({}));
    let mut out = Map::new();
    for key in ["access_token", "refresh_token", "id_token"] {
        if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                out.insert(key.into(), json!(v.trim()));
            }
        }
    }
    if out.get("access_token").is_none() {
        anyhow::bail!("token exchange missing access_token");
    }
    Ok(out)
}

fn cookie_value(jar: &Jar, name: &str) -> Result<String> {
    let url = Url::parse("https://auth.openai.com").context("auth url")?;
    let header = jar
        .cookies(&url)
        .and_then(|h| h.to_str().ok().map(|s| s.to_string()))
        .unwrap_or_default();
    for part in header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name {
                return Ok(v.trim().to_string());
            }
        }
    }
    Err(anyhow!("cookie {name} not found"))
}

fn workspace_id_from_cookie(cookie: &str) -> Result<String> {
    let part = cookie.split('.').next().unwrap_or(cookie);
    let padded = match part.len() % 4 {
        0 => part.to_string(),
        n => format!("{}{}", part, "=".repeat(4 - n)),
    };
    let normalized = padded.replace('-', "+").replace('_', "/");
    let raw = B64_STD
        .decode(normalized)
        .or_else(|_| URL_SAFE_NO_PAD.decode(part))
        .context("decode auth session cookie")?;
    let data: Value = serde_json::from_slice(&raw).unwrap_or(json!({}));
    let id = data
        .get("workspaces")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|w| w.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if id.is_empty() {
        anyhow::bail!("workspace_id not found in auth session cookie");
    }
    Ok(id.to_string())
}

fn parse_callback_code(url: &str) -> Result<(String, String)> {
    let parsed = Url::parse(url).context("parse callback url")?;
    let mut code = String::new();
    let mut state = String::new();
    for (k, v) in parsed.query_pairs() {
        if k == "code" {
            code = v.to_string();
        } else if k == "state" {
            state = v.to_string();
        }
    }
    if code.is_empty() {
        anyhow::bail!("callback URL missing code: {url}");
    }
    Ok((code, state))
}
