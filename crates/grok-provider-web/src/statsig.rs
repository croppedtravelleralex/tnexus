//! Statsig 签名器（无 chrome 直连路径）——对齐 Go `internal/infra/provider/web/statsig.go`。
//!
//! grok.com 上游要求 `x-statsig-id` 请求头（前端 Turbopack 签名器产物）。无浏览器方案：
//! - 抓 grok.com 首页 HTML 作为 `metaContent`（缓存 1h）。
//! - POST 外部 signer 服务 `{method, path, environment:{metaContent}}` → `{x-statsig-id}`。
//!
//! signer URL 安全校验对齐 Go `internal/pkg/signerurl/policy.go`（HTTPS:443 或内网 HTTP）。

use std::collections::HashMap;

/// 浏览器 UA（CF 拦截判定：无 UA / 通用 UA 会被重置连接）。
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
use std::sync::Mutex;
use std::time::{Duration, Instant};

use grok_domain::ProviderError;
use reqwest::Client;
use serde_json::json;

const META_CACHE_TTL: Duration = Duration::from_secs(3600);
const META_BODY_LIMIT: usize = 4 << 20; // 4 MiB
const SIGN_RESPONSE_LIMIT: usize = 4 << 10; // 4 KiB

/// 首页 meta 内容缓存（key = base_url \x00 signer_url）。
#[derive(Default)]
pub struct MetaCache {
    inner: Mutex<HashMap<String, (String, Instant)>>,
}

impl MetaCache {
    fn get(&self, key: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner
            .get(key)
            .filter(|(_, at)| at.elapsed() < META_CACHE_TTL)
            .map(|(v, _)| v.clone())
    }

    fn store(&self, key: &str, value: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.insert(key.to_string(), (value, Instant::now()));
    }
}

/// 签名器：缓存 + 默认签名地址（可注入，便于测试）。
/// 请求客户端由调用方每次传入（直连模式传代理 client）。
pub struct StatsigSigner {
    cache: MetaCache,
    default_signer_url: String,
}

impl StatsigSigner {
    /// 构造。`signer_url` 为空时使用默认公网签名服务。
    pub fn new(default_signer_url: String) -> Self {
        Self {
            cache: MetaCache::default(),
            default_signer_url,
        }
    }

    /// 为 `method + path` 签名，返回 `x-statsig-id` 值。
    /// `client` 为本次请求的出口客户端（直连模式传代理 client，meta 抓取/签名均走代理）。
    pub async fn sign(
        &self,
        client: &Client,
        base_url: &str,
        signer_url: &str,
        sso_cookie: &str,
        method: &str,
        path: &str,
    ) -> Result<String, ProviderError> {
        let signer = if signer_url.trim().is_empty() {
            self.default_signer_url.as_str()
        } else {
            signer_url
        };
        validate_signer_url(signer)?;

        let cache_key = format!(
            "{}\u{0}{}\u{0}meta",
            base_url.trim_end_matches('/'),
            signer.trim()
        );
        let meta = match self.cache.get(&cache_key) {
            Some(v) => v,
            None => {
                let fresh = fetch_meta_content(client, base_url, sso_cookie).await?;
                self.cache.store(&cache_key, fresh.clone());
                fresh
            }
        };
        request_signature(client, signer, method, path, &meta).await
    }
}

/// 抓取 grok.com 首页 HTML 作为签名 metaContent（带 cookie；失败即报错，不缓存）。
async fn fetch_meta_content(
    client: &Client,
    base_url: &str,
    sso_cookie: &str,
) -> Result<String, ProviderError> {
    let mut request = client.get(base_url);
    // 浏览器 UA + Accept：无 UA 会被 Cloudflare 直接 TCP 重置（实测 10054）。
    request = request
        .header("User-Agent", BROWSER_UA)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8");
    if !sso_cookie.is_empty() {
        request = request.header("Cookie", sso_cookie);
    }
    let resp = request
        .send()
        .await
        .map_err(|e| ProviderError::Bridge(format!("meta fetch: {e}")))?;
    if !resp.status().is_success() {
        return Err(ProviderError::Bridge(format!(
            "meta fetch status {}",
            resp.status()
        )));
    }
    let mut body = resp
        .bytes()
        .await
        .map_err(|e| ProviderError::Bridge(format!("meta body: {e}")))?;
    body.truncate(META_BODY_LIMIT);
    String::from_utf8(body.to_vec()).map_err(|e| ProviderError::Bridge(format!("meta utf8: {e}")))
}

/// POST signer 服务换取 x-statsig-id。响应 >4KiB 拒绝；非 2xx 报错。
async fn request_signature(
    client: &Client,
    signer_url: &str,
    method: &str,
    path: &str,
    meta_content: &str,
) -> Result<String, ProviderError> {
    let payload = json!({
        "method": method.to_uppercase(),
        "path": path,
        "environment": { "metaContent": meta_content },
    });
    let resp = client
        .post(signer_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| ProviderError::Bridge(format!("sign request: {e}")))?;
    if !resp.status().is_success() {
        return Err(ProviderError::Bridge(format!(
            "sign service status {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ProviderError::Bridge(format!("sign body: {e}")))?;
    if bytes.len() > SIGN_RESPONSE_LIMIT {
        return Err(ProviderError::Bridge("sign response too large".into()));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| ProviderError::Bridge(format!("sign parse: {e}")))?;
    let statsig_id = value
        .get("x-statsig-id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 512)
        .ok_or_else(|| ProviderError::Bridge("sign response invalid".into()))?;
    Ok(statsig_id.to_string())
}

/// signer URL 安全校验（对齐 Go `signerurl.Validate` 核心规则）。
/// 公网仅 HTTPS:443；HTTP/自定义端口仅允许可信内网（localhost/.local/.internal/私有 IP/单标签服务名）。
pub fn validate_signer_url(raw: &str) -> Result<(), ProviderError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2048 {
        return Err(ProviderError::Bridge("signer URL invalid".into()));
    }
    let parsed = url::Url::parse(raw)
        .map_err(|_| ProviderError::Bridge("signer URL parse failed".into()))?;
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(ProviderError::Bridge(
            "signer URL must not embed credentials".into(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ProviderError::Bridge(
            "signer URL must not have query/fragment".into(),
        ));
    }
    let host = parsed.host_str().unwrap_or("");
    let hostname = parsed.host().map(|h| h.to_string()).unwrap_or_default();
    if hostname.is_empty() {
        return Err(ProviderError::Bridge("signer URL missing host".into()));
    }
    let port = parsed.port();
    let internal = is_internal_host(&hostname);
    let lower = parsed.scheme().to_ascii_lowercase();
    match lower.as_str() {
        "http" if internal => Ok(()),
        "https" if internal || port.is_none() || port == Some(443) => Ok(()),
        _ => Err(ProviderError::Bridge(
            "signer URL must be HTTPS:443 or trusted internal HTTP".into(),
        )),
    }
    .map_err(|e| {
        tracing::warn!(url = %host, "signer url rejected: {e}");
        e
    })
}

fn is_internal_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return false;
    }
    // IPv4 字面量：私有/回环/链路本地
    if let Ok(addr) = host.parse::<std::net::Ipv4Addr>() {
        return addr.is_private() || addr.is_loopback() || addr.is_link_local();
    }
    if let Ok(addr) = host.parse::<std::net::Ipv6Addr>() {
        return addr.is_loopback() || addr.is_unicast_link_local();
    }
    let lower = host.to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || (!lower.contains('.') && is_service_label(&lower))
}

fn is_service_label(value: &str) -> bool {
    if value.is_empty() || value.len() > 63 {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_accepts_public_https_443() {
        assert!(validate_signer_url("https://grok.wodf.de/sign").is_ok());
        assert!(validate_signer_url("https://signer.example.com:443/sign").is_ok());
    }

    #[test]
    fn validate_rejects_public_http_and_bad_ports() {
        assert!(validate_signer_url("http://grok.wodf.de/sign").is_err());
        assert!(validate_signer_url("https://grok.wodf.de:8443/sign").is_err());
        assert!(validate_signer_url("https://grok.wodf.de/sign?x=1").is_err());
        assert!(validate_signer_url("https://user:pass@grok.wodf.de/sign").is_err());
    }

    #[test]
    fn validate_accepts_internal_http() {
        assert!(validate_signer_url("http://localhost:8099/sign").is_ok());
        assert!(validate_signer_url("http://127.0.0.1:9000/sign").is_ok());
        assert!(validate_signer_url("http://signer.internal/sign").is_ok());
        assert!(validate_signer_url("http://signer:9000/sign").is_ok());
        assert!(validate_signer_url("http://10.0.0.5:8080/sign").is_ok());
    }

    #[tokio::test]
    async fn sign_parses_statsig_id_from_fake_signer() {
        // 用本地 axum 服务模拟 signer + 首页。
        let (addr, _guard) = spawn_fake_upstream().await;
        let client = Client::new();
        let signer = StatsigSigner::new(format!("http://{addr}/sign"));
        let id = signer
            .sign(
                &client,
                &format!("http://{addr}"),
                &format!("http://{addr}/sign"),
                "",
                "POST",
                "/rest/app-chat/conversations/new",
            )
            .await
            .expect("sign");
        assert_eq!(id, "fake-statsig-id");
        // 第二次调用命中缓存：fake 首页计数仍为 1。
        let id2 = signer
            .sign(
                &client,
                &format!("http://{addr}"),
                &format!("http://{addr}/sign"),
                "",
                "POST",
                "/rest/app-chat/conversations/new",
            )
            .await
            .expect("sign 2");
        assert_eq!(id2, "fake-statsig-id");
        assert_eq!(HOME_HITS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sign_rejects_oversized_response() {
        let (addr, _guard) = spawn_big_signer().await;
        let client = Client::new();
        let signer = StatsigSigner::new(format!("http://{addr}/sign"));
        let err = signer
            .sign(
                &client,
                &format!("http://{addr}"),
                &format!("http://{addr}/sign"),
                "",
                "POST",
                "/x",
            )
            .await
            .expect_err("should reject");
        assert!(err.to_string().contains("too large"));
    }

    // ── fake 上游（axum）──────────────────────────────────────────

    use std::sync::atomic::AtomicUsize;

    static HOME_HITS: AtomicUsize = AtomicUsize::new(0);

    async fn spawn_fake_upstream() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{routing::get, Router};
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    HOME_HITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    "<html><head><meta charset=utf-8></head><body>grok fake home</body></html>"
                }),
            )
            .route(
                "/sign",
                axum::routing::post(|payload: axum::Json<serde_json::Value>| async move {
                    let _ = payload;
                    axum::Json(json!({ "x-statsig-id": "fake-statsig-id" }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (addr.to_string(), handle)
    }

    async fn spawn_big_signer() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{routing::get, Router};
        let app = Router::new()
            .route("/", get(|| async { "home".to_string() }))
            .route(
                "/sign",
                axum::routing::post(|_: axum::Json<serde_json::Value>| async move {
                    axum::Json(json!({ "x-statsig-id": "x".repeat(9000) }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (addr.to_string(), handle)
    }
}
