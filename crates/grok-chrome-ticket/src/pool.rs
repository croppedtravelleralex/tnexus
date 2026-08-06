//! Chrome 票池（对齐 Go `application/chrometicket/pool.go` + 关系库实现
//! `relational/chrome_ticket_repository.go` 的语义）。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

use crate::domain::{
    ttl_bucket, AccountCount, PushInput, Stats, Ticket, TicketSummary, DEFAULT_TICKET_TTL,
    STATUS_AVAILABLE, STATUS_CONSUMED, STATUS_EXPIRED,
};

/// Chrome 票池错误（对齐 Go `repository.ErrNotFound` 等哨兵）。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TicketError {
    #[error("chrome ticket pool not enabled")]
    PoolNotEnabled,
    #[error("ticket not found")]
    NotFound,
    #[error("account_id invalid")]
    InvalidAccount,
    #[error("statsig_meta cannot be empty")]
    EmptyMeta,
    #[error("ticket store error: {0}")]
    Store(String),
}

/// 票池持久化（对齐 Go `repository.ChromeTicketRepository` 接口）。
#[async_trait]
pub trait ChromeTicketRepository: Send + Sync {
    /// 入池一张新票（校验 meta 非空、account_id > 0；TTL <= 0 时用默认 12h）。
    async fn push(&self, input: PushInput) -> Result<Ticket, TicketError>;
    /// 为指定账号取最早可用票并原子标记 consumed；无票返回 `NotFound`。
    async fn pop_for_account(&self, account_id: i64) -> Result<Ticket, TicketError>;
    /// 将过期可用票标记为 expired，返回受影响数量。
    async fn sweep_expired(&self, now: DateTime<Utc>) -> Result<i64, TicketError>;
    /// 状态汇总（内部先清扫）。
    async fn stats(&self, now: DateTime<Utc>) -> Result<Stats, TicketError>;
    /// 列出可用票（按过期时刻升序），limit <= 0 时默认 200。
    async fn list_available(
        &self,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Ticket>, TicketError>;
}

/// 票池门面（Go `Pool`）。
pub struct Pool {
    repo: Arc<dyn ChromeTicketRepository>,
    ttl: Duration,
}

impl Pool {
    /// 使用默认 TTL（12h）。
    pub fn new(repo: Arc<dyn ChromeTicketRepository>) -> Self {
        Self {
            repo,
            ttl: DEFAULT_TICKET_TTL,
        }
    }

    /// 自定义默认 TTL。
    pub fn with_ttl(repo: Arc<dyn ChromeTicketRepository>, ttl: Duration) -> Self {
        Self { repo, ttl }
    }

    /// 入池（Go `Push`）：TTL <= 0 时按 Pool 默认兜底。
    pub async fn push(&self, mut input: PushInput) -> Result<Ticket, TicketError> {
        if input.ttl <= Duration::zero() {
            input.ttl = self.ttl;
        }
        self.repo.push(input).await
    }

    /// 为指定账号取票（Go `PopForAccount`）。
    pub async fn pop_for_account(&self, account_id: i64) -> Result<Ticket, TicketError> {
        self.repo.pop_for_account(account_id).await
    }

    /// 过期清扫（Go `Sweep`）。
    pub async fn sweep(&self) -> Result<i64, TicketError> {
        self.repo.sweep_expired(Utc::now()).await
    }

    /// 状态汇总（Go `Stats`）。
    pub async fn stats(&self) -> Result<Stats, TicketError> {
        self.repo.stats(Utc::now()).await
    }
}

/// 从结构化字段构建入池请求（Go `NormalizePushInputFromFields`）。
pub fn normalize_push_input_from_fields(
    account_id: i64,
    meta: impl Into<String>,
    device_cookie: impl Into<String>,
    user_agent: impl Into<String>,
    sign_source: impl Into<String>,
    ttl: Duration,
) -> PushInput {
    PushInput {
        account_id,
        statsig_meta: meta.into(),
        device_cookie: device_cookie.into(),
        user_agent: user_agent.into(),
        sign_source: sign_source.into(),
        ttl,
    }
}

/// 兼容 Python minter 字段命名的入池请求解析（Go `NormalizePushInput`）。
///
/// 字段别名：`account_id`/`accountId`、`statsig_meta`/`statsigMeta`、
/// `cookie`/`device_cookie`；均做 trim。
pub fn normalize_push_input(
    raw: &serde_json::Map<String, serde_json::Value>,
    ttl: Duration,
) -> Result<PushInput, TicketError> {
    let account_id = raw
        .get("account_id")
        .or_else(|| raw.get("accountId"))
        .and_then(uint64_value)
        .ok_or(TicketError::InvalidAccount)?;
    if account_id == 0 {
        return Err(TicketError::InvalidAccount);
    }
    let meta = raw
        .get("statsig_meta")
        .or_else(|| raw.get("statsigMeta"))
        .and_then(string_value)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if meta.is_empty() {
        return Err(TicketError::EmptyMeta);
    }
    let device_cookie = raw
        .get("cookie")
        .or_else(|| raw.get("device_cookie"))
        .and_then(string_value)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    Ok(PushInput {
        account_id,
        statsig_meta: meta,
        device_cookie,
        user_agent: raw
            .get("user_agent")
            .and_then(string_value)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        sign_source: raw
            .get("sign_source")
            .and_then(string_value)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        ttl,
    })
}

fn string_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// 正整数值提取（Go `uint64Value`）：数字（int/uint/float）且 > 0。
fn uint64_value(value: &serde_json::Value) -> Option<i64> {
    let n = value
        .as_i64()
        .or_else(|| value.as_u64().map(|u| u as i64))?;
    if n > 0 {
        Some(n)
    } else {
        None
    }
}

// ── 内存实现（测试 / 单实例兜底）──────────────────────────────────

/// 内存票池实现：线程安全，单进程内有效（对齐 Go 关系库语义，供测试用）。
#[derive(Default)]
pub struct MemoryChromeTicketRepository {
    tickets: std::sync::Mutex<Vec<Ticket>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl MemoryChromeTicketRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ChromeTicketRepository for MemoryChromeTicketRepository {
    async fn push(&self, input: PushInput) -> Result<Ticket, TicketError> {
        let meta = input.statsig_meta.trim();
        if meta.is_empty() {
            return Err(TicketError::EmptyMeta);
        }
        if input.account_id <= 0 {
            return Err(TicketError::InvalidAccount);
        }
        let ttl = if input.ttl <= Duration::zero() {
            DEFAULT_TICKET_TTL
        } else {
            input.ttl
        };
        let now = Utc::now();
        let id = format!(
            "mem-{:032x}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        let ticket = Ticket {
            id,
            account_id: input.account_id,
            statsig_meta: meta.to_string(),
            device_cookie: input.device_cookie.trim().to_string(),
            user_agent: input.user_agent.trim().to_string(),
            sign_source: input.sign_source.trim().to_string(),
            created_at: now,
            expires_at: now + ttl,
            consumed_at: None,
            status: STATUS_AVAILABLE.to_string(),
        };
        self.tickets.lock().unwrap().push(ticket.clone());
        Ok(ticket)
    }

    async fn pop_for_account(&self, account_id: i64) -> Result<Ticket, TicketError> {
        if account_id <= 0 {
            return Err(TicketError::InvalidAccount);
        }
        let mut tickets = self.tickets.lock().unwrap();
        let now = Utc::now();
        sweep_locked(&mut tickets, now);
        let index = tickets
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.status == STATUS_AVAILABLE && t.expires_at >= now && t.account_id == account_id
            })
            .min_by_key(|(_, t)| t.created_at)
            .map(|(i, _)| i);
        let Some(index) = index else {
            return Err(TicketError::NotFound);
        };
        let mut ticket = tickets[index].clone();
        ticket.status = STATUS_CONSUMED.to_string();
        ticket.consumed_at = Some(now);
        tickets[index] = ticket.clone();
        Ok(ticket)
    }

    async fn sweep_expired(&self, now: DateTime<Utc>) -> Result<i64, TicketError> {
        let mut tickets = self.tickets.lock().unwrap();
        Ok(sweep_locked(&mut tickets, now))
    }

    async fn stats(&self, now: DateTime<Utc>) -> Result<Stats, TicketError> {
        let mut tickets = self.tickets.lock().unwrap();
        let now = now.with_timezone(&Utc);
        sweep_locked(&mut tickets, now);
        let mut stats = Stats::default();
        for ticket in tickets.iter() {
            *stats.by_status.entry(ticket.status.clone()).or_insert(0) += 1;
        }
        let mut by_account: HashMap<i64, i64> = HashMap::new();
        for ticket in tickets.iter().filter(|t| t.status == STATUS_AVAILABLE) {
            *by_account.entry(ticket.account_id).or_insert(0) += 1;
        }
        let mut account_rows: Vec<AccountCount> = by_account
            .into_iter()
            .map(|(account_id, count)| AccountCount { account_id, count })
            .collect();
        account_rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.account_id.cmp(&b.account_id)));
        stats.available_by_account = account_rows.into_iter().take(20).collect();
        let available = list_available_locked(&tickets, now, 500);
        for ticket in available {
            let remaining = (ticket.expires_at - now).num_seconds().max(0);
            stats.available_tickets.push(TicketSummary {
                id: ticket.id,
                account_id: ticket.account_id,
                created_at: ticket.created_at,
                expires_at: ticket.expires_at,
                ttl_remaining_seconds: remaining,
                sign_source: ticket.sign_source,
            });
            *stats
                .ttl_distribution
                .entry(ttl_bucket(remaining).to_string())
                .or_insert(0) += 1;
            if stats
                .earliest_expires_at
                .is_none_or(|earliest| ticket.expires_at < earliest)
            {
                stats.earliest_expires_at = Some(ticket.expires_at);
            }
        }
        if let Some(earliest) = stats.earliest_expires_at {
            stats.earliest_expires_in_sec = (earliest - now).num_seconds().max(0);
        }
        Ok(stats)
    }

    async fn list_available(
        &self,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Ticket>, TicketError> {
        let limit = if limit == 0 { 200 } else { limit };
        let tickets = self.tickets.lock().unwrap();
        Ok(list_available_locked(&tickets, now, limit))
    }
}

/// 将过期可用票标记 expired，返回受影响数（Go `sweepExpiredTickets`）。
fn sweep_locked(tickets: &mut [Ticket], now: DateTime<Utc>) -> i64 {
    let mut swept = 0;
    for ticket in tickets.iter_mut() {
        if ticket.status == STATUS_AVAILABLE && ticket.expires_at < now {
            ticket.status = STATUS_EXPIRED.to_string();
            swept += 1;
        }
    }
    swept
}

/// 可用票列表（按 expires_at 升序、created_at 升序，Go `ListAvailable`）。
fn list_available_locked(tickets: &[Ticket], now: DateTime<Utc>, limit: usize) -> Vec<Ticket> {
    let mut available: Vec<Ticket> = tickets
        .iter()
        .filter(|t| t.status == STATUS_AVAILABLE && t.expires_at >= now)
        .cloned()
        .collect();
    available.sort_by(|a, b| {
        a.expires_at
            .cmp(&b.expires_at)
            .then(a.created_at.cmp(&b.created_at))
    });
    available.truncate(limit);
    available
}
