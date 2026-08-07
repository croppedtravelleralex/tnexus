//! grok2api-rs 配置加载与校验。
//!
//! 对齐 Go `internal/config/config.go` 语义（`config.Validate`）：无效配置必须
//! 拒绝启动，而不是用默认值悄悄掩盖。来源采用「环境变量优先」的最小实现，
//! 不引入 serde_yaml —— 需要 config.yaml 文件时再加（YAGNI）。
//!
//! G0 只保留入口 + 健康检查所需字段；号池 / provider / 后台任务配置留待
//! 对应 Phase（39a G3/G4）。

use std::env;

use anyhow::{bail, Context, Result};

/// grok2api-rs 运行配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// HTTP 监听地址，默认 `0.0.0.0:8000`（39 主文档 §5 目标端口）。
    pub server_addr: String,
    /// PostgreSQL 连接串。可选用 `GROK_DATABASE_URL`，回退 `DATABASE_URL`。
    pub database_url: String,
    /// Redis 连接串（G3+ 多实例必选）。G0 仅为占位字段，默认空 = memory runtime。
    pub redis_url: Option<String>,
    /// browser-bridge 侧车地址，默认 `http://browser-bridge:8192`（39d §8）。
    pub browser_bridge_url: String,
    /// Build 上游 base URL（N5 挂载 `/v1/responses` 用；None = 走
    /// `grok_provider_build::default_base_url()`，即 env `GROK2API_BUILD_BASE_URL` 或常量）。
    pub build_base_url: Option<String>,
    /// Console 上游 base URL（N5 挂载 `/v1/messages` 用；None = 走
    /// `grok_provider_console::default_base_url()`，即 env `GROK2API_CONSOLE_BASE_URL` 或常量）。
    pub console_base_url: Option<String>,
    /// `/v1` 写操作鉴权密钥（`GROK_GATEWAY_AUTH_KEY` 或 `GATEWAY_AUTH_KEY`）。
    /// 空 = 不校验（生产必须配置，启动时告警）。
    pub gateway_auth_key: Option<String>,
    /// `/admin/*` 独立监听地址（`GROK_ADMIN_LISTEN`，默认 `0.0.0.0:8091`，仅内网）。
    pub admin_listen: String,
    /// Grok 管理台 JWT secret（N5 起 `/admin/*` 鉴权用）。缺省时随机生成并告警。
    pub admin_secret: String,
    /// 首启 bootstrap 的管理员用户名（默认 `admin`）。
    pub admin_username: String,
    /// 首启 bootstrap 的管理员密码；缺省不设置 → admin 不可用并告警。
    pub admin_password: Option<String>,
    /// 生图引擎开关（`GROK_IMAGE_ENABLED=1`）。生图走真实 bridge（默认不外呼，
    /// 与 provider「未配置不外呼」红线一致）；未开启时 `/v1/images/generations` 500。
    pub image_enabled: bool,
    /// 无 chrome 直连开关（`GROK2API_DIRECT=1`）：chat/OCR 不走 browser-bridge，
    /// 直连 grok.com（sso cookie + statsig 签名）。**缺省开**（纯 http 为唯一路径）；
    /// 显式 `GROK2API_DIRECT=0` 才回退 bridge 模式。
    pub direct_enabled: bool,
    /// statsig signer 服务地址（`GROK2API_SIGNER_URL`，缺省 https://grok.wodf.de/sign）。
    pub signer_url: String,
    /// 凭据解密主密钥（`GROK_CREDENTIAL_KEY`，base64 32B AES-256-GCM）。
    /// 直连模式下缺失 → chat/OCR 503 不外呼（安全红线）。
    pub credential_key: Option<String>,
    /// 住宅代理列表文件（`GROK2API_PROXY_FILE`）；空 = 直连。
    pub proxy_file: Option<String>,
    /// 内联代理列表（`GROK2API_PROXY_LIST`，逗号分隔 webshare 格式条目）。
    pub proxy_list: Option<String>,
    /// 本地出口代理（`GROK_LOCAL_PROXY`，如 http://127.0.0.1:7897；meta/签名/直连走它）。
    pub local_proxy: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let server_addr = env::var("GROK2API_ADDR").unwrap_or_else(|_| "0.0.0.0:8000".into());

        let database_url = env::var("GROK_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .context("missing database_url (GROK_DATABASE_URL or DATABASE_URL)")?;

        let redis_url = env::var("GROK_REDIS_URL")
            .or_else(|_| env::var("REDIS_URL"))
            .ok()
            .filter(|s| !s.trim().is_empty());

        let browser_bridge_url = env::var("GROK2API_BROWSER_BRIDGE_URL")
            .unwrap_or_else(|_| "http://browser-bridge:8192".into());

        let build_base_url = env::var("GROK2API_BUILD_BASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let console_base_url = env::var("GROK2API_CONSOLE_BASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let gateway_auth_key = env::var("GROK_GATEWAY_AUTH_KEY")
            .or_else(|_| env::var("GATEWAY_AUTH_KEY"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        let admin_listen = env::var("GROK_ADMIN_LISTEN").unwrap_or_else(|_| "0.0.0.0:8091".into());

        let admin_secret = env::var("GROK_ADMIN_SECRET").unwrap_or_default();
        let admin_username = env::var("GROK_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
        let admin_password = env::var("GROK_ADMIN_PASSWORD")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let image_enabled = env::var("GROK_IMAGE_ENABLED")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);

        let direct_enabled = env::var("GROK2API_DIRECT")
            .map(|v| v.trim() == "1")
            .unwrap_or(true);
        let signer_url =
            env::var("GROK2API_SIGNER_URL").unwrap_or_else(|_| "https://grok.wodf.de/sign".into());
        let credential_key = env::var("GROK_CREDENTIAL_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let proxy_file = env::var("GROK2API_PROXY_FILE")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let proxy_list = env::var("GROK2API_PROXY_LIST")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let local_proxy = env::var("GROK_LOCAL_PROXY").ok();

        let config = Self {
            server_addr,
            database_url,
            redis_url,
            browser_bridge_url,
            build_base_url,
            console_base_url,
            gateway_auth_key,
            admin_listen,
            admin_secret,
            admin_username,
            admin_password,
            image_enabled,
            direct_enabled,
            signer_url,
            credential_key,
            proxy_file,
            proxy_list,
            local_proxy,
        };
        config.validate()?;
        Ok(config)
    }

    /// 对齐 Go `config.Validate`：非法值拒绝启动。
    pub fn validate(&self) -> Result<()> {
        self.server_addr
            .parse::<std::net::SocketAddr>()
            .context("invalid server_addr, expected host:port")?;
        self.admin_listen
            .parse::<std::net::SocketAddr>()
            .context("invalid admin_listen, expected host:port")?;

        let db = &self.database_url;
        if !db.starts_with("postgres://") && !db.starts_with("postgresql://") {
            bail!("database_url must be a postgres:// or postgresql:// URL");
        }

        if let Some(r) = self.redis_url.as_deref() {
            if !r.starts_with("redis://") && !r.starts_with("rediss://") {
                bail!("redis_url must be redis:// or rediss:// URL");
            }
        }

        let bridge = &self.browser_bridge_url;
        if !bridge.starts_with("http://") && !bridge.starts_with("https://") {
            bail!("browser_bridge_url must be http(s):// URL");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(addr: &str, db: Option<&str>, redis: Option<&str>) -> Config {
        Config {
            server_addr: addr.to_string(),
            database_url: db
                .unwrap_or("postgres://user:pass@localhost:5432/grok")
                .to_string(),
            redis_url: redis.map(str::to_string),
            browser_bridge_url: "http://browser-bridge:8192".to_string(),
            build_base_url: None,
            console_base_url: None,
            gateway_auth_key: None,
            admin_listen: "0.0.0.0:8091".to_string(),
            admin_secret: "12345678901234567890123456789012".to_string(),
            admin_username: "admin".to_string(),
            admin_password: Some("admin123456".to_string()),
            image_enabled: false,
            direct_enabled: true,
            signer_url: "https://grok.wodf.de/sign".to_string(),
            credential_key: None,
            proxy_file: None,
            proxy_list: None,
            local_proxy: None,
        }
    }

    #[test]
    fn valid_config_passes() {
        let c = config_with("0.0.0.0:8000", None, None);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn invalid_addr_rejected() {
        let c = config_with("not-an-addr", None, None);
        assert!(c.validate().is_err());
    }

    #[test]
    fn missing_database_rejected() {
        let c = config_with("0.0.0.0:8000", Some(""), None);
        assert!(c.validate().is_err());
    }

    #[test]
    fn invalid_database_scheme_rejected() {
        let c = config_with("0.0.0.0:8000", Some("sqlite:///tmp/x.db"), None);
        assert!(c.validate().is_err());
    }

    #[test]
    fn invalid_redis_scheme_rejected() {
        let c = config_with("0.0.0.0:8000", None, Some("mysql://x"));
        assert!(c.validate().is_err());
    }
    #[test]
    fn direct_enabled_defaults_on_via_env() {
        // 缺省（未设 GROK2API_DIRECT）→ 直连开（纯 http 为唯一路径）。
        std::env::remove_var("GROK2API_DIRECT");
        std::env::set_var("GROK_DATABASE_URL", "postgres://u:p@h:1/d");
        std::env::remove_var("GROK2API_PROXY_FILE");
        std::env::remove_var("GROK2API_PROXY_LIST");
        let c = Config::from_env().expect("config");
        assert!(c.direct_enabled, "GROK2API_DIRECT 缺省必须为 true");
        assert!(c.proxy_file.is_none());
        // 显式 "0" → 关（回退 bridge）。
        std::env::set_var("GROK2API_DIRECT", "0");
        let c2 = Config::from_env().expect("config");
        assert!(!c2.direct_enabled);
        std::env::remove_var("GROK2API_DIRECT");
        std::env::remove_var("GROK_DATABASE_URL");
    }
}
