//! 系统信息端点（对齐 Go `transport/http` 的 `/system`；版本/就绪）。
//!
//! 无存储依赖，常量 + 环境注入。

use serde::Serialize;

/// 系统信息视图。
#[derive(Debug, Clone, Serialize)]
pub struct SystemView {
    pub version: String,
    pub commit: String,
    pub uptime_seconds: i64,
    pub ready: bool,
}

/// 系统信息端点服务。
pub struct SystemService {
    version: String,
    started_at: std::sync::OnceLock<std::time::Instant>,
}

impl Default for SystemService {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: std::sync::OnceLock::new(),
        }
    }
}

impl SystemService {
    pub fn new() -> Self {
        Self::default()
    }

    /// 可注入版本（测试用）。
    pub fn with_version(version: &str) -> Self {
        Self {
            version: version.to_string(),
            started_at: std::sync::OnceLock::new(),
        }
    }

    pub fn view(&self) -> SystemView {
        let uptime = self
            .started_at
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_secs() as i64;
        SystemView {
            version: self.version.clone(),
            commit: option_env!("GIT_COMMIT_SHORT").unwrap_or("unknown").to_string(),
            uptime_seconds: uptime,
            ready: true,
        }
    }
}