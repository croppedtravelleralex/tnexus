//! 按账号加载 Playwright 提取的 `pure_http_keys`（对齐 Python `reports/pure_http_keys`）。
//!
//! 文件命名（任一命中即可）：
//! - `account_{id}.json` — grok2api 老池（推荐）
//! - `{email.replace('@', '_at_')}.json` — yumail / 邮箱键
//!
//! JSON 内可选 `"account_id": 86` 用于反查。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use grok_domain::ProviderError;

use crate::signer::SessionKeys;

/// 磁盘上的 session keys 目录（`GROK_PURE_HTTP_KEYS_DIR`）。
#[derive(Debug)]
pub struct SessionKeyStore {
    dir: PathBuf,
    cache: Mutex<HashMap<i64, Arc<SessionKeys>>>,
}

impl SessionKeyStore {
    /// 从环境变量 `GROK_PURE_HTTP_KEYS_DIR` 构造；未设置或目录不存在 → None。
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var("GROK_PURE_HTTP_KEYS_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let path = PathBuf::from(dir.trim());
        if !path.is_dir() {
            tracing::warn!(
                "GROK_PURE_HTTP_KEYS_DIR={} 不是目录，忽略 session keys",
                path.display()
            );
            return None;
        }
        Some(Self::new(path))
    }

    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 按 grok2api 账号 id 取 session keys（有 fingerprint 才视为可用）。
    pub fn has(&self, account_id: i64) -> bool {
        self.get(account_id).is_some()
    }

    /// 扫描目录下 `account_{id}.json` 且含有效 fingerprint 的账号 id。
    pub fn list_account_ids(&self) -> Vec<i64> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let Some(id_str) = name.strip_prefix("account_").and_then(|s| s.strip_suffix(".json")) else {
                continue;
            };
            let Ok(id) = id_str.parse::<i64>() else {
                continue;
            };
            if self.has(id) {
                out.push(id);
            }
        }
        out.sort_unstable();
        out
    }

    /// 按 grok2api 账号 id 取 session keys（有 fingerprint 才视为可用）。
    pub fn get(&self, account_id: i64) -> Option<Arc<SessionKeys>> {
        if let Some(hit) = self.cache.lock().unwrap().get(&account_id).cloned() {
            return Some(hit);
        }
        let keys = self.load_for_account(account_id)?;
        if keys.fingerprint.is_empty() {
            tracing::debug!(
                account_id,
                "session keys 缺 fingerprint，回退 NativeSigner"
            );
            return None;
        }
        self.cache
            .lock()
            .unwrap()
            .insert(account_id, keys.clone());
        Some(keys)
    }

    fn load_for_account(&self, account_id: i64) -> Option<Arc<SessionKeys>> {
        let by_id = self.dir.join(format!("account_{account_id}.json"));
        if by_id.is_file() {
            return Self::parse_file(&by_id, Some(account_id)).ok();
        }
        // 扫描目录：匹配 JSON 内 account_id 或仅 email 命名（调用方已知 id 时跳过）
        let entries = std::fs::read_dir(&self.dir).ok()?;
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("gate_") || name.starts_with("quota_") || name.starts_with("batch_") {
                continue;
            }
            if let Ok(keys) = Self::parse_file(&path, Some(account_id)) {
                if keys.account_id == Some(account_id) {
                    return Some(keys);
                }
            }
        }
        None
    }

    fn parse_file(path: &Path, expect_id: Option<i64>) -> Result<Arc<SessionKeys>, ProviderError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ProviderError::NotConfigured(format!("read {}: {e}", path.display())))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::NotConfigured(format!("json {}: {e}", path.display())))?;
        let file_id = value.get("account_id").and_then(|v| v.as_i64());
        if let (Some(exp), Some(fid)) = (expect_id, file_id) {
            if exp != fid {
                return Err(ProviderError::NotConfigured("account_id mismatch".into()));
            }
        }
        let mut keys = SessionKeys::from_json(&value)?;
        keys.account_id = file_id.or(expect_id);
        keys.cookie = value
            .get("cookie")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|c| !c.trim().is_empty());
        Ok(Arc::new(keys))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_account_id_file() {
        let dir = std::env::temp_dir().join(format!(
            "tnexus_session_keys_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account_42.json");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{
  "account_id": 42,
  "meta_b64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  "fingerprint": "fp-test",
  "trailer_hex": "03",
  "cookie": "sso=x; cf_clearance=y"
}}"#
        )
        .unwrap();
        let store = SessionKeyStore::new(dir.clone());
        let keys = store.get(42).expect("keys");
        assert_eq!(keys.fingerprint, "fp-test");
        assert!(keys.cookie.as_ref().unwrap().contains("cf_clearance"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
