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
    logs: std::sync::Mutex<std::collections::VecDeque<LogEntry>>,
}

/// 内存环形日志条目。
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// 配置状态视图（只报布尔，不泄露值）。
#[derive(Debug, Clone, Serialize)]
pub struct ConfigView {
    pub admin_password_set: bool,
    pub gateway_auth_key_set: bool,
    pub database_url_set: bool,
    pub redis_url_set: bool,
    pub build_token_set: bool,
    pub console_token_set: bool,
}

impl Default for SystemService {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: std::sync::OnceLock::new(),
            logs: std::sync::Mutex::new(std::collections::VecDeque::new()),
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
            logs: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// 记录一条运行日志（环形缓冲，上限 200 条）。
    pub fn log(&self, level: &str, message: &str) {
        let mut logs = self.logs.lock().unwrap();
        logs.push_back(LogEntry {
            level: level.to_string(),
            message: message.to_string(),
            at: chrono::Utc::now(),
        });
        while logs.len() > 200 {
            logs.pop_front();
        }
    }

    /// 最近 N 条日志。
    pub fn recent_logs(&self, limit: usize) -> Vec<LogEntry> {
        let logs = self.logs.lock().unwrap();
        logs.iter().rev().take(limit).cloned().collect()
    }

    /// 关键配置状态（env 探测，不泄露值）。
    pub fn config_view(&self) -> ConfigView {
        ConfigView {
            admin_password_set: std::env::var("GROK_ADMIN_PASSWORD").is_ok(),
            gateway_auth_key_set: std::env::var("GROK_GATEWAY_AUTH_KEY").is_ok()
                || std::env::var("GATEWAY_AUTH_KEY").is_ok(),
            database_url_set: std::env::var("GROK_DATABASE_URL").is_ok()
                || std::env::var("DATABASE_URL").is_ok(),
            redis_url_set: std::env::var("GROK_REDIS_URL").is_ok()
                || std::env::var("REDIS_URL").is_ok(),
            build_token_set: std::env::var("GROK2API_BUILD_TOKEN").is_ok(),
            console_token_set: std::env::var("GROK2API_CONSOLE_TOKEN").is_ok(),
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
            commit: option_env!("GIT_COMMIT_SHORT")
                .unwrap_or("unknown")
                .to_string(),
            uptime_seconds: uptime,
            ready: true,
        }
    }
}
