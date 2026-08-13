//! HttpDirectClient — 无 chrome 直连 grok.com（对齐 Go 无桥路径）。
//!
//! BridgeClient 的第二个实现：不需要浏览器。认证用账号 sso token
//! （`Cookie: sso=<t>; sso-rw=<t>`），`x-statsig-id` 由外部 signer 服务签名。
//!
//! - chat/OCR：POST `{base}/rest/app-chat/conversations/new`，解析 SSE 文本增量。
//! - 生图：WS 直连 `wss://{host}/ws/imagine/listen` 收帧（对齐 Go image.go）；
//!   WS 帧二进制格式与上游耦合，需真实上游联调（残余风险，见 docs/39f）。
//!
//! 安全红线：客户端只组装带 cookie 的请求；调用方（grok2api-rs）负责在
//! token/密钥缺失时返回 503 绝不外呼。

use std::sync::Arc;

use grok_domain::{ProviderError, SsoTokenProvider};
use serde_json::{json, Value};

use crate::bridge::BridgeClient;
use crate::proxy::{proxy_err, ProxyPool};
use crate::signer::{build_signer, SignerMode, SignerTrait};
use crate::statsig::BROWSER_UA;

/// 上游总超时（对齐 provider「connect 5s / total 60s」红线语义）。
const TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// SSE 分块读取上限（防御失控响应）。
const MAX_BITSTREAM_READ_LIMIT: usize = 64 << 20;

/// 直连客户端配置。
#[derive(Clone)]
pub struct DirectConfig {
    /// grok.com 基地址（缺省 https://grok.com）。
    pub base_url: String,
    /// statsig signer 服务地址（缺省公网 grok.wodf.de/sign）。
    pub signer_url: String,
    /// 签名器模式（缺省 remote，外部 signer 服务）。
    pub signer_mode: SignerMode,
    /// 直连需要 sso token；`sso` 提供按账号取 token。
    pub sso: Option<Arc<dyn SsoTokenProvider>>,
    /// 住宅代理池（webshare 等）；空池 = 直连。
    pub proxy: Arc<ProxyPool>,
    /// 本地出口代理（GROK_LOCAL_PROXY，如 Clash 127.0.0.1:7897）：
    /// meta 抓取/签名/直连请求走它（公网直连常被墙或 CF 拦）。
    pub local_proxy: Option<String>,
    /// Playwright 提取的 session 签名材料；有则优先于 signer_mode（Python statsig 路径）。
    pub session: Option<crate::signer::SessionKeys>,
    /// 按账号 id 加载 `pure_http_keys`（`GROK_PURE_HTTP_KEYS_DIR`）。
    pub session_store: Option<Arc<crate::session_store::SessionKeyStore>>,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            base_url: "https://grok.com".to_string(),
            signer_url: "https://grok.wodf.de/sign".to_string(),
            signer_mode: SignerMode::default(),
            sso: None,
            proxy: Arc::new(ProxyPool::empty()),
            local_proxy: None,
            session: None,
            session_store: None,
        }
    }
}

/// 直连客户端。
pub struct HttpDirectClient {
    client: reqwest::Client,
    signer: Box<dyn SignerTrait>,
    pub(crate) cfg: DirectConfig,
}

impl HttpDirectClient {
    /// 构造直连客户端。
    pub fn new(cfg: DirectConfig) -> Self {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .danger_accept_invalid_certs(false);
        // 本地出口代理（GROK_LOCAL_PROXY）：meta/签名/直连走它（公网直连常被墙/CF 拦）。
        if let Some(local_proxy) = cfg.local_proxy.as_deref() {
            if !local_proxy.trim().is_empty() {
                builder = builder
                    .proxy(reqwest::Proxy::all(local_proxy).expect("invalid GROK_LOCAL_PROXY"));
            }
        }
        let client = builder.build().expect("reqwest client");
        let signer: Box<dyn SignerTrait> = if let Some(session) = cfg.session.clone() {
            Box::new(crate::signer::SessionSigner::new(session))
        } else {
            build_signer(cfg.signer_mode, &cfg.signer_url)
        };
        Self {
            client,
            signer,
            cfg,
        }
    }

    /// 组装直连请求头（对齐 Go `headers.go` + statsig）。
    fn build_headers(
        &self,
        sso_cookie: &str,
        statsig_id: &str,
        content_type: &str,
    ) -> reqwest::header::HeaderMap {
        use reqwest::header::*;
        let mut map = HeaderMap::new();
        map.insert(
            CONTENT_TYPE,
            HeaderValue::from_str(content_type)
                .unwrap_or(HeaderValue::from_static("application/json")),
        );
        map.insert(ACCEPT, HeaderValue::from_static("*/*"));
        map.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        let origin = self.cfg.base_url.trim_end_matches('/').to_string();
        map.insert(
            ORIGIN,
            HeaderValue::from_str(&origin)
                .unwrap_or_else(|_| HeaderValue::from_static("https://grok.com")),
        );
        map.insert(
            REFERER,
            HeaderValue::from_str(&format!("{origin}/"))
                .unwrap_or_else(|_| HeaderValue::from_static("https://grok.com/")),
        );
        map.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        map.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        map.insert(
            HeaderName::from_static("priority"),
            HeaderValue::from_static("u=1, i"),
        );
        map.insert(
            HeaderName::from_static("sec-fetch-dest"),
            HeaderValue::from_static("empty"),
        );
        map.insert(
            HeaderName::from_static("sec-fetch-mode"),
            HeaderValue::from_static("cors"),
        );
        map.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );
        map.insert(USER_AGENT, HeaderValue::from_static(BROWSER_UA));
        if let Ok(req_id) = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()) {
            map.insert(HeaderName::from_static("x-xai-request-id"), req_id);
        }
        if !sso_cookie.is_empty() {
            if let Ok(cookie_value) = HeaderValue::from_str(sso_cookie) {
                map.insert(COOKIE, cookie_value);
            }
        }
        if !statsig_id.is_empty() {
            if let Ok(id_value) = HeaderValue::from_str(statsig_id) {
                map.insert(HeaderName::from_static("x-statsig-id"), id_value);
            }
        }
        map
    }

    /// 解析账号 session keys：全局 cfg.session 或按 id 从 store 加载。
    pub(crate) fn session_for(
        &self,
        account_id: Option<i64>,
    ) -> Option<crate::signer::SessionKeys> {
        if let Some(id) = account_id {
            if let Some(store) = &self.cfg.session_store {
                if let Some(keys) = store.get(id) {
                    return Some((*keys).clone());
                }
            }
        }
        self.cfg.session.clone()
    }

    /// cookie：有 cf_clearance 的 session cookie 优先，否则 sso 拼接。
    pub(crate) fn resolve_cookie(
        sso_token: &str,
        session: Option<&crate::signer::SessionKeys>,
    ) -> String {
        if let Some(sk) = session {
            if let Some(ref full) = sk.cookie {
                if full.contains("cf_clearance=") {
                    return full.clone();
                }
            }
        }
        Self::build_sso_cookie(sso_token)
    }

    /// 对目标 path 签名（method + path → x-statsig-id）。
    async fn sign_path(
        &self,
        sso_cookie: &str,
        method: &str,
        path: &str,
        session: Option<&crate::signer::SessionKeys>,
    ) -> Result<String, ProviderError> {
        if let Some(sk) = session.filter(|s| !s.fingerprint.is_empty()) {
            return crate::signer::SessionSigner::new(sk.clone())
                .sign(
                    &self.client,
                    &self.cfg.base_url,
                    &self.cfg.signer_url,
                    sso_cookie,
                    method,
                    path,
                )
                .await;
        }
        // remote signer 需先抓 grok.com 首页 meta；Panda 机房 IP 常被 CF 403，
        // 有住宅代理时 meta/签名也走账号出口（与上游 API 一致）。
        let sign_client = self
            .cfg
            .proxy
            .client_for(sso_cookie)
            .unwrap_or(&self.client);
        self.signer
            .sign(
                sign_client,
                &self.cfg.base_url,
                &self.cfg.signer_url,
                sso_cookie,
                method,
                path,
            )
            .await
            .map_err(|e| {
                tracing::warn!(signer = %self.cfg.signer_url, "statsig sign failed: {e}");
                e
            })
    }

    /// 从 cookie 头构造（sso=<t> 原始 token 直接拼接）。
    pub(crate) fn build_sso_cookie(token: &str) -> String {
        let token = token.trim();
        let token = token.strip_prefix("sso=").map(str::trim).unwrap_or(token);
        let token = token.split(';').next().unwrap_or("").trim();
        // 清理危险字符（防 http 头注入）。
        let sanitized: String = token
            .chars()
            .filter(|c| *c != '\r' && *c != '\n' && *c != '\0')
            .collect();
        format!("sso={sanitized}; sso-rw={sanitized}")
    }

    /// 按 cookie（=账号身份）稳定取出口 client：有代理池 → 哈希映射；否则直连。
    pub(crate) fn client_for(&self, sso_cookie: &str) -> &reqwest::Client {
        self.cfg
            .proxy
            .client_for(sso_cookie)
            .unwrap_or(&self.client)
    }

    /// 上传 payload 中的每张附件（对齐 Go `uploadImage`：JSON body，非 multipart），
    /// 返回 fileMetadataId 数组。上传失败 → 整体失败（调用方冷却该账号）。
    async fn upload_attachments(
        &self,
        cookie: &str,
        client: &reqwest::Client,
        payload: &Value,
        session: Option<&crate::signer::SessionKeys>,
    ) -> Result<Vec<String>, ProviderError> {
        let Some(list) = payload.get("fileAttachments").and_then(|v| v.as_array()) else {
            return Ok(Vec::new());
        };
        let mut ids = Vec::with_capacity(list.len());
        for att in list {
            let file_name = att
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("attachment.png");
            let mime = att
                .get("mime_type")
                .and_then(|v| v.as_str())
                .unwrap_or("image/png");
            let content = att
                .get("data_base64")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ProviderError::Bridge("attachment missing data_base64".into()))?;
            let id = self
                .upload_one(cookie, client, file_name, mime, content, session)
                .await?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// 单张上传（POST /rest/app-chat/upload-file → fileMetadataId/fileId）。
    async fn upload_one(
        &self,
        cookie: &str,
        client: &reqwest::Client,
        file_name: &str,
        mime: &str,
        content_b64: &str,
        session: Option<&crate::signer::SessionKeys>,
    ) -> Result<String, ProviderError> {
        let path = "/rest/app-chat/upload-file";
        let statsig_id = self.sign_path(cookie, "POST", path, session).await?;
        let endpoint = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        let body = json!({
            "fileName": file_name,
            "fileMimeType": mime,
            "content": content_b64,
        });
        let resp = client
            .post(&endpoint)
            .headers(self.build_headers(cookie, &statsig_id, "application/json"))
            .json(&body)
            .send()
            .await
            .map_err(proxy_err)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Upstream(format!("upload status {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("upload body: {e}")))?;
        if bytes.len() > MAX_BITSTREAM_READ_LIMIT {
            return Err(ProviderError::Upstream("upload response over limit".into()));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::Upstream("upload response invalid".into()))?;
        let id = value
            .get("fileMetadataId")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("fileId").and_then(|v| v.as_str()))
            .ok_or_else(|| ProviderError::Upstream("upload missing fileMetadataId".into()))?;
        if id.is_empty() {
            return Err(ProviderError::Upstream(
                "upload missing fileMetadataId".into(),
            ));
        }
        Ok(id.to_string())
    }

    /// POST 指定 chat 路径，返回 SSE 解析结果（含 conversation/response id）。
    pub async fn fetch_chat_turn(
        &self,
        path: &str,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<SseChatParse, ProviderError> {
        let Some(sso_token) = sso_token else {
            return Err(ProviderError::NoAvailableAccount);
        };
        let session = self.session_for(account_id);
        let cookie = Self::resolve_cookie(sso_token, session.as_ref());
        let client = self.client_for(&cookie);
        let mut owned = payload.clone();
        if has_uploadable_attachments(payload) {
            let ids = self
                .upload_attachments(&cookie, client, payload, session.as_ref())
                .await?;
            owned["fileAttachments"] = json!(ids);
        }
        let statsig_id = self
            .sign_path(&cookie, "POST", path, session.as_ref())
            .await?;
        let endpoint = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        let resp = client
            .post(&endpoint)
            .headers(self.build_headers(&cookie, &statsig_id, "application/json"))
            .json(&owned)
            .send()
            .await
            .map_err(proxy_err)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Upstream(format!("chat status {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("chat body: {e}")))?;
        if bytes.len() > MAX_BITSTREAM_READ_LIMIT {
            return Err(ProviderError::Upstream("chat stream over limit".into()));
        }
        parse_sse_chat(&bytes)
    }

    /// POST `/rest/rate-limits` 拉取对话额度（对齐 Python `grok_quota_probe.probe_rate_limits`）。
    pub async fn fetch_rate_limits(
        &self,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<Value, ProviderError> {
        let Some(sso_token) = sso_token else {
            return Err(ProviderError::NoAvailableAccount);
        };
        let path = "/rest/rate-limits";
        let session = self.session_for(account_id);
        let cookie = Self::resolve_cookie(sso_token, session.as_ref());
        let client = self.client_for(&cookie);
        let statsig_id = self
            .sign_path(&cookie, "POST", path, session.as_ref())
            .await?;
        let endpoint = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        for body in [json!({}), json!({"modelName": "grok-3"})] {
            let resp = client
                .post(&endpoint)
                .headers(self.build_headers(&cookie, &statsig_id, "application/json"))
                .json(&body)
                .send()
                .await
                .map_err(proxy_err)?;
            let status = resp.status();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ProviderError::Bridge(format!("rate-limits body: {e}")))?;
            if !status.is_success() {
                continue;
            }
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| ProviderError::Upstream("rate-limits invalid json".into()))?;
            if value.get("remainingQueries").is_some() || value.get("totalQueries").is_some() {
                return Ok(value);
            }
        }
        Err(ProviderError::Upstream(
            "rate-limits empty or failed".into(),
        ))
    }

    /// POST chat 路径并返回原始响应体（lite 生图从 SSE 提取 imageUrl）。
    pub(crate) async fn fetch_chat_raw_body(
        &self,
        path: &str,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<Vec<u8>, ProviderError> {
        let Some(sso_token) = sso_token else {
            return Err(ProviderError::NoAvailableAccount);
        };
        let session = self.session_for(account_id);
        let cookie = Self::resolve_cookie(sso_token, session.as_ref());
        let client = self.client_for(&cookie);
        let mut owned = payload.clone();
        if has_uploadable_attachments(payload) {
            let ids = self
                .upload_attachments(&cookie, client, payload, session.as_ref())
                .await?;
            owned["fileAttachments"] = json!(ids);
        }
        let statsig_id = self
            .sign_path(&cookie, "POST", path, session.as_ref())
            .await?;
        let endpoint = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        let resp = client
            .post(&endpoint)
            .headers(self.build_headers(&cookie, &statsig_id, "application/json"))
            .json(&owned)
            .send()
            .await
            .map_err(proxy_err)?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Upstream(
                "invalid session (sso token expired or revoked)".into(),
            ));
        }
        if !status.is_success() {
            return Err(ProviderError::Upstream(format!("chat status {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("chat body: {e}")))?;
        if bytes.len() > MAX_BITSTREAM_READ_LIMIT {
            return Err(ProviderError::Upstream("chat stream over limit".into()));
        }
        Ok(bytes.to_vec())
    }

    /// 多轮 followup：POST `/rest/app-chat/conversations/{id}/responses`。
    pub async fn fetch_chat_followup(
        &self,
        conversation_id: &str,
        parent_response_id: &str,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<SseChatParse, ProviderError> {
        let mut owned = payload.clone();
        owned["responseId"] = json!(parent_response_id);
        let path = format!("/rest/app-chat/conversations/{conversation_id}/responses");
        self.fetch_chat_turn(&path, &owned, sso_token, account_id)
            .await
    }

    /// 单张上传（供探针/gate 直接调用）。
    pub async fn upload_file_b64(
        &self,
        file_name: &str,
        mime: &str,
        content_b64: &str,
        sso_token: &str,
    ) -> Result<String, ProviderError> {
        let session = self.cfg.session.clone();
        let cookie = Self::resolve_cookie(sso_token, session.as_ref());
        let client = self.client_for(&cookie);
        self.upload_one(
            &cookie,
            client,
            file_name,
            mime,
            content_b64,
            session.as_ref(),
        )
        .await
    }
}

/// payload.fileAttachments 是否是需要先上传的对象数组（含 data_base64）。
fn has_uploadable_attachments(payload: &Value) -> bool {
    payload
        .get("fileAttachments")
        .and_then(|v| v.as_array())
        .map(|list| {
            !list.is_empty()
                && list
                    .first()
                    .map(|a| a.get("data_base64").is_some())
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// SSE 解析结果（含多轮 followup 所需的 id）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseChatParse {
    pub text: String,
    pub conversation_id: Option<String>,
    pub response_id: Option<String>,
    pub parent_response_id: Option<String>,
}

/// 解析 grok chat SSE：兼容 `data:` 前缀与裸 JSON 行；嵌套/扁平 `result` 结构。
pub fn parse_sse_chat(body: &[u8]) -> Result<SseChatParse, ProviderError> {
    let text = String::from_utf8_lossy(body);
    let mut out = SseChatParse::default();
    let mut saw_line = false;
    let mut text_parts: Vec<String> = Vec::new();

    for line in text.lines() {
        let mut line = line.trim();
        if line.is_empty() || line == "[DONE]" {
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            line = data.trim();
        }
        saw_line = true;
        let Ok(root) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(err) = root.get("error") {
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("upstream error");
            return Err(ProviderError::Upstream(msg.to_string()));
        }
        let Some(res) = root.get("result").and_then(|v| v.as_object()) else {
            continue;
        };
        if let Some(conv) = res.get("conversation").and_then(|v| v.as_object()) {
            if let Some(id) = conv.get("conversationId").and_then(|v| v.as_str()) {
                out.conversation_id = Some(id.to_string());
            }
        }
        let blocks: Vec<&serde_json::Map<String, Value>> = res
            .get("response")
            .and_then(|v| v.as_object())
            .map(|m| vec![m])
            .unwrap_or_else(|| vec![res]);
        for block in blocks {
            if let Some(id) = block.get("responseId").and_then(|v| v.as_str()) {
                out.response_id = Some(id.to_string());
            }
            if let Some(mr) = block.get("modelResponse").and_then(|v| v.as_object()) {
                if let Some(id) = mr.get("responseId").and_then(|v| v.as_str()) {
                    out.response_id = Some(id.to_string());
                }
                if let Some(msg) = mr.get("message").and_then(|v| v.as_str()) {
                    text_parts = vec![msg.to_string()];
                }
                if let Some(pid) = mr.get("parentResponseId").and_then(|v| v.as_str()) {
                    out.parent_response_id = Some(pid.to_string());
                }
            }
            if let Some(ur) = block.get("userResponse").and_then(|v| v.as_object()) {
                if let Some(pid) = ur.get("responseId").and_then(|v| v.as_str()) {
                    out.parent_response_id = Some(pid.to_string());
                }
            }
            let thinking = block
                .get("isThinking")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tag = block
                .get("messageTag")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(tok) = block.get("token").and_then(|v| v.as_str()) {
                if !thinking && matches!(tag, "final" | "response_start" | "") && !tok.is_empty() {
                    if text_parts.is_empty() || text_parts.last().map(|s| s.as_str()) != Some(tok) {
                        text_parts.push(tok.to_string());
                    }
                }
            }
        }
    }

    if !saw_line {
        if let Ok(root) = serde_json::from_str::<Value>(&text) {
            if let Some(msg) = root
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
            {
                return Err(ProviderError::Upstream(msg.to_string()));
            }
            if let Some(content) = root
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
            {
                out.text = content.to_string();
                return Ok(out);
            }
        }
    }

    out.text = text_parts.join("");
    if out.text.is_empty() && !saw_line {
        return Err(ProviderError::Upstream("empty chat response".into()));
    }
    Ok(out)
}

/// 快速解析 SSE 文本（仅正文，兼容旧调用方）。
#[allow(dead_code)]
fn parse_sse_text(body: &[u8]) -> Result<String, ProviderError> {
    parse_sse_chat(body).map(|p| p.text)
}

#[async_trait::async_trait]
impl BridgeClient for HttpDirectClient {
    async fn fetch_bytes(
        &self,
        image_url: &str,
        _sso_token: Option<&str>,
    ) -> Result<Vec<u8>, ProviderError> {
        // data URI 本地直解
        if let Some(payload) = crate::bridge::decode_data_uri_public(image_url) {
            return Ok(payload);
        }
        let resp = self
            .client
            .get(image_url)
            .send()
            .await
            .map_err(|e| ProviderError::Bridge(format!("direct fetch bytes: {e}")))?;
        if !resp.status().is_success() {
            return Err(ProviderError::Upstream(format!(
                "image fetch status {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("image body: {e}")))?;
        Ok(bytes.to_vec())
    }

    async fn fetch_chat(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<String, ProviderError> {
        self.fetch_chat_turn(
            "/rest/app-chat/conversations/new",
            payload,
            sso_token,
            account_id,
        )
        .await
        .map(|p| p.text)
    }

    async fn fetch_imagine(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<Value, ProviderError> {
        self.imagine_upstream(payload, sso_token, account_id).await
    }

    fn has_pure_http_keys(&self, account_id: i64) -> bool {
        match &self.cfg.session_store {
            Some(store) => store.has(account_id),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_followup_flat_result() {
        let body = r#"
{"result":{"userResponse":{"responseId":"u1","message":"hi"}}}
{"result":{"token":"P","isThinking":false,"messageTag":"final","responseId":"r1"}}
{"result":{"token":"ONG","isThinking":false,"messageTag":"final","responseId":"r1"}}
{"result":{"modelResponse":{"responseId":"r1","message":"PONG","parentResponseId":"u1"}}}
"#;
        let out = parse_sse_chat(body.as_bytes()).unwrap();
        assert_eq!(out.text, "PONG");
        assert_eq!(out.response_id.as_deref(), Some("r1"));
        assert_eq!(out.parent_response_id.as_deref(), Some("u1"));
    }

    #[test]
    fn parse_sse_extracts_tokens() {
        let body = r#"
data: {"result":{"conversation":{"conversationId":"c1"}}}
data: {"result":{"response":{"token":"你","isThinking":false,"messageTag":"final"}}}
data: {"result":{"response":{"token":"好","isThinking":false,"messageTag":"final"}}}
data: [DONE]
"#;
        let out = parse_sse_text(body.as_bytes()).unwrap();
        assert_eq!(out, "你好");
    }

    #[test]
    fn parse_sse_ignores_thinking_and_empty() {
        let body = r#"
data: {"result":{"response":{"token":"思考中","isThinking":true}}}
data: {"result":{"response":{"token":"答","isThinking":false,"messageTag":""}}}
"#;
        let out = parse_sse_text(body.as_bytes()).unwrap();
        assert_eq!(out, "答");
    }

    #[test]
    fn parse_sse_surfaces_errors() {
        let body = r#"data: {"error":{"message":"boom"}}
"#;
        let err = parse_sse_text(body.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn sso_cookie_build_is_sanitized() {
        assert_eq!(
            HttpDirectClient::build_sso_cookie("sso=abc"),
            "sso=abc; sso-rw=abc"
        );
        assert_eq!(
            HttpDirectClient::build_sso_cookie("abc; extra=1"),
            "sso=abc; sso-rw=abc"
        );
        assert_eq!(
            HttpDirectClient::build_sso_cookie("a\r\nb"),
            "sso=ab; sso-rw=ab"
        );
    }

    #[tokio::test]
    async fn fetch_chat_without_token_is_503_semantics() {
        let client = HttpDirectClient::new(DirectConfig::default());
        let err = client
            .fetch_chat(&serde_json::json!({"model":"x"}), None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NoAvailableAccount));
    }

    #[tokio::test]
    async fn no_upstream_call_when_token_missing() {
        // 用会失败的 signer url 验证：token 缺失时根本不发上游请求（直接短路）。
        let cfg = DirectConfig {
            base_url: "https://grok.com".to_string(),
            signer_url: "https://127.0.0.1:1/sign".to_string(),
            signer_mode: SignerMode::default(),
            sso: None,
            proxy: Arc::new(crate::proxy::ProxyPool::empty()),
            local_proxy: None,
            session: None,
            session_store: None,
        };
        let client = HttpDirectClient::new(cfg);
        let err = client
            .fetch_chat(&serde_json::json!({"model":"x"}), None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NoAvailableAccount));
    }

    // ── 全链路：图片上传 → fileAttachments 替换 → chat SSE ──────────

    struct FakeGrok {
        uploaded_ids: std::sync::Mutex<Vec<String>>,
        chat_bodies: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    async fn spawn_fake_grok() -> (
        String,
        std::sync::Arc<FakeGrok>,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::{routing::get, Router};
        let shared = std::sync::Arc::new(FakeGrok {
            uploaded_ids: std::sync::Mutex::new(Vec::new()),
            chat_bodies: std::sync::Mutex::new(Vec::new()),
        });
        let uploaded = std::sync::Arc::clone(&shared);
        let bodies = std::sync::Arc::clone(&shared);
        let app = Router::new()
            .route("/", get(|| async { "<html>grok home</html>" }))
            .route(
                "/sign",
                axum::routing::post(|_: axum::Json<serde_json::Value>| async move {
                    axum::Json(serde_json::json!({ "x-statsig-id": "fake-id" }))
                }),
            )
            .route(
                "/rest/app-chat/upload-file",
                axum::routing::post(|payload: axum::Json<serde_json::Value>| async move {
                    let id = format!("file-{}", payload["fileName"].as_str().unwrap_or("x"));
                    uploaded.uploaded_ids.lock().unwrap().push(id.clone());
                    axum::Json(serde_json::json!({ "fileMetadataId": id }))
                }),
            )
            .route(
                "/rest/app-chat/conversations/new",
                axum::routing::post(|payload: axum::Json<serde_json::Value>| async move {
                    bodies.chat_bodies.lock().unwrap().push(payload.0.clone());
                    // SSE：两段 token + DONE（裸文本，非 JSON）
                    String::from(
                        "data: {\"result\":{\"response\":{\"token\":\"图\",\"isThinking\":false}}}
data: {\"result\":{\"response\":{\"token\":\"片\",\"isThinking\":false}}}
data: [DONE]
",
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (addr.to_string(), shared, handle)
    }

    #[tokio::test]
    async fn chat_with_attachment_uploads_then_replaces_file_attachments() {
        let (addr, fake, _guard) = spawn_fake_grok().await;
        let cfg = DirectConfig {
            base_url: format!("http://{addr}"),
            signer_url: format!("http://{addr}/sign"),
            signer_mode: SignerMode::default(),
            sso: None,
            proxy: Arc::new(crate::proxy::ProxyPool::empty()),
            local_proxy: None,
            session: None,
            session_store: None,
        };
        let client = HttpDirectClient::new(cfg);
        let payload = serde_json::json!({
            "model": "grok-chat-fast",
            "enableImageGeneration": false,
            "fileAttachments": [{
                "source_url": "data:image/png;base64,aGVsbG8=",
                "file_name": "attachment_1.png",
                "mime_type": "image/png",
                "data_base64": "aGVsbG8=",
            }]
        });
        let text = client
            .fetch_chat(&payload, Some("sso-token-1"), None)
            .await
            .unwrap();
        assert_eq!(text, "图片");
        // 上传发生了一次且 fileAttachments 已替换为 fileMetadataId 数组
        let ids = fake.uploaded_ids.lock().unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "file-attachment_1.png");
        let bodies = fake.chat_bodies.lock().unwrap();
        let sent = &bodies[0];
        assert_eq!(sent["fileAttachments"][0], "file-attachment_1.png");
        // 上传后的 payload 不再含 data_base64
        assert!(sent["fileAttachments"][0].get("data_base64").is_none());
    }

    #[tokio::test]
    async fn chat_without_attachments_skips_upload() {
        let (addr, fake, _guard) = spawn_fake_grok().await;
        let cfg = DirectConfig {
            base_url: format!("http://{addr}"),
            signer_url: format!("http://{addr}/sign"),
            signer_mode: SignerMode::default(),
            sso: None,
            proxy: Arc::new(crate::proxy::ProxyPool::empty()),
            local_proxy: None,
            session: None,
            session_store: None,
        };
        let client = HttpDirectClient::new(cfg);
        let payload = serde_json::json!({
            "model": "grok-chat-fast",
            "enableImageGeneration": false,
            "fileAttachments": [],
        });
        let text = client
            .fetch_chat(&payload, Some("sso-token-2"), None)
            .await
            .unwrap();
        assert_eq!(text, "图片");
        assert!(
            fake.uploaded_ids.lock().unwrap().is_empty(),
            "无附件不应上传"
        );
        let bodies = fake.chat_bodies.lock().unwrap();
        assert_eq!(
            bodies.last().unwrap()["fileAttachments"],
            serde_json::json!([])
        );
    }
}
