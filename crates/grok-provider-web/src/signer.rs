//! 签名器端口（SignerTrait）+ 本地/远程/假 三种实现。
//!
//! 直连路径需要 `x-statsig-id`：
//! - Remote：外部 signer 服务（statsig.rs `StatsigSigner`，HTTP POST，默认 grok.wodf.de/sign）
//! - Local：本地 JS 引擎执行 grok.com 前端签名 bundle（`grok-signer` crate）
//! - Fake：固定格式假签名（仅测试/联调，上游会 403，安全红线：不误当真实）
//!
//! env：`GROK2API_SIGNER_MODE=native|remote|local|fake`（缺省 native：Rust statsig，无 QuickJS/外网 signer）。

use crate::statsig::{NativeSigner, StatsigSigner};
use grok_domain::ProviderError;
use reqwest::Client;
use std::fmt;

/// 签名器模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignerMode {
    /// Rust `generate_statsig`（抓 meta + 本地签名，推荐生产）。
    #[default]
    Native,
    Remote,
    Local,
    Fake,
}

impl fmt::Display for SignerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignerMode::Native => write!(f, "native"),
            SignerMode::Remote => write!(f, "remote"),
            SignerMode::Local => write!(f, "local"),
            SignerMode::Fake => write!(f, "fake"),
        }
    }
}

impl std::str::FromStr for SignerMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "native" | "" => Ok(SignerMode::Native),
            "remote" => Ok(SignerMode::Remote),
            "local" => Ok(SignerMode::Local),
            "fake" => Ok(SignerMode::Fake),
            other => Err(format!(
                "GROK2API_SIGNER_MODE 非法: {other}（期望 native|remote|local|fake）"
            )),
        }
    }
}

/// 签名端口：`method + path → x-statsig-id`。
/// `client` 为本次请求出口客户端（Remote 用它抓 meta + 调 signer；Local/Fake 忽略）。
#[async_trait::async_trait]
pub trait SignerTrait: Send + Sync {
    async fn sign(
        &self,
        client: &Client,
        base_url: &str,
        signer_url: &str,
        sso_cookie: &str,
        method: &str,
        path: &str,
    ) -> Result<String, ProviderError>;
}

/// 远程 signer：委托 statsig.rs `StatsigSigner`（含 meta 缓存 + signer URL 校验）。
#[async_trait::async_trait]
impl SignerTrait for StatsigSigner {
    async fn sign(
        &self,
        client: &Client,
        base_url: &str,
        signer_url: &str,
        sso_cookie: &str,
        method: &str,
        path: &str,
    ) -> Result<String, ProviderError> {
        self.sign_remote(client, base_url, signer_url, sso_cookie, method, path)
            .await
    }
}

/// 本地 signer：本地 JS 引擎执行 grok.com 前端签名 bundle。
/// - bundle 未就绪（assets/grok_sign_standalone.js 缺失）→ NotConfigured（直连降级 503，不外呼）
/// - Fake 模式：用 FAKE_SIGNER_BUNDLE 产出固定格式 id（仅测试）
pub struct LocalSigner {
    /// 签名 bundle JS（None = 未就绪）。
    bundle: Option<String>,
}

impl LocalSigner {
    /// local 模式：运行时加载资产（缺失 → None，sign 时 NotConfigured）。
    pub fn local() -> Self {
        Self {
            bundle: grok_signer::load_asset(),
        }
    }

    /// fake 模式：内置假 bundle（仅测试/联调）。
    pub fn fake() -> Self {
        Self {
            bundle: Some(grok_signer::FAKE_SIGNER_BUNDLE.to_string()),
        }
    }

    pub fn bundle_ready(&self) -> bool {
        self.bundle.is_some()
    }
}

#[async_trait::async_trait]
impl SignerTrait for LocalSigner {
    async fn sign(
        &self,
        _client: &Client,
        _base_url: &str,
        _signer_url: &str,
        _sso_cookie: &str,
        method: &str,
        path: &str,
    ) -> Result<String, ProviderError> {
        let bundle = self
            .bundle
            .as_deref()
            .ok_or_else(|| ProviderError::NotConfigured("本地签名 bundle 未就绪".into()))?;
        grok_signer::execute_standalone_bundle(bundle, path, method)
            .map_err(|e| ProviderError::Bridge(format!("local signer: {e}")))
    }
}

/// Playwright 一次提取的 session 签名材料（对齐 Python pure_http_keys/*.json）。
#[derive(Debug, Clone)]
pub struct SessionKeys {
    pub meta48: [u8; 48],
    pub fingerprint: String,
    pub trailer: Vec<u8>,
    /// Playwright 提取的完整 Cookie（含 `cf_clearance` 时优先于裸 sso）。
    pub cookie: Option<String>,
    /// grok2api 账号 id（`account_{id}.json` 或 JSON 字段）。
    pub account_id: Option<i64>,
}

impl SessionKeys {
    /// 从 `pure_http_keys` JSON 加载。
    pub fn from_json(value: &serde_json::Value) -> Result<Self, ProviderError> {
        use base64::Engine;
        let meta_b64 = value
            .get("meta_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::NotConfigured("session keys missing meta_b64".into()))?;
        let pad = "=".repeat((4 - meta_b64.len() % 4) % 4);
        let raw = base64::engine::general_purpose::STANDARD
            .decode(format!("{meta_b64}{pad}"))
            .map_err(|e| ProviderError::NotConfigured(format!("meta_b64 decode: {e}")))?;
        if raw.len() != 48 {
            return Err(ProviderError::NotConfigured(format!(
                "meta48 len={}",
                raw.len()
            )));
        }
        let mut meta48 = [0u8; 48];
        meta48.copy_from_slice(&raw);
        let fingerprint = value
            .get("fingerprint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let trailer = value
            .get("trailer_hex")
            .and_then(|v| v.as_str())
            .map(hex::decode)
            .transpose()
            .map_err(|e| ProviderError::NotConfigured(format!("trailer_hex: {e}")))?
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![0x03]);
        Ok(Self {
            meta48,
            fingerprint,
            trailer,
            cookie: None,
            account_id: None,
        })
    }
}

/// Python `generate_statsig` 路径：meta48 + fingerprint → x-statsig-id。
pub struct SessionSigner {
    keys: SessionKeys,
}

impl SessionSigner {
    pub fn new(keys: SessionKeys) -> Self {
        Self { keys }
    }
}

#[async_trait::async_trait]
impl SignerTrait for SessionSigner {
    async fn sign(
        &self,
        _client: &Client,
        _base_url: &str,
        _signer_url: &str,
        _sso_cookie: &str,
        method: &str,
        path: &str,
    ) -> Result<String, ProviderError> {
        grok_signer::statsig_obfiowerehiring::generate_statsig(
            method,
            path,
            &self.keys.meta48,
            &self.keys.fingerprint,
            &self.keys.trailer,
        )
        .map_err(|e| ProviderError::Bridge(format!("session sign: {e}")))
    }
}

/// 按模式构造签名器。
pub fn build_signer(mode: SignerMode, signer_url: &str) -> Box<dyn SignerTrait> {
    match mode {
        SignerMode::Native => Box::new(NativeSigner::new()),
        SignerMode::Remote => Box::new(StatsigSigner::new(signer_url.to_string())),
        SignerMode::Local => Box::new(LocalSigner::local()),
        SignerMode::Fake => Box::new(LocalSigner::fake()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parse_roundtrip() {
        assert_eq!("native".parse::<SignerMode>().unwrap(), SignerMode::Native);
        assert_eq!("".parse::<SignerMode>().unwrap(), SignerMode::Native);
        assert_eq!("remote".parse::<SignerMode>().unwrap(), SignerMode::Remote);
        assert_eq!("LOCAL".parse::<SignerMode>().unwrap(), SignerMode::Local);
        assert_eq!("fake".parse::<SignerMode>().unwrap(), SignerMode::Fake);
        assert!("bogus".parse::<SignerMode>().is_err());
    }

    #[tokio::test]
    async fn local_without_asset_is_not_configured() {
        let signer = LocalSigner::local();
        // 资产可能已就绪（并行任务落地）；未就绪时断言 NotConfigured。
        if !signer.bundle_ready() {
            let client = Client::new();
            let err = signer
                .sign(&client, "https://grok.com", "", "", "POST", "/x")
                .await
                .expect_err("should be not configured");
            assert!(matches!(err, ProviderError::NotConfigured(_)), "got {err}");
        }
    }

    #[tokio::test]
    async fn fake_mode_produces_expected_format() {
        let signer = LocalSigner::fake();
        let client = Client::new();
        let id = signer
            .sign(
                &client,
                "https://grok.com",
                "",
                "",
                "POST",
                "/rest/app-chat/conversations/new",
            )
            .await
            .expect("fake sign");
        assert!(id.starts_with("x0:"), "got {id}");
        // 同一 path/method 不同时刻结果不同（含时间戳）→ 仅验证格式。
        assert!(id.len() > 10);
    }
}
