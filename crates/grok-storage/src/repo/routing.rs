//! 路由候选查询辅助（对齐 Go `account_repository.go` 的 hydrateRoutingCandidates）。
//!
//! 只含 PG 行映射与批量查询 helper；真正的时序（排序、first-window 选择、bound 过滤）
//! 由 `repo/account.rs` 的 `list_routing_candidates*` 调用本模块函数拼装。

use std::collections::HashMap;

use grok_domain::{
    Billing, ModelQuotaBlock, ModelState, ModelStatus, QuotaRecovery, QuotaRecoveryKind,
    QuotaRecoveryStatus, QuotaSource, QuotaWindow, WebTier,
};
use sqlx::{postgres::PgRow, PgPool, Row};

use crate::StorageError;

#[allow(dead_code)]
pub(crate) fn web_tier_from_str(s: &str) -> WebTier {
    match s {
        "super" => WebTier::Super,
        "heavy" => WebTier::Heavy,
        _ => WebTier::Basic,
    }
}

pub(crate) fn quota_source_from_str(s: &str) -> QuotaSource {
    match s {
        "estimated" => QuotaSource::Estimated,
        "upstream" => QuotaSource::Upstream,
        _ => QuotaSource::Default,
    }
}

pub(crate) fn quota_recovery_kind_from_str(s: &str) -> Result<QuotaRecoveryKind, StorageError> {
    match s {
        "free" => Ok(QuotaRecoveryKind::Free),
        "paid" => Ok(QuotaRecoveryKind::Paid),
        other => Err(StorageError::Decode(format!(
            "unknown quota recovery kind: {other}"
        ))),
    }
}

pub(crate) fn quota_recovery_status_from_str(s: &str) -> Result<QuotaRecoveryStatus, StorageError> {
    match s {
        "active" => Ok(QuotaRecoveryStatus::Active),
        "exhausted" => Ok(QuotaRecoveryStatus::Exhausted),
        "probing" => Ok(QuotaRecoveryStatus::Probing),
        other => Err(StorageError::Decode(format!(
            "unknown quota recovery status: {other}"
        ))),
    }
}

pub(crate) fn model_status_from_str(s: &str) -> Result<ModelStatus, StorageError> {
    match s {
        "unknown" => Ok(ModelStatus::Unknown),
        "quota_available" => Ok(ModelStatus::QuotaAvailable),
        "available" => Ok(ModelStatus::Available),
        "soft_stop" => Ok(ModelStatus::SoftStop),
        "quota_exhausted" => Ok(ModelStatus::QuotaExhausted),
        "auth_failed" => Ok(ModelStatus::AuthFailed),
        "signature_failed" => Ok(ModelStatus::SignatureFailed),
        other => Err(StorageError::Decode(format!(
            "unknown model status: {other}"
        ))),
    }
}

fn trim_opt(row: &PgRow, col: &str) -> Result<Option<String>, StorageError> {
    Ok(row
        .try_get::<Option<String>, _>(col)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

pub(crate) fn to_quota_window(row: &PgRow) -> Result<QuotaWindow, StorageError> {
    let source_str: String = row.try_get("source")?;
    Ok(QuotaWindow {
        account_id: row.try_get("account_id")?,
        mode: row.try_get("mode")?,
        remaining: row.try_get("remaining")?,
        total: row.try_get("total")?,
        reset_at: row.try_get("reset_at")?,
        synced_at: row.try_get("synced_at")?,
        source: quota_source_from_str(&source_str),
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn to_quota_recovery(row: &PgRow) -> Result<QuotaRecovery, StorageError> {
    let kind: String = row.try_get("kind")?;
    let status: String = row.try_get("status")?;
    Ok(QuotaRecovery {
        account_id: row.try_get("account_id")?,
        kind: quota_recovery_kind_from_str(&kind)?,
        status: quota_recovery_status_from_str(&status)?,
        confirmed_used: row.try_get("confirmed_used")?,
        confirmed_limit: row.try_get("confirmed_limit")?,
        exhausted_at: row.try_get("exhausted_at")?,
        next_probe_at: row.try_get("next_probe_at")?,
        last_confirmed_at: row.try_get("last_confirmed_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn to_billing(row: &PgRow) -> Result<Billing, StorageError> {
    Ok(Billing {
        account_id: row.try_get("account_id")?,
        plan_code: row.try_get("plan_code")?,
        plan_name: row.try_get("plan_name")?,
        monthly_limit: row.try_get("monthly_limit")?,
        used: row.try_get("used")?,
        on_demand_cap: row.try_get("on_demand_cap")?,
        on_demand_used: row.try_get("on_demand_used")?,
        prepaid_balance: row.try_get("prepaid_balance")?,
        credit_usage_percent: row.try_get("credit_usage_percent")?,
        is_unified_billing_user: row.try_get("is_unified_billing_user")?,
        top_up_method: row.try_get("top_up_method")?,
        usage_period_type: row.try_get("usage_period_type")?,
        usage_period_start: row.try_get("usage_period_start")?,
        usage_period_end: row.try_get("usage_period_end")?,
        billing_period_start: row.try_get("billing_period_start")?,
        billing_period_end: row.try_get("billing_period_end")?,
        synced_at: row.try_get("synced_at")?,
    })
}

pub(crate) fn to_model_state(row: &PgRow) -> Result<ModelState, StorageError> {
    let status: String = row.try_get("status")?;
    Ok(ModelState {
        account_id: row.try_get("account_id")?,
        upstream_model: row.try_get("upstream_model")?,
        status: model_status_from_str(&status)?,
        reason: trim_opt(row, "reason")?,
        consecutive_failures: row.try_get("consecutive_failures")?,
        last_attempt_at: row.try_get("last_attempt_at")?,
        last_success_at: row.try_get("last_success_at")?,
        cooldown_until: row.try_get("cooldown_until")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn to_model_quota_block(row: &PgRow) -> Result<ModelQuotaBlock, StorageError> {
    Ok(ModelQuotaBlock {
        account_id: row.try_get("account_id")?,
        upstream_model: row.try_get("upstream_model")?,
        reason: row.try_get("reason")?,
        cooldown_until: row.try_get("cooldown_until")?,
        updated_at: row.try_get("updated_at")?,
    })
}

// ── 批量查询（hydrate 用）────────────────────────────────────────

/// 批量读额度恢复快照（Go `GetQuotaRecoveries`）。
pub(crate) async fn fetch_quota_recoveries(
    pool: &PgPool,
    ids: &[i64],
) -> Result<HashMap<i64, QuotaRecovery>, StorageError> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(
        "SELECT account_id, kind, status, confirmed_used, confirmed_limit, \
                exhausted_at, next_probe_at, last_confirmed_at, updated_at \
         FROM grok_quota_recovery WHERE account_id = ANY($1::bigint[])",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    for row in rows {
        let value = to_quota_recovery(&row)?;
        out.insert(value.account_id, value);
    }
    Ok(out)
}

/// 批量读账单快照（Go `GetBillings`）。
pub(crate) async fn fetch_billings(
    pool: &PgPool,
    ids: &[i64],
) -> Result<HashMap<i64, Billing>, StorageError> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(
        "SELECT account_id, plan_code, plan_name, monthly_limit, used, on_demand_cap, \
                on_demand_used, prepaid_balance, credit_usage_percent, is_unified_billing_user, \
                top_up_method, usage_period_type, usage_period_start, usage_period_end, \
                billing_period_start, billing_period_end, synced_at \
         FROM grok_billing_snapshots WHERE account_id = ANY($1::bigint[])",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    for row in rows {
        let value = to_billing(&row)?;
        out.insert(value.account_id, value);
    }
    Ok(out)
}

/// 批量读模型状态（Go `GetModelStates`）。
pub(crate) async fn fetch_model_states(
    pool: &PgPool,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<ModelState>>, StorageError> {
    let mut out: HashMap<i64, Vec<ModelState>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(
        "SELECT account_id, upstream_model, status, reason, consecutive_failures, \
                last_attempt_at, last_success_at, cooldown_until, updated_at \
         FROM grok_model_states WHERE account_id = ANY($1::bigint[]) \
         ORDER BY account_id ASC, upstream_model ASC",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    for row in rows {
        let state = to_model_state(&row)?;
        out.entry(state.account_id).or_default().push(state);
    }
    Ok(out)
}

/// 读取路由候选目标模式的额度窗口，按模式优先级取每账号首个（Go `quotaWindows` 组装）。
///
/// ``modes` 签名：web 恒含 `weekly`；`quota_mode` 非空时追加；imagine 优先（独立 allowance）。
/// SQL 以 `CASE` 排序保证每账号首个为最高优先级模式，Rust 端 dedupe。
pub(crate) async fn fetch_quota_windows_first(
    pool: &PgPool,
    ids: &[i64],
    quota_mode: &str,
    want_weekly: bool,
) -> Result<HashMap<i64, QuotaWindow>, StorageError> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut modes = Vec::new();
    if want_weekly {
        modes.push("weekly".to_string());
    }
    if !quota_mode.trim().is_empty() {
        modes.push(quota_mode.trim().to_string());
    }
    if modes.is_empty() {
        return Ok(out);
    }
    let order = if quota_mode.trim() == "imagine" {
        "CASE WHEN mode = 'imagine' THEN 0 WHEN mode = 'weekly' THEN 1 ELSE 2 END ASC"
    } else {
        "CASE WHEN mode = 'weekly' THEN 0 ELSE 1 END ASC"
    };
    let sql = format!(
        "SELECT account_id, mode, remaining::bigint AS remaining, total::bigint AS total, reset_at, synced_at, source, updated_at \
         FROM grok_quota_windows WHERE account_id = ANY($1::bigint[]) AND mode = ANY($2::text[]) \
         ORDER BY {order}"
    );
    let rows = sqlx::query(&sql)
        .bind(ids)
        .bind(&modes)
        .fetch_all(pool)
        .await?;
    for row in rows {
        let account_id: i64 = row.try_get("account_id")?;
        if out.contains_key(&account_id) {
            continue;
        }
        let window = to_quota_window(&row)?;
        out.insert(account_id, window);
    }
    Ok(out)
}
