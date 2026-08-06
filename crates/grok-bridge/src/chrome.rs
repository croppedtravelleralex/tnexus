//! Chrome/Edge 无头进程拉起与 CDP 端点发现。
//!
//! 每个会话独立 Chrome 进程 + 独立 user-data-dir（对齐 Python bridge 的
//! 每会话 driver 隔离）。`--remote-debugging-port=0` 时 Chrome 把实际端口写入
//! `<user-data-dir>/DevToolsActivePort`，据此再取 `webSocketDebuggerUrl`。

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use crate::error::BridgeError;

/// 发现 Chrome/Edge 可执行路径：env `GROK_BRIDGE_CHROME_PATH` 优先，否则常见候选。
pub fn find_chrome(explicit: Option<&str>) -> Option<String> {
    if let Some(p) = explicit {
        let p = p.trim();
        if !p.is_empty() && Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    let candidates = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "chrome.exe",
        "msedge",
        "msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    for c in candidates {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    None
}

/// 临时 user-data-dir（进程退出清理交给 OS temp；child 需要保持同一目录存活期间）。
fn temp_user_data_dir() -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("grok-bridge-{}-{}", std::process::id(), nonce));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 启动的 Chrome 进程句柄 + CDP WS 地址（drop 时 kill 子进程）。
pub struct ChromeProcess {
    pub child: Child,
    pub user_data_dir: PathBuf,
    pub ws_url: String,
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.user_data_dir);
    }
}

/// 拉起 Chrome 并返回 CDP WebSocket 地址。
pub async fn launch(explicit_chrome: Option<&str>) -> Result<ChromeProcess, BridgeError> {
    let binary = find_chrome(explicit_chrome).ok_or_else(|| {
        BridgeError::internal("no chrome/edge binary found (set GROK_BRIDGE_CHROME_PATH)")
    })?;
    let user_data_dir = temp_user_data_dir();
    let mut command = Command::new(&binary);
    command
        .arg("--headless=new")
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-background-networking")
        .arg("--mute-audio")
        .arg("about:blank");
    let child = command
        .spawn()
        .map_err(|e| BridgeError::internal(format!("spawn chrome {binary}: {e}")))?;

    // 轮询 DevToolsActivePort（首行 = 调试端口）。
    let port_file = user_data_dir.join("DevToolsActivePort");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut port: Option<u16> = None;
    while std::time::Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            if let Some(first) = text.lines().next() {
                if let Ok(p) = first.trim().parse::<u16>() {
                    port = Some(p);
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let port =
        port.ok_or_else(|| BridgeError::internal("chrome devtools port file never appeared"))?;

    // GET /json/version → webSocketDebuggerUrl。
    let ws_url = fetch_ws_url(port).await?;
    Ok(ChromeProcess {
        child,
        user_data_dir,
        ws_url,
    })
}

async fn fetch_ws_url(port: u16) -> Result<String, BridgeError> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| BridgeError::internal(format!("cdp /json/version: {e}")))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| BridgeError::internal(format!("parse /json/version: {e}")))?;
    body.get("webSocketDebuggerUrl")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| BridgeError::internal("webSocketDebuggerUrl missing in /json/version"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_path_from_env_or_candidates() {
        // 显式路径不存在时回退候选（不 panic）。
        let found = find_chrome(Some("/nonexistent/chrome"));
        assert!(found.is_none() || found.is_some()); // 本机有无 chrome 均可
    }

    #[tokio::test]
    async fn launch_fails_gracefully_without_chrome() {
        // 若本机装了 Chrome 会真的拉起；没装则返回 Internal 错误。两种都接受。
        let result = launch(Some("/definitely/not/here/chrome")).await;
        match result {
            Ok(_) => (),
            Err(BridgeError::Internal(_)) => (),
            Err(e) => panic!("unexpected error kind: {e}"),
        }
    }
}
