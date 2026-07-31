//! Account pool file loader for tnexus-api (gptimage-compatible responses).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

const STATE_VERIFIED: &str = "verified_ready";
const STATE_ISOLATED: &str = "identity_isolated";

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AccountFile {
    email: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    proxy: Option<String>,
    #[serde(default)]
    proxy_egress_ip: Option<String>,
    #[serde(default)]
    proxy_provider: Option<String>,
    #[serde(default)]
    cf_daily: Vec<Value>,
    #[serde(default)]
    egress_daily: Vec<Value>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    quota: Option<i64>,
    #[serde(default)]
    stored_status: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SchedulingStateFile {
    #[serde(default)]
    by_email: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub added: usize,
    pub skipped: usize,
    pub updated: usize,
}

#[derive(Clone)]
pub struct AccountsStore {
    inner: Arc<RwLock<HashMap<String, AccountFile>>>,
    scheduling: Arc<RwLock<HashMap<String, String>>>,
    scheduling_path: PathBuf,
    pool_path: PathBuf,
}

impl Default for AccountsStore {
    fn default() -> Self {
        Self::from_env().unwrap_or_else(|_| Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            scheduling: Arc::new(RwLock::new(HashMap::new())),
            scheduling_path: scheduling_state_path(),
            pool_path: pool_file_path(),
        })
    }
}

impl AccountsStore {
    pub fn from_env() -> Result<Self> {
        let scheduling_path = scheduling_state_path();
        let pool_path = pool_file_path();
        let scheduling = load_scheduling_state(&scheduling_path)?;
        let mut map = HashMap::new();
        if pool_path.exists() {
            for row in load_accounts_file(pool_path.clone())? {
                map.insert(row.email.to_lowercase(), row);
            }
        }
        if let Ok(path) = std::env::var("ACCOUNTS_FILE") {
            let custom = PathBuf::from(path);
            if custom != pool_path {
                for row in load_accounts_file(custom)? {
                    map.insert(row.email.to_lowercase(), row);
                }
            }
        }
        if let Ok(path) = std::env::var("PIN_ACCOUNT_FILE") {
            let row = load_single(PathBuf::from(path))?;
            map.insert(row.email.to_lowercase(), row);
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
            scheduling: Arc::new(RwLock::new(scheduling)),
            scheduling_path,
            pool_path,
        })
    }

    pub async fn reload(&self) -> Result<usize> {
        let fresh = Self::from_env()?;
        let n = fresh.inner.read().await.len();
        {
            let mut accounts = self.inner.write().await;
            *accounts = fresh.inner.read().await.clone();
        }
        {
            let mut scheduling = self.scheduling.write().await;
            *scheduling = fresh.scheduling.read().await.clone();
        }
        Ok(n)
    }

    fn receive_state_for(email: &str, scheduling: &HashMap<String, String>) -> String {
        scheduling
            .get(&email.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    fn manual_scheduling_enabled(receive_state: &str) -> bool {
        let receive = receive_state.trim().to_lowercase();
        if receive.is_empty() {
            return true;
        }
        matches!(
            receive.as_str(),
            "verified_ready" | "verified" | "local_verified"
        )
    }

    fn row_to_json(row: &AccountFile, scheduling: &HashMap<String, String>) -> Value {
        let receive_state = Self::receive_state_for(&row.email, scheduling);
        let has_token = !row.access_token.is_empty();
        let status = row
            .stored_status
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if has_token {
                    "正常".to_string()
                } else {
                    "异常".to_string()
                }
            });
        let manual_on = Self::manual_scheduling_enabled(&receive_state);
        let image_schedulable = has_token && status == "正常" && manual_on;
        let obs = crate::usage_metrics::load_observability_by_email();
        let extra = obs.get(&row.email.to_lowercase());
        let cf_daily = if !row.cf_daily.is_empty() {
            Value::Array(row.cf_daily.clone())
        } else {
            extra
                .and_then(|v| v.get("cf_daily"))
                .cloned()
                .unwrap_or_else(|| Value::Array(vec![]))
        };
        let egress_daily = if !row.egress_daily.is_empty() {
            Value::Array(row.egress_daily.clone())
        } else {
            extra
                .and_then(|v| v.get("egress_daily"))
                .cloned()
                .unwrap_or_else(|| Value::Array(vec![]))
        };
        let proxy_egress_ip = row
            .proxy_egress_ip
            .clone()
            .or_else(|| {
                extra
                    .and_then(|v| v.get("proxy_egress_ip"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            });
        let proxy_provider = row
            .proxy_provider
            .clone()
            .or_else(|| {
                extra
                    .and_then(|v| v.get("proxy_provider"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            });
        json!({
            "access_token": row.access_token,
            "email": row.email,
            "type": "openai",
            "status": status,
            "quota": row.quota.unwrap_or(0),
            "image_quota_unknown": true,
            "image_schedulable": image_schedulable,
            "panda_receive_state": if receive_state.is_empty() { Value::Null } else { json!(receive_state) },
            "proxy": row.proxy,
            "proxy_egress_ip": proxy_egress_ip,
            "proxy_provider": proxy_provider,
            "cf_daily": cf_daily,
            "egress_daily": egress_daily,
            "success": 0,
            "fail": 0,
            "created_at": null,
        })
    }

    pub async fn list(&self, offset: usize, limit: usize) -> Value {
        let guard = self.inner.read().await;
        let scheduling = self.scheduling.read().await;
        let mut all: Vec<Value> = guard
            .values()
            .map(|row| Self::row_to_json(row, &scheduling))
            .collect();
        all.sort_by(|a, b| {
            let ea = a.get("email").and_then(|v| v.as_str()).unwrap_or("");
            let eb = b.get("email").and_then(|v| v.as_str()).unwrap_or("");
            ea.cmp(eb)
        });
        let stats = compute_stats(&all);
        let total = all.len();
        let page = if limit == 0 {
            vec![]
        } else {
            all.into_iter().skip(offset).take(limit).collect()
        };
        json!({
            "items": page,
            "total": total,
            "offset": offset,
            "limit": limit,
            "stats": stats,
        })
    }

    pub async fn set_scheduling_by_token(&self, access_token: &str, enabled: bool) -> Result<Option<Value>> {
        let email = {
            let guard = self.inner.read().await;
            guard
                .values()
                .find(|row| row.access_token == access_token)
                .map(|row| row.email.to_lowercase())
        };
        let Some(email) = email else {
            return Ok(None);
        };
        self.set_scheduling_email(&email, enabled).await?;
        let guard = self.inner.read().await;
        let scheduling = self.scheduling.read().await;
        let row = guard.get(&email).context("account disappeared")?;
        Ok(Some(Self::row_to_json(row, &scheduling)))
    }

    pub async fn set_scheduling_bulk(&self, access_tokens: &[String], enabled: bool) -> Result<usize> {
        let emails: Vec<String> = {
            let guard = self.inner.read().await;
            access_tokens
                .iter()
                .filter_map(|token| {
                    guard.values().find(|row| row.access_token == *token).map(|row| {
                        row.email.to_lowercase()
                    })
                })
                .collect()
        };
        let mut updated = 0usize;
        for email in emails {
            self.set_scheduling_email(&email, enabled).await?;
            updated += 1;
        }
        Ok(updated)
    }

    async fn set_scheduling_email(&self, email: &str, enabled: bool) -> Result<()> {
        let key = email.to_lowercase();
        let next_state = if enabled {
            STATE_VERIFIED.to_string()
        } else {
            STATE_ISOLATED.to_string()
        };
        {
            let mut scheduling = self.scheduling.write().await;
            scheduling.insert(key, next_state);
            save_scheduling_state(&self.scheduling_path, &scheduling)?;
        }
        Ok(())
    }

    pub async fn schedulable_breakdown(&self) -> Value {
        let guard = self.inner.read().await;
        let scheduling = self.scheduling.read().await;
        let mut excluded_by_status = 0u64;
        let mut excluded_by_failure_evidence = 0u64;
        let mut excluded_by_receive_state = 0u64;
        let mut schedulable = 0u64;
        for row in guard.values() {
            let receive_state = Self::receive_state_for(&row.email, &scheduling);
            let has_token = !row.access_token.is_empty();
            let status = if has_token { "正常" } else { "异常" };
            let manual_on = Self::manual_scheduling_enabled(&receive_state);
            if status != "正常" {
                excluded_by_status += 1;
            } else if !has_token {
                excluded_by_failure_evidence += 1;
            } else if !manual_on {
                excluded_by_receive_state += 1;
            } else {
                schedulable += 1;
            }
        }
        json!({
            "buckets": {
                "excluded_by_status": excluded_by_status,
                "excluded_by_failure_evidence": excluded_by_failure_evidence,
                "excluded_by_receive_state": excluded_by_receive_state,
                "excluded_by_quota": 0,
                "excluded_by_quota_freshness": 0,
                "excluded_by_dup_binding": 0,
                "excluded_by_dup_egress": 0,
                "excluded_by_interval": 0,
                "excluded_by_backoff": 0,
                "excluded_by_inflight": 0,
                "schedulable": schedulable,
                "ready_not_dispatchable": 0,
            },
            "total": guard.len(),
        })
    }

    pub async fn import_payloads(&self, payloads: Vec<Value>) -> Result<ImportSummary> {
        let mut summary = ImportSummary::default();
        {
            let mut guard = self.inner.write().await;
            for value in payloads {
                let Some(row) = parse_import_item(&value) else {
                    continue;
                };
                let key = row.email.to_lowercase();
                if let Some(existing) = guard.get(&key) {
                    if existing.access_token == row.access_token {
                        summary.skipped += 1;
                        continue;
                    }
                    summary.updated += 1;
                } else {
                    summary.added += 1;
                }
                guard.insert(key, row);
            }
            save_accounts_file(&self.pool_path, &guard)?;
        }
        Ok(summary)
    }

    pub async fn export_items(&self, access_tokens: &[String]) -> Vec<Value> {
        let guard = self.inner.read().await;
        let filter: Option<std::collections::HashSet<&str>> = if access_tokens.is_empty() {
            None
        } else {
            Some(access_tokens.iter().map(String::as_str).collect())
        };
        let mut rows: Vec<Value> = guard
            .values()
            .filter(|row| {
                filter
                    .as_ref()
                    .map(|set| set.contains(row.access_token.as_str()))
                    .unwrap_or(true)
            })
            .map(|row| {
                json!({
                    "email": row.email,
                    "access_token": row.access_token,
                    "proxy": row.proxy,
                    "type": "openai",
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            let ea = a.get("email").and_then(|v| v.as_str()).unwrap_or("");
            let eb = b.get("email").and_then(|v| v.as_str()).unwrap_or("");
            ea.cmp(eb)
        });
        rows
    }

    pub async fn usage_recent(&self, days: usize) -> Value {
        let days = days.clamp(1, 14);
        let tz = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let today = chrono::Utc::now().with_timezone(&tz).date_naive();
        let dates: Vec<String> = (0..days)
            .rev()
            .map(|offset| {
                (today - chrono::Days::new(offset as u64))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .collect();
        let guard = self.inner.read().await;
        let since = today - chrono::Days::new((days + 7) as u64);
        let events = crate::usage_metrics::read_events_public(since).unwrap_or_default();
        let mut by_email = serde_json::Map::new();
        for row in guard.values() {
            let mail = row.email.to_lowercase();
            let mut day_map: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
            for date in &dates {
                day_map.insert(date.clone(), (0, 0, 0, 0));
            }
            for event in &events {
                if event.email.to_lowercase() != mail {
                    continue;
                }
                let date = event.ts.get(0..10).unwrap_or("").to_string();
                let entry = day_map.entry(date).or_insert((0, 0, 0, 0));
                match event.metric.as_str() {
                    "dialogues_nurture" => entry.3 += 1,
                    "dialogues_real" => entry.2 += 1,
                    "images_chat" => {
                        entry.1 += 1;
                        entry.0 += 1;
                    }
                    _ => entry.0 += 1,
                }
            }
            let series: Vec<Value> = dates
                .iter()
                .map(|date| {
                    let (images, images_chat, dialogues_real, dialogues_nurture) =
                        day_map.get(date).copied().unwrap_or((0, 0, 0, 0));
                    json!({
                        "date": date,
                        "images": images,
                        "images_api": images,
                        "images_chat": images_chat,
                        "dialogues": dialogues_real + dialogues_nurture,
                        "dialogues_real": dialogues_real,
                        "dialogues_nurture": dialogues_nurture,
                    })
                })
                .collect();
            by_email.insert(mail, Value::Array(series));
        }
        json!({
            "days": days,
            "dates": dates,
            "by_email": by_email,
        })
    }

    pub async fn email_to_binding_map(&self) -> HashMap<String, String> {
        let guard = self.inner.read().await;
        guard
            .values()
            .map(|row| {
                let email = row.email.to_lowercase();
                let binding = crate::usage_metrics::binding_key_for_proxy(
                    row.proxy.as_deref(),
                    row.proxy_egress_ip.as_deref(),
                );
                (email, binding)
            })
            .collect()
    }

    pub async fn merge_remote_items(&self, items: &[Value]) -> Result<ImportSummary> {
        self.import_payloads(items.to_vec()).await
    }

    pub async fn export_account_for_token(&self, access_token: &str) -> Option<Value> {
        let guard = self.inner.read().await;
        let row = guard.values().find(|r| r.access_token == access_token)?;
        Some(self.account_file_to_export(row))
    }

    fn account_file_to_export(&self, row: &AccountFile) -> Value {
        json!({
            "email": row.email,
            "access_token": row.access_token,
            "proxy": row.proxy,
            "proxy_egress_ip": row.proxy_egress_ip,
            "proxy_provider": row.proxy_provider,
            "cf_daily": row.cf_daily,
            "egress_daily": row.egress_daily,
            "refresh_token": row.refresh_token,
            "id_token": row.id_token,
            "password": row.password,
            "quota": row.quota,
            "status": row.stored_status,
            "device_id": row.device_id,
            "user_agent": row.user_agent,
            "type": "openai",
        })
    }
}

fn compute_stats(items: &[Value]) -> Value {
    let total = items.len();
    let mut normal = 0usize;
    let mut abnormal = 0usize;
    let mut schedulable = 0usize;
    for item in items {
        let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "正常" {
            normal += 1;
        } else {
            abnormal += 1;
        }
        if item.get("image_schedulable").and_then(|v| v.as_bool()).unwrap_or(false) {
            schedulable += 1;
        }
    }
    json!({
        "total": total,
        "active": normal,
        "limited": 0,
        "abnormal": abnormal,
        "disabled": 0,
        "total_quota": 0,
        "schedulable": schedulable,
        "scheduling_enabled": schedulable,
        "image_schedulable": schedulable,
        "available_image_quota": 0,
        "verified_quota_count": 0,
        "stale_quota_count": total,
    })
}

pub fn activity_daily(days: usize) -> Value {
    let days = days.clamp(1, 90);
    let today = chrono::Utc::now().date_naive();
    let mut items = Vec::with_capacity(days);
    for i in (0..days).rev() {
        let date = today - chrono::Days::new(i as u64);
        items.push(json!({
            "date": date.format("%Y-%m-%d").to_string(),
            "registered": 0,
            "uploaded": 0,
            "received": 0,
            "deleted": 0,
            "images": 0,
            "images_api": 0,
            "images_chat": 0,
            "dialogues": 0,
            "dialogues_real": 0,
            "dialogues_nurture": 0,
        }));
    }
    json!({
        "days": days,
        "sync_label": "本地",
        "items": items,
    })
}

fn pool_file_path() -> PathBuf {
    std::env::var("ACCOUNTS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/accounts_pool.json"))
}

fn parse_import_item(value: &Value) -> Option<AccountFile> {
    let token = value
        .get("access_token")
        .or_else(|| value.get("accessToken"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let email = value
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let suffix = token.chars().take(8).collect::<String>();
            format!("import-{suffix}@local")
        });
    let proxy = value
        .get("proxy")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let proxy_egress_ip = value
        .get("proxy_egress_ip")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let proxy_provider = value
        .get("proxy_provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let cf_daily = value
        .get("cf_daily")
        .and_then(|v| v.as_array())
        .map(|a| a.clone())
        .unwrap_or_default();
    let egress_daily = value
        .get("egress_daily")
        .and_then(|v| v.as_array())
        .map(|a| a.clone())
        .unwrap_or_default();
    let refresh_token = value
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let id_token = value
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let password = value
        .get("password")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let quota = value.get("quota").and_then(|v| v.as_i64());
    let stored_status = value
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let device_id = value
        .get("device_id")
        .or_else(|| value.get("oai-device-id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let user_agent = value
        .get("user_agent")
        .or_else(|| value.get("user-agent"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    Some(AccountFile {
        email,
        access_token: token.to_string(),
        proxy,
        proxy_egress_ip,
        proxy_provider,
        cf_daily,
        egress_daily,
        refresh_token,
        id_token,
        password,
        quota,
        stored_status,
        device_id,
        user_agent,
    })
}

fn save_accounts_file(path: &Path, map: &HashMap<String, AccountFile>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    let mut rows: Vec<&AccountFile> = map.values().collect();
    rows.sort_by(|a, b| a.email.cmp(&b.email));
    let owned: Vec<AccountFile> = rows.into_iter().cloned().collect();
    let raw = serde_json::to_string_pretty(&owned).context("serialize accounts pool")?;
    fs::write(path, raw).with_context(|| format!("write accounts pool {:?}", path))?;
    Ok(())
}

fn scheduling_state_path() -> PathBuf {
    std::env::var("SCHEDULING_STATE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/scheduling_state.json"))
}

fn load_scheduling_state(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read scheduling state {:?}", path))?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let parsed: SchedulingStateFile = serde_json::from_str(&raw).context("parse scheduling state")?;
    Ok(parsed.by_email)
}

fn save_scheduling_state(path: &Path, map: &HashMap<String, String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    let payload = SchedulingStateFile {
        by_email: map.clone(),
    };
    let raw = serde_json::to_string_pretty(&payload).context("serialize scheduling state")?;
    fs::write(path, raw).with_context(|| format!("write scheduling state {:?}", path))?;
    Ok(())
}

fn load_single(path: PathBuf) -> Result<AccountFile> {
    let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    serde_json::from_str(&raw).context("parse PIN_ACCOUNT_FILE")
}

fn load_accounts_file(path: PathBuf) -> Result<Vec<AccountFile>> {
    let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    serde_json::from_str(&raw).context("parse ACCOUNTS_FILE")
}
