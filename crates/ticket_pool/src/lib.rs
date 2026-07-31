//! In-memory Sentinel ticket pool for Rust gateway orchestration.
//!
//! Production default remains **per-call finalize**; pooling is optional and
//! conservative (TTL 300s from validation evidence). Immediate / gap reuse are
//! feature-flag placeholders only — reversible without schema change.
//!
//! # Why no `Serialize` / `Deserialize`
//!
//! [`SentinelTicket`] holds four plaintext upstream tokens. Deriving serde or a
//! naive `Debug` would let a single `{:?}` or a JSON log line dump the whole
//! pool's credentials. Tokens are therefore accessible only through explicit
//! accessors, and `Debug` is hand-written to redact them.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

/// Default ticket TTL — aligned with `gptimage` validation (reuse-gap + longevity).
pub const DEFAULT_TICKET_TTL_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReusePolicy {
    /// Production path: one finalize per image SSE. Pooled acquire is refused.
    PerCallFinalize,
    /// Placeholder — ablation showed immediate 2× SSE often CF403.
    ImmediateReuseExperimental,
    /// Placeholder — gap reuse (60s ok, 120s mixed); not enabled by default.
    GapReuseExperimental,
}

impl ReusePolicy {
    /// Whether a deposited ticket may be handed out again.
    ///
    /// `PerCallFinalize` means exactly one finalize per SSE, so a pooled
    /// acquire would violate it — only the experimental policies allow reuse.
    pub fn allows_pooled_acquire(self) -> bool {
        !matches!(self, ReusePolicy::PerCallFinalize)
    }
}

#[derive(Clone)]
pub struct SentinelTicket {
    id: Uuid,
    account_key: String,
    requirements_token: String,
    proof_token: String,
    turnstile_token: String,
    so_token: String,
    opened_at_unix: u64,
    egress_ip: Option<String>,
    consumed: bool,
}

impl std::fmt::Debug for SentinelTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SentinelTicket")
            .field("id", &self.id)
            .field("account_key", &self.account_key)
            .field("opened_at_unix", &self.opened_at_unix)
            .field("egress_ip", &self.egress_ip)
            .field("consumed", &self.consumed)
            .field("tokens", &"<redacted>")
            .finish()
    }
}

impl SentinelTicket {
    pub fn new(
        account_key: impl Into<String>,
        requirements_token: impl Into<String>,
        proof_token: impl Into<String>,
        turnstile_token: impl Into<String>,
        so_token: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            account_key: account_key.into(),
            requirements_token: requirements_token.into(),
            proof_token: proof_token.into(),
            turnstile_token: turnstile_token.into(),
            so_token: so_token.into(),
            opened_at_unix: now_unix(),
            egress_ip: None,
            consumed: false,
        }
    }

    pub fn with_egress(mut self, ip: impl Into<String>) -> Self {
        self.egress_ip = Some(ip.into());
        self
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn account_key(&self) -> &str {
        &self.account_key
    }

    pub fn requirements_token(&self) -> &str {
        &self.requirements_token
    }

    pub fn proof_token(&self) -> &str {
        &self.proof_token
    }

    pub fn turnstile_token(&self) -> &str {
        &self.turnstile_token
    }

    pub fn so_token(&self) -> &str {
        &self.so_token
    }

    pub fn egress_ip(&self) -> Option<&str> {
        self.egress_ip.as_deref()
    }

    pub fn is_consumed(&self) -> bool {
        self.consumed
    }

    pub fn age_secs(&self) -> u64 {
        now_unix().saturating_sub(self.opened_at_unix)
    }

    pub fn is_expired(&self, ttl_secs: u64) -> bool {
        self.age_secs() > ttl_secs
    }

    /// Test-only: backdate the ticket to exercise TTL paths.
    #[cfg(test)]
    fn backdate(mut self, secs: u64) -> Self {
        self.opened_at_unix = self.opened_at_unix.saturating_sub(secs);
        self
    }
}

#[derive(Debug, Clone)]
pub struct TicketPoolConfig {
    pub ttl_secs: u64,
    pub max_per_account: usize,
    pub reuse_policy: ReusePolicy,
}

impl Default for TicketPoolConfig {
    fn default() -> Self {
        Self {
            ttl_secs: DEFAULT_TICKET_TTL_SECS,
            max_per_account: 2,
            reuse_policy: ReusePolicy::PerCallFinalize,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RefreshStats {
    pub expired_removed: usize,
    pub consumed_removed: usize,
    pub remaining: usize,
}

#[derive(Debug, Error)]
pub enum TicketPoolError {
    #[error("ticket expired (age={age_secs}s ttl={ttl_secs}s)")]
    Expired { age_secs: u64, ttl_secs: u64 },
    #[error("ticket already consumed")]
    AlreadyConsumed,
    #[error("reuse policy forbids pooled acquire: {0:?}")]
    ReuseForbidden(ReusePolicy),
    #[error("no ticket for account {account_key}")]
    NotFound { account_key: String },
}

/// Thread-unsafe in-memory pool; gateway wraps with `Mutex` if shared.
#[derive(Debug, Default)]
pub struct TicketPool {
    cfg: TicketPoolConfig,
    by_id: HashMap<Uuid, SentinelTicket>,
    by_account: HashMap<String, Vec<Uuid>>,
}

impl TicketPool {
    pub fn new(cfg: TicketPoolConfig) -> Self {
        Self {
            cfg,
            by_id: HashMap::new(),
            by_account: HashMap::new(),
        }
    }

    /// 入票 — store a freshly finalized ticket, evicting the oldest past capacity.
    pub fn deposit(&mut self, ticket: SentinelTicket) -> Uuid {
        let id = ticket.id;
        let acct = ticket.account_key.clone();
        self.by_id.insert(id, ticket);
        let list = self.by_account.entry(acct).or_default();
        list.push(id);
        while list.len() > self.cfg.max_per_account {
            let old = list.remove(0);
            self.by_id.remove(&old);
        }
        id
    }

    /// 刷票 — purge expired / consumed entries.
    pub fn refresh(&mut self) -> RefreshStats {
        let ttl = self.cfg.ttl_secs;
        let mut expired = 0usize;
        let mut consumed = 0usize;

        self.by_id.retain(|_, t| {
            if t.consumed {
                consumed += 1;
                false
            } else if t.is_expired(ttl) {
                expired += 1;
                false
            } else {
                true
            }
        });

        let live: std::collections::HashSet<Uuid> = self.by_id.keys().copied().collect();
        self.by_account.retain(|_, ids| {
            ids.retain(|id| live.contains(id));
            !ids.is_empty()
        });

        RefreshStats {
            expired_removed: expired,
            consumed_removed: consumed,
            remaining: self.by_id.len(),
        }
    }

    /// 取票 — acquire the youngest valid ticket for an account, marking it consumed.
    ///
    /// Refused under [`ReusePolicy::PerCallFinalize`], which mandates one
    /// finalize per SSE and therefore forbids handing a ticket out twice.
    pub fn acquire(&mut self, account_key: &str) -> Result<SentinelTicket, TicketPoolError> {
        if !self.cfg.reuse_policy.allows_pooled_acquire() {
            return Err(TicketPoolError::ReuseForbidden(self.cfg.reuse_policy));
        }
        self.refresh();
        let ttl = self.cfg.ttl_secs;
        let id = self
            .by_account
            .get(account_key)
            .and_then(|ids| {
                ids.iter().rev().find(|id| {
                    self.by_id
                        .get(id)
                        .is_some_and(|t| !t.consumed && !t.is_expired(ttl))
                })
            })
            .copied()
            .ok_or_else(|| TicketPoolError::NotFound {
                account_key: account_key.to_string(),
            })?;

        let ticket = self
            .by_id
            .get_mut(&id)
            .expect("id came from by_account after refresh");
        ticket.consumed = true;
        Ok(ticket.clone())
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn config(&self) -> &TicketPoolConfig {
        &self.cfg
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reusable_cfg(ttl_secs: u64) -> TicketPoolConfig {
        TicketPoolConfig {
            ttl_secs,
            reuse_policy: ReusePolicy::GapReuseExperimental,
            ..Default::default()
        }
    }

    fn ticket(acct: &str) -> SentinelTicket {
        SentinelTicket::new(acct, "req", "pow", "turn", "so")
    }

    #[test]
    fn deposit_and_acquire_within_ttl() {
        let mut pool = TicketPool::new(reusable_cfg(300));
        let id = pool.deposit(ticket("a@x.com"));
        assert_eq!(pool.len(), 1);
        let got = pool.acquire("a@x.com").expect("acquire");
        assert_eq!(got.id(), id);
        assert!(got.is_consumed());
    }

    #[test]
    fn per_call_finalize_refuses_pooled_acquire() {
        let mut pool = TicketPool::new(TicketPoolConfig::default());
        pool.deposit(ticket("a@x.com"));
        let err = pool.acquire("a@x.com").unwrap_err();
        assert!(matches!(
            err,
            TicketPoolError::ReuseForbidden(ReusePolicy::PerCallFinalize)
        ));
    }

    #[test]
    fn refresh_drops_expired() {
        let mut pool = TicketPool::new(reusable_cfg(60));
        pool.deposit(ticket("a@x.com").backdate(120));
        let stats = pool.refresh();
        assert_eq!(stats.expired_removed, 1);
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn refresh_prunes_account_index() {
        let mut pool = TicketPool::new(reusable_cfg(60));
        pool.deposit(ticket("a@x.com").backdate(120));
        pool.refresh();
        let err = pool.acquire("a@x.com").unwrap_err();
        assert!(matches!(err, TicketPoolError::NotFound { .. }));
    }

    #[test]
    fn acquire_missing_account() {
        let mut pool = TicketPool::new(reusable_cfg(300));
        let err = pool.acquire("nobody@x.com").unwrap_err();
        assert!(matches!(err, TicketPoolError::NotFound { .. }));
    }

    #[test]
    fn acquire_twice_is_refused() {
        let mut pool = TicketPool::new(reusable_cfg(300));
        pool.deposit(ticket("a@x.com"));
        pool.acquire("a@x.com").expect("first acquire");
        let err = pool.acquire("a@x.com").unwrap_err();
        assert!(matches!(err, TicketPoolError::NotFound { .. }));
    }

    #[test]
    fn deposit_evicts_oldest_past_capacity() {
        let mut pool = TicketPool::new(TicketPoolConfig {
            max_per_account: 2,
            ..reusable_cfg(300)
        });
        let first = pool.deposit(ticket("a@x.com"));
        pool.deposit(ticket("a@x.com"));
        pool.deposit(ticket("a@x.com"));
        assert_eq!(pool.len(), 2);
        let survivors = pool.by_account.get("a@x.com").expect("account index");
        assert!(!survivors.contains(&first), "oldest should be evicted");
    }

    #[test]
    fn debug_redacts_tokens() {
        let rendered = format!("{:?}", ticket("a@x.com"));
        assert!(rendered.contains("<redacted>"));
        for secret in ["req", "pow", "turn", "so"] {
            assert!(
                !rendered.contains(&format!("\"{secret}\"")),
                "token {secret} leaked into Debug output"
            );
        }
    }

    #[test]
    fn pool_debug_redacts_tokens() {
        let mut pool = TicketPool::new(reusable_cfg(300));
        pool.deposit(ticket("a@x.com"));
        let rendered = format!("{pool:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("\"pow\""));
    }
}
