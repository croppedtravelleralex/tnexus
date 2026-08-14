//! 住宅代理池（webshare 等 `host:port:user:pass` 列表）。
//!
//! - 解析文本行：`host:port:user:pass`、`user:pass@host:port`、`host:port`（空行/`#` 注释跳过）。
//! - 账号→出口稳定映射：`hash(sso_token) % len`（同一账号同一出口，对齐 Go「账号↔代理绑定」语义）。
//! - 每代理一个独立 `reqwest::Client`（reqwest 的 client 绑定单一 proxy，不能共享）。

use std::sync::Arc;

use grok_domain::ProviderError;

/// 单个代理端点（`http://user:pass@host:port`）。
#[derive(Debug, Clone)]
pub struct ProxyEndpoint {
    pub url: String,
}

/// 代理池：解析 + 稳定映射 + 惰性 per-proxy client。
pub struct ProxyPool {
    endpoints: Vec<ProxyEndpoint>,
    clients: Vec<reqwest::Client>,
    /// Lite 生图 SSE 用更长超时（与 chat/OCR 的 60s 分离）。
    imagine_clients: Vec<reqwest::Client>,
}

impl ProxyPool {
    /// 空池（无代理：直连）。
    pub fn empty() -> Self {
        Self {
            endpoints: Vec::new(),
            clients: Vec::new(),
            imagine_clients: Vec::new(),
        }
    }

    /// 从文本解析（每行一个代理；支持 `host:port:user:pass` / `user:pass@host:port` / `host:port`）。
    pub fn from_text(text: &str) -> Self {
        Self::from_text_with_timeouts(text, 60, imagine_timeout_secs())
    }

    /// chat/OCR 与 Lite 生图可设不同 HTTP 总超时。
    pub fn from_text_with_timeouts(text: &str, timeout_secs: u64, imagine_timeout_secs: u64) -> Self {
        let endpoints: Vec<ProxyEndpoint> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| parse_proxy_line(l).map(|url| ProxyEndpoint { url }))
            .collect();
        let clients = endpoints
            .iter()
            .map(|e| build_proxy_client(&e.url, timeout_secs))
            .collect();
        let imagine_clients = endpoints
            .iter()
            .map(|e| build_proxy_client(&e.url, imagine_timeout_secs))
            .collect();
        Self {
            endpoints,
            clients,
            imagine_clients,
        }
    }

    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    /// 按 sso token 稳定取出口 client（同一 token 恒同一代理）。
    pub fn client_for(&self, sso_token: &str) -> Option<&reqwest::Client> {
        if self.clients.is_empty() {
            return None;
        }
        let idx = fnv1a(sso_token) as usize % self.clients.len();
        self.clients.get(idx)
    }

    /// Lite 生图 SSE 专用 client（更长超时）。
    pub fn client_for_imagine(&self, sso_token: &str) -> Option<&reqwest::Client> {
        if self.imagine_clients.is_empty() {
            return None;
        }
        let idx = fnv1a(sso_token) as usize % self.imagine_clients.len();
        self.imagine_clients.get(idx)
    }

    /// 按 sso/cookie 稳定取代理 URL（WS CONNECT 用）。
    pub fn proxy_url_for(&self, sso_token: &str) -> Option<&str> {
        if self.endpoints.is_empty() {
            return None;
        }
        let idx = fnv1a(sso_token) as usize % self.endpoints.len();
        self.endpoints.get(idx).map(|e| e.url.as_str())
    }

    /// 供测试/日志：代理地址清单（脱敏：隐藏密码段）。
    pub fn describe(&self) -> Vec<String> {
        self.endpoints.iter().map(|e| mask_proxy(&e.url)).collect()
    }
}

/// 解析单行代理 → `http://user:pass@host:port`。
fn parse_proxy_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line
        .strip_prefix("http://")
        .or_else(|| line.strip_prefix("https://"))
        .unwrap_or(line);
    let parts: Vec<&str> = line.split(':').collect();
    match parts.len() {
        // user:pass@host:port 或 host:port（2 段：先看有无 @）
        2 => {
            if let Some((auth, hp)) = line.split_once('@') {
                if let Some((host, port)) = hp.split_once(':') {
                    if host.is_empty() || port.is_empty() {
                        return None;
                    }
                    return Some(format!("http://{auth}@{host}:{port}"));
                }
                return None;
            }
            let (host, port) = (parts[0], parts[1]);
            if host.is_empty() || port.is_empty() {
                return None;
            }
            Some(format!("http://{host}:{port}"))
        }
        // user:pass@host:port（3 段：split(':') 后中间段含 @）
        3 => {
            let (auth, hp) = line.split_once('@')?;
            let (host, port) = hp.split_once(':')?;
            if host.is_empty() || port.is_empty() {
                return None;
            }
            Some(format!("http://{auth}@{host}:{port}"))
        }
        // host:port:user:pass（4 段）
        4 => {
            let (host, port, user, pass) = (parts[0], parts[1], parts[2], parts[3]);
            if host.is_empty() || port.is_empty() || user.is_empty() {
                return None;
            }
            Some(format!("http://{user}:{pass}@{host}:{port}"))
        }
        _ => None,
    }
}

fn build_proxy_client(proxy_url: &str, timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url).expect("proxy url"))
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(timeout_secs.max(5)))
        .build()
        .expect("proxy client")
}

/// Lite 生图 HTTP 总超时（秒），默认 120。
pub fn imagine_timeout_secs() -> u64 {
    std::env::var("GROK_IMAGINE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120)
        .max(30)
}

/// FNV-1a 64 位（稳定、跨进程一致，用于代理绑定）。
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 脱敏：`http://user:pass@host:port` → `http://user:***@host:port`。
fn mask_proxy(url: &str) -> String {
    match url.split_once('@') {
        Some((auth, rest)) => {
            let auth = auth.strip_prefix("http://").unwrap_or(auth);
            let user = auth.rsplit_once(':').map(|(u, _)| u).unwrap_or(auth);
            format!("http://{user}:***@{rest}")
        }
        None => url.to_string(),
    }
}

/// 从文件路径解析代理池（失败 → 空池 + 告警，不阻断启动）。
pub fn proxy_pool_from_file(path: Option<&str>) -> Arc<ProxyPool> {
    let Some(path) = path else {
        return Arc::new(ProxyPool::empty());
    };
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let pool = ProxyPool::from_text(&text);
            tracing::info!(
                "代理池加载 {} 个住宅节点: {:?}",
                pool.len(),
                pool.describe()
            );
            Arc::new(pool)
        }
        Err(e) => {
            tracing::warn!("代理文件不可读（走直连）: {path}: {e}");
            Arc::new(ProxyPool::empty())
        }
    }
}

/// 合并代理文件 + 内联列表（webshare 文件 + udeal 单条均可并存）。
pub fn proxy_pool_merged(file: Option<&str>, inline_list: Option<&str>) -> Arc<ProxyPool> {
    let mut lines: Vec<String> = Vec::new();
    if let Some(path) = file {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                for line in text.lines() {
                    let l = line.trim();
                    if !l.is_empty() && !l.starts_with('#') {
                        lines.push(l.to_string());
                    }
                }
            }
            Err(e) => tracing::warn!("代理文件不可读: {path}: {e}"),
        }
    }
    if let Some(list) = inline_list {
        for part in list.split(',') {
            let p = part.trim();
            if !p.is_empty() {
                lines.push(p.to_string());
            }
        }
    }
    if lines.is_empty() {
        return Arc::new(ProxyPool::empty());
    }
    let pool = ProxyPool::from_text(&lines.join("\n"));
    tracing::info!(
        "代理池合并 {} 个住宅节点: {:?}",
        pool.len(),
        pool.describe()
    );
    Arc::new(pool)
}

/// 构造直连请求错误（统一错误面）。
pub fn proxy_err(e: reqwest::Error) -> ProviderError {
    let tag = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else {
        "unknown"
    };
    ProviderError::Bridge(format!("direct {tag}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_webshare_four_part_lines() {
        let pool = ProxyPool::from_text("1.2.3.4:8080:user1:pass1\n5.6.7.8:9000:user2:pass2\n");
        assert_eq!(pool.len(), 2);
        let d = pool.describe();
        assert_eq!(d[0], "http://user1:***@1.2.3.4:8080");
        assert!(!d[0].contains("pass1"), "密码不得出现在 describe");
    }

    #[test]
    fn parses_user_pass_at_host_port() {
        let pool = ProxyPool::from_text("user:pass@10.0.0.1:3128\n");
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.describe()[0], "http://user:***@10.0.0.1:3128");
    }

    #[test]
    fn parses_plain_host_port() {
        let pool = ProxyPool::from_text("10.0.0.2:8080\n");
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn skips_blank_comments_and_bad_lines() {
        let pool = ProxyPool::from_text("# comment\n\n1.2.3.4:8080:u:p\nnot-a-proxy\n");
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn mapping_is_stable_per_token() {
        let pool = ProxyPool::from_text("1.1.1.1:1:u1:p1\n2.2.2.2:2:u2:p2\n3.3.3.3:3:u3:p3\n");
        let a = pool.client_for("token-A").unwrap() as *const _;
        for _ in 0..10 {
            let b = pool.client_for("token-A").unwrap() as *const _;
            assert_eq!(a, b, "同一 token 必须恒同一代理");
        }
        // 不同 token 至少落在一个池内索引（不越界）
        for t in ["x", "y", "z", "long-token-1"] {
            let _ = pool.client_for(t).unwrap();
        }
    }

    #[test]
    fn empty_pool_returns_none() {
        let pool = ProxyPool::empty();
        assert!(pool.is_empty());
        assert!(pool.client_for("x").is_none());
    }

    #[test]
    fn fnv_is_stable_and_spreads() {
        let h1 = fnv1a("token-1");
        let h2 = fnv1a("token-2");
        assert_eq!(h1, fnv1a("token-1"), "FNV 必须确定");
        assert_ne!(h1, h2);
    }
}
