//! Grok 生图直连：Pro WS + Lite HTTP SSE（对齐 scripts/grok_imagine_pro_ws_probe.py）。

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use grok_domain::ProviderError;
use regex::Regex;
use rustls::pki_types::ServerName;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, MaybeTlsStream, WebSocketStream};

use crate::direct::HttpDirectClient;
use crate::statsig::BROWSER_UA;

const IMAGINE_WS_PATH: &str = "/ws/imagine/listen";
const IMAGINE_TIMEOUT: Duration = Duration::from_secs(120);

impl HttpDirectClient {
    /// 直连生图：lite → chat SSE；pro → WS imagine/listen。
    pub(crate) async fn imagine_upstream(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<Value, ProviderError> {
        let Some(sso_token) = sso_token else {
            return Err(ProviderError::NoAvailableAccount);
        };
        let session = self.session_for(account_id);
        let cookie = Self::resolve_cookie(sso_token, session.as_ref());
        let prompt = payload
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if prompt.is_empty() {
            return Err(ProviderError::Bridge("imagine missing prompt".into()));
        }
        let n = payload.get("n").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let aspect_ratio = payload
            .get("aspect_ratio")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("1:1")
            .to_string();
        let lite = payload
            .get("model")
            .and_then(|v| v.as_str())
            .map(|m| m.contains("lite"))
            .unwrap_or(false);
        let want_b64 = payload
            .get("response_format")
            .and_then(|v| v.as_str())
            .map(|s| s == "b64_json")
            .unwrap_or(false);

        let urls = if lite {
            self.imagine_lite_sse(sso_token, &prompt, n, want_b64, account_id)
                .await?
        } else {
            self.imagine_pro_ws(&cookie, &prompt, n, &aspect_ratio, want_b64)
                .await?
        };

        if urls.is_empty() {
            return Err(ProviderError::Upstream(
                "no image data in imagine response".into(),
            ));
        }
        let data: Vec<Value> = urls
            .into_iter()
            .map(|item| {
                if want_b64 {
                    json!({ "b64_json": item })
                } else {
                    json!({ "url": item })
                }
            })
            .collect();
        Ok(json!({
            "object": "list",
            "created": chrono::Utc::now().timestamp(),
            "data": data,
        }))
    }

    async fn imagine_lite_sse(
        &self,
        sso_token: &str,
        prompt: &str,
        n: usize,
        want_b64: bool,
        account_id: Option<i64>,
    ) -> Result<Vec<String>, ProviderError> {
        let payload = json!({
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
            "enableImageGeneration": true,
            "enableImageStreaming": true,
            "enableSideBySide": true,
            "fileAttachments": [],
            "forceConcise": false,
            "forceSideBySide": false,
            "imageAttachments": [],
            "imageGenerationCount": n.max(1).min(4),
            "isAsyncChat": false,
            "message": prompt,
            "modeId": "fast",
            "responseMetadata": {},
            "returnImageBytes": false,
            "returnRawGrokInXaiRequest": false,
            "sendFinalMetadata": true,
            "temporary": true,
        });
        let body = self
            .fetch_chat_raw_body(
                "/rest/app-chat/conversations/new",
                &payload,
                Some(sso_token),
                account_id,
            )
            .await?;
        let body_text = String::from_utf8_lossy(&body);
        let mut urls = extract_image_urls(&body_text);
        urls.truncate(n.max(1));
        let session = self.session_for(account_id);
        let cookie = Self::resolve_cookie(sso_token, session.as_ref());
        if want_b64 {
            urls = self.urls_to_b64(&cookie, urls).await?;
        }
        Ok(urls)
    }

    async fn imagine_pro_ws(
        &self,
        cookie: &str,
        prompt: &str,
        n: usize,
        aspect_ratio: &str,
        want_b64: bool,
    ) -> Result<Vec<String>, ProviderError> {
        let base = self.cfg.base_url.trim_end_matches('/');
        let ws_url = format!(
            "wss://{}{}",
            base.strip_prefix("https://")
                .or_else(|| base.strip_prefix("http://"))
                .unwrap_or(base),
            IMAGINE_WS_PATH
        );
        let proxy_url = self.cfg.proxy.proxy_url_for(cookie);
        let mut ws = connect_imagine_ws(&ws_url, cookie, proxy_url).await?;

        ws.send(Message::Text(reset_ws_msg().into()))
            .await
            .map_err(ws_err)?;
        ws.send(Message::Text(
            request_ws_msg(prompt, aspect_ratio, n.max(1).min(4)).into(),
        ))
        .await
        .map_err(ws_err)?;

        let deadline = tokio::time::Instant::now() + IMAGINE_TIMEOUT;
        let mut urls = Vec::new();
        while tokio::time::Instant::now() < deadline && urls.len() < n.max(1) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = match tokio::time::timeout(remaining, ws.next()).await {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(e))) => return Err(ws_err(e)),
                Ok(None) => break,
                Err(_) => break,
            };
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                Message::Close(_) => break,
                _ => continue,
            };
            if text.contains("failed") || text.contains("\"error\"") {
                if urls.is_empty() {
                    return Err(ProviderError::Upstream(truncate_err(&text)));
                }
                break;
            }
            for u in extract_image_urls(&text) {
                if !urls.iter().any(|x| x == &u) {
                    urls.push(u);
                }
            }
            if text.contains("completed") && !urls.is_empty() {
                break;
            }
        }
        urls.truncate(n.max(1));
        if want_b64 {
            urls = self.urls_to_b64(cookie, urls).await?;
        }
        Ok(urls)
    }

    async fn urls_to_b64(
        &self,
        cookie: &str,
        urls: Vec<String>,
    ) -> Result<Vec<String>, ProviderError> {
        let client = self.client_for(cookie);
        let mut out = Vec::with_capacity(urls.len());
        for url in urls {
            if url.starts_with("data:") {
                if let Some(b64) = url.split(',').nth(1) {
                    out.push(b64.to_string());
                    continue;
                }
            }
            let resp = client
                .get(&url)
                .header("Cookie", cookie)
                .header("User-Agent", BROWSER_UA)
                .send()
                .await
                .map_err(crate::proxy::proxy_err)?;
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
            out.push(base64::engine::general_purpose::STANDARD.encode(bytes));
        }
        Ok(out)
    }
}

fn ws_err(e: impl std::fmt::Display) -> ProviderError {
    ProviderError::Bridge(format!("imagine ws: {e}"))
}

fn truncate_err(text: &str) -> String {
    text.chars().take(500).collect()
}

fn new_request_id() -> String {
    format!(
        "img_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

fn reset_ws_msg() -> String {
    json!({
        "type": "conversation.item.create",
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "item": { "type": "message", "content": [{ "type": "reset" }] },
    })
    .to_string()
}

fn request_ws_msg(prompt: &str, aspect_ratio: &str, generations: usize) -> String {
    json!({
        "type": "conversation.item.create",
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "item": {
            "type": "message",
            "content": [{
                "requestId": new_request_id(),
                "text": prompt,
                "type": "input_text",
                "properties": {
                    "section_count": 0,
                    "is_kids_mode": false,
                    "enable_nsfw": false,
                    "skip_upsampler": false,
                    "enable_side_by_side": true,
                    "is_initial": false,
                    "aspect_ratio": aspect_ratio,
                    "enable_pro": true,
                    "num_generations": generations,
                },
            }],
        },
    })
    .to_string()
}

pub fn extract_image_urls(body: &str) -> Vec<String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#""imageUrl"\s*:\s*"([^"]+)""#).expect("re"));
    re.captures_iter(body)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

async fn connect_imagine_ws(
    ws_url: &str,
    cookie: &str,
    proxy_url: Option<&str>,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, ProviderError> {
    let mut request = ws_url
        .into_client_request()
        .map_err(|e| ProviderError::Bridge(format!("ws request: {e}")))?;
    let headers = request.headers_mut();
    headers.insert(
        "Cookie",
        cookie
            .parse()
            .map_err(|e| ProviderError::Bridge(format!("cookie: {e}")))?,
    );
    headers.insert("Origin", "https://grok.com".parse().unwrap());
    headers.insert("User-Agent", BROWSER_UA.parse().unwrap());
    headers.insert(
        "Accept-Language",
        "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap(),
    );

    if proxy_url.is_none() {
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| ProviderError::Bridge(format!("ws connect: {e}")))?;
        return Ok(ws);
    }

    let parsed = url::Url::parse(ws_url)
        .map_err(|e| ProviderError::Bridge(format!("ws url: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| ProviderError::Bridge("ws missing host".into()))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let tcp = connect_via_http_proxy(proxy_url.unwrap(), host, port).await?;

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| ProviderError::Bridge(format!("ws sni: {e}")))?;
    let tls = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| ProviderError::Bridge(format!("ws tls: {e}")))?;

    let (ws, _) = client_async(request, MaybeTlsStream::Rustls(tls))
        .await
        .map_err(|e| ProviderError::Bridge(format!("ws handshake: {e}")))?;
    Ok(ws)
}

async fn connect_via_http_proxy(
    proxy_url: &str,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream, ProviderError> {
    let proxy = url::Url::parse(proxy_url)
        .map_err(|e| ProviderError::Bridge(format!("proxy url: {e}")))?;
    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| ProviderError::Bridge("proxy missing host".into()))?;
    let proxy_port = proxy.port().unwrap_or(8080);

    let mut stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(|e| ProviderError::Bridge(format!("proxy tcp: {e}")))?;

    let mut req = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n"
    );
    if !proxy.username().is_empty() {
        let user = proxy.username();
        let pass = proxy.password().unwrap_or("");
        let cred = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        req.push_str(&format!("Proxy-Authorization: Basic {cred}\r\n"));
    }
    req.push_str("\r\n");
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| ProviderError::Bridge(format!("proxy connect write: {e}")))?;
    let mut buf = [0u8; 1024];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| ProviderError::Bridge(format!("proxy connect read: {e}")))?;
    let resp = String::from_utf8_lossy(&buf[..n]);
    if !(resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200")) {
        return Err(ProviderError::Bridge(format!(
            "proxy CONNECT failed: {}",
            resp.lines().next().unwrap_or("unknown")
        )));
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_image_urls_finds_multiple() {
        let body = r#"{"imageUrl":"https://x/a.jpg"}{"imageUrl":"https://x/b.jpg"}"#;
        let urls = extract_image_urls(body);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://x/a.jpg");
    }

    #[test]
    fn request_ws_msg_includes_aspect_ratio() {
        let msg = request_ws_msg("cat", "16:9", 2);
        assert!(msg.contains(r#""aspect_ratio":"16:9""#));
        assert!(msg.contains(r#""num_generations":2"#));
    }
}
