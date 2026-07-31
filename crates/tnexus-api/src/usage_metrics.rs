//! Usage events + binding-slot heatmaps (gptimage-compatible shape).

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, FixedOffset, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static APPEND_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub ts: String,
    pub email: String,
    pub binding: String,
    pub metric: String,
    pub ok: bool,
}

fn events_path() -> PathBuf {
    std::env::var("USAGE_EVENTS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/usage_events.ndjson"))
}

pub fn record_event(event: &UsageEvent) -> Result<()> {
    let path = events_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    let line = serde_json::to_string(event).context("serialize usage event")?;
    let _guard = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open usage events {:?}", path))?;
    writeln!(file, "{line}").context("append usage event")?;
    Ok(())
}

fn read_events(since: NaiveDate) -> Result<Vec<UsageEvent>> {
    let path = events_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: UsageEvent = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let date = event.ts.get(0..10).unwrap_or("");
        if let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            if d >= since {
                out.push(event);
            }
        }
    }
    Ok(out)
}

fn blank_binding_payload() -> Value {
    json!({
        "images_api": empty_matrix(),
        "images_chat": empty_matrix(),
        "dialogues_real": empty_matrix(),
        "dialogues_nurture": empty_matrix(),
    })
}

fn empty_matrix() -> Vec<Vec<i64>> {
    (0..7).map(|_| vec![0i64; 12]).collect()
}

fn week_bounds(week_offset: i64, tz: FixedOffset) -> (NaiveDate, NaiveDate) {
    let today = Utc::now().with_timezone(&tz).date_naive();
    let weekday = today.weekday().num_days_from_monday() as i64;
    let this_monday = today - Duration::days(weekday);
    let monday = this_monday - Duration::weeks(week_offset);
    (monday, monday + Duration::days(6))
}

fn slot_index_for_week(
    iso_time: &str,
    week_start: NaiveDate,
    week_end: NaiveDate,
    tz: FixedOffset,
) -> Option<(usize, usize)> {
    let raw = iso_time.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(raw)
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|ndt| ndt.and_utc().fixed_offset())
        })
        .ok()?;
    let local = parsed.with_timezone(&tz);
    let date = local.date_naive();
    if date < week_start || date > week_end {
        return None;
    }
    let day = (date - week_start).num_days() as usize;
    if day >= 7 {
        return None;
    }
    let hour_slot = (local.hour() as usize / 2).min(11);
    Some((day, hour_slot))
}

pub fn binding_key_for_proxy(proxy: Option<&str>, egress_ip: Option<&str>) -> String {
    if let Some(ip) = egress_ip.map(str::trim).filter(|s| !s.is_empty()) {
        return ip.to_string();
    }
    let raw = proxy.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("");
    if raw.is_empty() {
        return "default".to_string();
    }
    let stripped = raw
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("socks5://");
    let host_part = stripped.split('/').next().unwrap_or(stripped);
    let host_part = host_part.split('@').last().unwrap_or(host_part);
    if host_part.is_empty() {
        "default".to_string()
    } else {
        host_part.to_string()
    }
}

pub fn get_binding_usage_slots(
    email_to_binding: &HashMap<String, String>,
    week_offset: i64,
    timezone: &str,
) -> Result<Value> {
    let tz = FixedOffset::east_opt(8 * 3600).context("tz")?;
    let (week_start, week_end) = week_bounds(week_offset, tz);
    let events = read_events(week_start - Duration::days(7))?;

    let mut by_binding: HashMap<String, Value> = HashMap::new();

    for event in events {
        let binding = if !event.binding.is_empty() {
            event.binding.clone()
        } else {
            email_to_binding
                .get(&event.email.to_lowercase())
                .cloned()
                .unwrap_or_else(|| "default".into())
        };
        let slot = slot_index_for_week(&event.ts, week_start, week_end, tz);
        let Some((day, hour_slot)) = slot else { continue };
        let payload = by_binding
            .entry(binding)
            .or_insert_with(blank_binding_payload);
        let metric = if event.metric.is_empty() {
            "images_api".to_string()
        } else {
            event.metric.clone()
        };
        if let Some(matrix) = payload.get_mut(&metric).and_then(|v| v.as_array_mut()) {
            if let Some(row) = matrix.get_mut(day).and_then(|r| r.as_array_mut()) {
                if let Some(cell) = row.get_mut(hour_slot).and_then(|c| c.as_i64()) {
                    row[hour_slot] = json!(cell + 1);
                }
            }
        }
    }

    Ok(json!({
        "week_offset": week_offset,
        "week_start": week_start.format("%Y-%m-%d").to_string(),
        "week_end": week_end.format("%Y-%m-%d").to_string(),
        "week_label": format!("{} ~ {}", week_start.format("%m-%d"), week_end.format("%m-%d")),
        "weekday_labels": ["一","二","三","四","五","六","日"],
        "timezone": timezone,
        "timezone_label": "Asia/Shanghai",
        "by_binding": by_binding,
    }))
}

pub fn bump_cf_daily(path: &Path, email: &str, kind: &str) -> Result<()> {
    #[derive(Default, Serialize, Deserialize)]
    struct ObsFile {
        #[serde(default)]
        by_email: HashMap<String, Value>,
    }
    let key = email.to_lowercase();
    let mut file = ObsFile::default();
    if path.exists() {
        let raw = fs::read_to_string(path)?;
        if !raw.trim().is_empty() {
            file = serde_json::from_str(&raw).unwrap_or_default();
        }
    }
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let entry = file.by_email.entry(key).or_insert_with(|| json!({ "cf_daily": [] }));
    let days = entry
        .get_mut("cf_daily")
        .and_then(|v| v.as_array_mut())
        .context("cf_daily array")?;
    let row = days
        .iter_mut()
        .find(|d| d.get("date").and_then(|v| v.as_str()) == Some(&today));
    if let Some(day_value) = row {
        let obj = day_value.as_object_mut().context("cf day object")?;
        let field = match kind {
            "cf" => "cf",
            "image_fail" => "image_fail",
            _ => "ok",
        };
        let cur = obj.get(field).and_then(|v| v.as_u64()).unwrap_or(0);
        obj.insert(field.to_string(), json!(cur + 1));
    } else {
        let mut obj = serde_json::Map::new();
        obj.insert("date".into(), json!(today));
        obj.insert("ok".into(), json!(if kind == "ok" { 1 } else { 0 }));
        obj.insert("cf".into(), json!(if kind == "cf" { 1 } else { 0 }));
        obj.insert("image_fail".into(), json!(if kind == "image_fail" { 1 } else { 0 }));
        days.push(Value::Object(obj));
    }
    while days.len() > 14 {
        days.remove(0);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&file)?)?;
    Ok(())
}

pub fn observability_path() -> PathBuf {
    std::env::var("ACCOUNT_OBSERVABILITY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/account_observability.json"))
}

pub fn read_events_public(since: NaiveDate) -> Result<Vec<UsageEvent>> {
    read_events(since)
}

pub fn load_observability_by_email() -> HashMap<String, Value> {
    let path = observability_path();
    if !path.exists() {
        return HashMap::new();
    }
    let Ok(raw) = fs::read_to_string(&path) else {
        return HashMap::new();
    };
    #[derive(Deserialize)]
    struct ObsFile {
        #[serde(default)]
        by_email: HashMap<String, Value>,
    }
    serde_json::from_str::<ObsFile>(&raw)
        .map(|f| f.by_email)
        .unwrap_or_default()
}
