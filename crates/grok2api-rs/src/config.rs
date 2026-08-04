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

        let config = Self {
            server_addr,
            database_url,
            redis_url,
            browser_bridge_url,
        };
        config.validate()?;
        Ok(config)
    }

    /// 对齐 Go `config.Validate`：非法值拒绝启动。
    pub fn validate(&self) -> Result<()> {
        self.server_addr
            .parse::<std::net::SocketAddr>()
            .context("invalid server_addr, expected host:port")?;

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
}
