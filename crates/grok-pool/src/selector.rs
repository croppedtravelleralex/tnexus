//! 账号选择器（对齐 Go `gateway/selector.go`）。
//!
//! [`Selector::acquire`] 在候选账号中按 配额闸门 → 模型能力 → 冷却 → 排序 逐层过滤，
//! 输出一个并发租约（`SelectionLease`，drop 自动释放）。
//!
//! IO 全部抽象为 trait（候选加载 / 并发限制 / 粘滞 / 调度索引 / 票池 / 档位序），
//! 测试注入 fake（Go `selector_test.go` 用 SQLite repo + memory runtime）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use grok_domain::{
    Account, AuthStatus, Billing, ModelQuotaBlock, ModelState, ModelStatus, Provider,
    QuotaRecovery, QuotaRecoveryKind, QuotaRecoveryStatus, RoutingCandidate, WebLane, WebTier,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Web lite 生图上游模型名（Go `webLiteImageUpstreamModel`）。
pub const WEB_LITE_IMAGE_UPSTREAM_MODEL: &str = "grok-imagine-image";
/// 配额探针租约时长（Go `quotaProbeLease`）。
pub const QUOTA_PROBE_LEASE: Duration = Duration::minutes(5);
/// 成功落库间隔（Go `successPersistInterval`）。
pub const SUCCESS_PERSIST_INTERVAL: Duration = Duration::seconds(30);
/// 候选快照 TTL（Go `candidateCacheTTL`）。
pub const CANDIDATE_CACHE_TTL: Duration = Duration::seconds(1);
/// Build 索引水合初批 / 上限（Go `buildDispatchHydrateInitial/Max`）。
pub const BUILD_DISPATCH_HYDRATE_INITIAL: usize = 64;
pub const BUILD_DISPATCH_HYDRATE_MAX: usize = 256;
/// Build 普通池探针水合上限（Go `buildNormalProbeHydrateLimit`）。
pub const BUILD_NORMAL_PROBE_HYDRATE_LIMIT: usize = 32;
/// 模型成功加权的有效期（Go `modelOutcomeSuccessTTL`）。
pub const MODEL_OUTCOME_SUCCESS_TTL: Duration = Duration::minutes(30);
/// 模型 soft-stop 退避基值 / 上限（Go `modelSoftStopBaseCooldown/MaxCooldown`）。
pub const MODEL_SOFT_STOP_BASE_COOLDOWN: Duration = Duration::seconds(30);
pub const MODEL_SOFT_STOP_MAX_COOLDOWN: Duration = Duration::minutes(5);
/// 模型 outcome 保留期（Go `modelOutcomeRetention`）。
pub const MODEL_OUTCOME_RETENTION: Duration = Duration::hours(1);
/// Imagine 额度新鲜 TTL（Go `imagineQuotaFreshTTL`）。
pub const IMAGINE_QUOTA_FRESH_TTL: Duration = Duration::minutes(30);
/// 默认探索率（Go `defaultExplorationEpsilon`）。
pub const DEFAULT_EXPLORATION_EPSILON: f64 = 0.05;
/// Billing 新鲜 TTL（sortCandidates 用）。
pub const BILLING_FRESH_TTL: Duration = Duration::minutes(30);

/// 选号失败原因（Go `SelectionUnavailableReason`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionUnavailableReason {
    NoAccounts,
    NoDispatchIndex,
    PinFiltered,
    UnsupportedModel,
    Cooling,
    ModelCooling,
    QuotaExhausted,
    QuotaStale,
    Saturated,
    NoChromeTickets,
}

impl SelectionUnavailableReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SelectionUnavailableReason::NoAccounts => "no_accounts",
            SelectionUnavailableReason::NoDispatchIndex => "no_dispatch_index",
            SelectionUnavailableReason::PinFiltered => "pin_filtered",
            SelectionUnavailableReason::UnsupportedModel => "unsupported_model",
            SelectionUnavailableReason::Cooling => "cooling",
            SelectionUnavailableReason::ModelCooling => "model_cooling",
            SelectionUnavailableReason::QuotaExhausted => "quota_exhausted",
            SelectionUnavailableReason::QuotaStale => "quota_stale",
            SelectionUnavailableReason::Saturated => "saturated",
            SelectionUnavailableReason::NoChromeTickets => "no_chrome_tickets",
        }
    }
}

/// 选号不可用错误（保留真实原因，避免一律 503）。
#[derive(Debug, Clone)]
pub struct SelectionUnavailableError {
    pub reason: SelectionUnavailableReason,
    pub retry_after: Duration,
}

impl std::fmt::Display for SelectionUnavailableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self.reason {
            SelectionUnavailableReason::UnsupportedModel => "当前账号池不支持该模型",
            SelectionUnavailableReason::Cooling => "可用上游账号正在冷却",
            SelectionUnavailableReason::ModelCooling => "可用上游账号的目标模型正在冷却",
            SelectionUnavailableReason::QuotaExhausted => "可用上游账号额度等待恢复",
            SelectionUnavailableReason::QuotaStale => "可用上游账号 Imagine 额度未同步或已过期",
            SelectionUnavailableReason::Saturated => "可用上游账号均达到并发上限",
            SelectionUnavailableReason::NoChromeTickets => "Chrome 票池暂无可用票据",
            _ => "没有可用上游账号",
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for SelectionUnavailableError {}

/// selector 错误。
#[derive(Debug, Error)]
pub enum SelectorError {
    #[error("selection unavailable: {0}")]
    Unavailable(#[from] SelectionUnavailableError),
    #[error("candidate loader error: {0}")]
    Loader(String),
    #[error("concurrency error: {0}")]
    Concurrency(String),
    #[error("sticky store error: {0}")]
    Sticky(String),
    #[error("lease store error: {0}")]
    Store(String),
}

pub type SelectorResult<T> = Result<T, SelectorError>;

/// 一次成功选号获得的并发租约（drop 自动释放并发槽位）。
pub struct SelectionLease {
    pub account: Account,
    pub quota_probe: bool,
    pub quota_probe_kind: Option<QuotaRecoveryKind>,
    pub billing: Option<Billing>,
    pub quota_mode: Option<String>,
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for SelectionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionLease")
            .field("account_id", &self.account.id)
            .field("quota_probe", &self.quota_probe)
            .field("quota_mode", &self.quota_mode)
            .finish_non_exhaustive()
    }
}

impl SelectionLease {
    /// 显式释放（Go `Release`）；drop 时同样释放，双保险。
    pub fn release(mut self) {
        self.take_release();
    }

    fn take_release(&mut self) {
        if let Some(f) = self.release.take() {
            f();
        }
    }
}

impl Drop for SelectionLease {
    fn drop(&mut self) {
        self.take_release();
    }
}

// ── IO trait ─────────────────────────────────────────────────────

/// 候选加载 + 选号副作用（对齐 Go `AccountRepository` 的相关方法）。
#[async_trait]
pub trait CandidateLoader: Send + Sync {
    async fn list_routing_candidates(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
    ) -> SelectorResult<Vec<RoutingCandidate>>;
    async fn list_routing_candidates_by_ids(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        ids: &[i64],
    ) -> SelectorResult<Vec<RoutingCandidate>>;
    async fn claim_quota_probe(
        &self,
        account_id: i64,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> SelectorResult<bool>;
    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        reason: &str,
        reset_last_success: bool,
    ) -> SelectorResult<()>;
    async fn clear_quota_recovery(&self, id: i64) -> SelectorResult<()>;
    async fn save_quota_recovery(&self, recovery: QuotaRecovery) -> SelectorResult<()>;
    async fn save_model_state(&self, state: ModelState) -> SelectorResult<()>;
    async fn save_model_quota_block(&self, block: ModelQuotaBlock) -> SelectorResult<()>;
}

/// 并发限制器（Go `ConcurrencyLimiter`）。
#[async_trait]
pub trait ConcurrencyLimiter: Send + Sync {
    /// 尝试获取租约；`Ok(None)` = 已满。
    async fn acquire(
        &self,
        key: &str,
        limit: i32,
    ) -> SelectorResult<Option<Box<dyn FnOnce() + Send>>>;
    async fn current(&self, key: &str) -> SelectorResult<i32>;
    /// 批量读当前并发（Go `ConcurrencySnapshotReader.CurrentMany`），按账号 id 返回。
    async fn current_many(&self, ids: &[i64]) -> SelectorResult<HashMap<i64, i32>>;
}

/// 会话粘滞（Go `StickySessionRepository`）。
#[async_trait]
pub trait StickyStore: Send + Sync {
    async fn get(&self, key: &str, now: DateTime<Utc>) -> SelectorResult<Option<i64>>;
    async fn set(
        &self,
        key: &str,
        account_id: i64,
        expires_at: DateTime<Utc>,
    ) -> SelectorResult<()>;
    async fn delete_by_account(&self, account_id: i64) -> SelectorResult<()>;
}

/// Build 调度池有序索引（Go `buildDispatchSource`）。
#[async_trait]
pub trait BuildDispatchSource: Send + Sync {
    fn ordered_dispatch_ids(&self, limit: usize) -> Vec<i64>;
    fn due_normal_probe_ids(&self, now: DateTime<Utc>, limit: usize) -> Vec<i64>;
    fn note_dispatch_selected(&self, id: i64, at: DateTime<Utc>);
    async fn ensure_warm(&self) -> SelectorResult<()>;
}

/// Web 双轨调度池有序索引（Go `webDispatchSource`）。
#[async_trait]
pub trait WebDispatchSource: Send + Sync {
    fn ordered_web_dispatch_ids(&self, lane: WebLane, limit: usize) -> Vec<i64>;
    fn note_web_dispatch_selected(&self, lane: WebLane, id: i64, at: DateTime<Utc>);
    async fn ensure_web_warm(&self) -> SelectorResult<()>;
}

/// Chrome 票池（Go `ChromeTicketSource`）。
pub trait ChromeTicketSource: Send + Sync {
    fn available_counts(&self) -> HashMap<i64, i64>;
}

/// Web 档位序（Go `TierOrder`）。
pub trait TierOrderSource: Send + Sync {
    fn tier_order(&self, provider: Provider, upstream_model: &str) -> Vec<WebTier>;
}

// ── 纯函数 ───────────────────────────────────────────────────────

/// prompt cache key 的粘滞键：sha256 hex（64 字符）；空 key 保持空（Go `promptCacheStickyKey`）。
pub fn prompt_cache_sticky_key(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 合并 dispatch 与普通探针候选 ID，去重、跳过 0（Go `mergeBuildHydrateIDs`）。
pub fn merge_build_hydrate_ids(dispatch_ids: &[i64], probe_ids: &[i64]) -> Vec<i64> {
    merge_ids(dispatch_ids, probe_ids)
}

/// 票持有者优先合并（Go `mergeDispatchIDs`）。
pub fn merge_dispatch_ids(preferred: &[i64], current: &[i64]) -> Vec<i64> {
    if preferred.is_empty() {
        return current.to_vec();
    }
    merge_ids(preferred, current)
}

fn merge_ids(first: &[i64], second: &[i64]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::with_capacity(first.len() + second.len());
    for id in first.iter().chain(second.iter()) {
        if *id == 0 || !seen.insert(*id) {
            continue;
        }
        merged.push(*id);
    }
    merged
}

/// 账号单模型并发上限（Go `accountConcurrencyLimit`）：lite image 恒 1。
pub fn account_concurrency_limit(account: &Account, upstream_model: &str) -> i32 {
    if upstream_model
        .trim()
        .eq_ignore_ascii_case(WEB_LITE_IMAGE_UPSTREAM_MODEL)
    {
        return 1;
    }
    if account.max_concurrent > 0 {
        return account.max_concurrent;
    }
    8 // Go `DefaultMaxConcurrent`
}

/// 单模型额度 block 是否生效（Go `candidateModelQuotaBlocked`）。
pub fn candidate_model_quota_blocked(candidate: &RoutingCandidate, now: DateTime<Utc>) -> bool {
    let Some(block) = &candidate.model_quota_block else {
        return false;
    };
    if !(now < block.cooldown_until) {
        return false;
    }
    match &candidate.quota {
        None => true,
        Some(w) => w.total <= 0 || w.remaining <= 0,
    }
}

/// 生效的额度模式（Go `effectiveQuotaMode`）：weekly 窗口优先。
pub fn effective_quota_mode(candidate: &RoutingCandidate, fallback: &str) -> Option<String> {
    if candidate.quota.as_ref().is_some_and(|w| w.mode == "weekly") {
        return Some("weekly".to_string());
    }
    if fallback.is_empty() {
        None
    } else {
        Some(fallback.to_string())
    }
}

/// 是否需要 imagine 额度准入（Go `requiresImagineQuotaAdmission`）。
pub fn requires_imagine_quota_admission(upstream_model: &str, quota_mode: &str) -> bool {
    upstream_model
        .trim()
        .eq_ignore_ascii_case(WEB_LITE_IMAGE_UPSTREAM_MODEL)
        || quota_mode.trim() == "imagine"
}

/// 是否需要 Chrome 票准入（Go `requiresChromeTicketAdmission`）。
pub fn requires_chrome_ticket_admission(upstream_model: &str) -> bool {
    upstream_model
        .trim()
        .eq_ignore_ascii_case(WEB_LITE_IMAGE_UPSTREAM_MODEL)
}

/// imagine 额度是否可准入（Go `candidateImagineQuotaAdmissible`）。
pub fn candidate_imagine_quota_admissible(
    candidate: &RoutingCandidate,
    now: DateTime<Utc>,
) -> bool {
    grok_domain::imagine_quota::imagine_dispatch_quota_admissible(
        candidate.quota.as_ref(),
        candidate.model_state.as_ref(),
        now,
    )
}

/// 取未来更早的时刻（Go `earlierFuture`）。
pub fn earlier_future(
    current: DateTime<Utc>,
    candidate: DateTime<Utc>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    if candidate == DateTime::<Utc>::UNIX_EPOCH || !(now < candidate) {
        return current;
    }
    if current == DateTime::<Utc>::UNIX_EPOCH || candidate < current {
        return candidate;
    }
    current
}

/// 距 retry 时刻的等待时长（Go `retryDelay`）。
pub fn retry_delay(now: DateTime<Utc>, retry_at: DateTime<Utc>) -> Duration {
    if retry_at == DateTime::<Utc>::UNIX_EPOCH || !(now < retry_at) {
        return Duration::zero();
    }
    retry_at - now
}

/// Fisher-Yates 洗牌（Go `shuffleRoutingCandidates`）。
pub fn shuffle_routing_candidates(
    values: &mut [RoutingCandidate],
    mut random: impl FnMut() -> f64,
) {
    for i in (1..values.len()).rev() {
        let mut j = (random() * (i + 1) as f64) as usize;
        if j > i {
            j = i;
        }
        values.swap(i, j);
    }
}

/// 探索洗牌（Go `maybeExploreShuffle`）。
pub fn maybe_explore_shuffle(
    values: &mut [RoutingCandidate],
    epsilon: f64,
    mut random: impl FnMut() -> f64,
) {
    if values.len() <= 1 || random() >= epsilon {
        return;
    }
    shuffle_routing_candidates(values, random);
}

/// 档位排序秩（Go `tierOrderRank`）。
pub fn tier_order_rank(order: &[WebTier], tier: WebTier) -> usize {
    order.iter().position(|t| *t == tier).unwrap_or(order.len())
}

// ── 排序上下文 ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
struct ModelOutcome {
    last_success_at: DateTime<Utc>,
    last_soft_stop_at: DateTime<Utc>,
    soft_stop_until: DateTime<Utc>,
    consecutive_soft_stop: i32,
}

/// 排序所需的批量快照（对齐 Go `sortCandidates` 的收集阶段）。
#[derive(Debug, Default)]
pub struct SortContext {
    pub last_selected_at: HashMap<i64, DateTime<Utc>>,
    pub model_ranks: HashMap<i64, i32>,
    pub ticket_counts: HashMap<i64, i64>,
    pub in_flight: HashMap<i64, i32>,
    pub billing_remaining: HashMap<i64, f64>,
    pub billing_fresh: HashMap<i64, bool>,
    pub imagine_remaining: HashMap<i64, i64>,
    pub imagine_fresh: HashMap<i64, bool>,
}

/// 候选排序比较器（对齐 Go `sortCandidates` 的 `sort.SliceStable`）。
pub fn compare_candidates(
    left: &RoutingCandidate,
    right: &RoutingCandidate,
    ctx: &SortContext,
    tier_order: &[WebTier],
    upstream_model: &str,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let l = &left.account;
    let r = &right.account;

    if left.supports_model != right.supports_model {
        return left.supports_model.cmp(&right.supports_model).reverse();
    }
    if left.model_capability_known != right.model_capability_known {
        return left
            .model_capability_known
            .cmp(&right.model_capability_known)
            .reverse();
    }
    if l.provider == Provider::GrokBuild && r.provider == Provider::GrokBuild {
        let lv = !l.observed_model.as_deref().unwrap_or("").trim().is_empty();
        let rv = !r.observed_model.as_deref().unwrap_or("").trim().is_empty();
        if lv != rv {
            return lv.cmp(&rv).reverse();
        }
    }
    let lt = tier_order_rank(tier_order, l.web_tier);
    let rt = tier_order_rank(tier_order, r.web_tier);
    if lt != rt {
        return lt.cmp(&rt);
    }
    let lr = ctx.model_ranks.get(&l.id).copied().unwrap_or(1);
    let rr = ctx.model_ranks.get(&r.id).copied().unwrap_or(1);
    if lr != rr {
        return lr.cmp(&rr);
    }
    if upstream_model
        .trim()
        .eq_ignore_ascii_case(WEB_LITE_IMAGE_UPSTREAM_MODEL)
    {
        let (lk, lc) = ctx
            .ticket_counts
            .get(&l.id)
            .map(|c| (true, *c))
            .unwrap_or((false, 0));
        let (rk, rc) = ctx
            .ticket_counts
            .get(&r.id)
            .map(|c| (true, *c))
            .unwrap_or((false, 0));
        if lk != rk {
            return lk.cmp(&rk).reverse();
        }
        if lc != rc {
            return lc.cmp(&rc).reverse();
        }
        let lf = ctx.imagine_fresh.get(&l.id).copied().unwrap_or(false);
        let rf = ctx.imagine_fresh.get(&r.id).copied().unwrap_or(false);
        if lf != rf {
            return lf.cmp(&rf).reverse();
        }
        let (lok, lq) = ctx
            .imagine_remaining
            .get(&l.id)
            .map(|q| (true, *q))
            .unwrap_or((false, 0));
        let (rok, rq) = ctx
            .imagine_remaining
            .get(&r.id)
            .map(|q| (true, *q))
            .unwrap_or((false, 0));
        if lok != rok {
            return lok.cmp(&rok).reverse();
        }
        if lq != rq {
            return lq.cmp(&rq).reverse();
        }
    }
    if l.priority != r.priority {
        return l.priority.cmp(&r.priority).reverse();
    }
    let lf = ctx.billing_fresh.get(&l.id).copied().unwrap_or(false);
    let rf = ctx.billing_fresh.get(&r.id).copied().unwrap_or(false);
    if lf != rf {
        return lf.cmp(&rf).reverse();
    }
    let li = ctx.in_flight.get(&l.id).copied().unwrap_or(0);
    let ri = ctx.in_flight.get(&r.id).copied().unwrap_or(0);
    if li != ri {
        return li.cmp(&ri);
    }
    let lr = ctx.billing_remaining.get(&l.id).copied().unwrap_or(0.0);
    let rr = ctx.billing_remaining.get(&r.id).copied().unwrap_or(0.0);
    if lr != rr {
        return lr.partial_cmp(&rr).unwrap_or(Ordering::Equal).reverse();
    }
    let ls = ctx.last_selected_at.get(&l.id).copied().unwrap_or_default();
    let rs = ctx.last_selected_at.get(&r.id).copied().unwrap_or_default();
    if ls != rs {
        return ls.cmp(&rs);
    }
    l.id.cmp(&r.id)
}

// ── Selector ──────────────────────────────────────────────────────

/// 候选缓存键（provider + upstream_model + quota_mode）。
type CandidateCacheKey = (Provider, String, String);
/// 候选缓存：快照 + 过期时刻。
type CandidateCache = HashMap<CandidateCacheKey, (Vec<RoutingCandidate>, DateTime<Utc>)>;

struct SelectorState {
    last_selected_at: HashMap<i64, DateTime<Utc>>,
    last_success_at: HashMap<i64, DateTime<Utc>>,
    model_outcomes: HashMap<(i64, String), ModelOutcome>,
    candidates: CandidateCache,
    lease_wake: Arc<tokio::sync::Notify>,
}

/// 账号选择器。
pub struct Selector {
    loader: Arc<dyn CandidateLoader>,
    concurrency: Arc<dyn ConcurrencyLimiter>,
    sticky: Option<Arc<dyn StickyStore>>,
    tier_orders: Option<Arc<dyn TierOrderSource>>,
    build_dispatch: Option<Arc<dyn BuildDispatchSource>>,
    web_dispatch: Option<Arc<dyn WebDispatchSource>>,
    chrome_tickets: Option<Arc<dyn ChromeTicketSource>>,
    sticky_ttl: Duration,
    cooldown_base: Duration,
    cooldown_max: Duration,
    capacity_wait: Duration,
    disable_cooldown: bool,
    exploration_epsilon: f64,
    state: Mutex<SelectorState>,
}

impl Selector {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        loader: Arc<dyn CandidateLoader>,
        concurrency: Arc<dyn ConcurrencyLimiter>,
        sticky: Option<Arc<dyn StickyStore>>,
        tier_orders: Option<Arc<dyn TierOrderSource>>,
        sticky_ttl: Duration,
        cooldown_base: Duration,
        cooldown_max: Duration,
        capacity_wait: Duration,
    ) -> Self {
        Self {
            loader,
            concurrency,
            sticky,
            tier_orders,
            build_dispatch: None,
            web_dispatch: None,
            chrome_tickets: None,
            sticky_ttl,
            cooldown_base,
            cooldown_max,
            capacity_wait,
            disable_cooldown: false,
            exploration_epsilon: DEFAULT_EXPLORATION_EPSILON,
            state: Mutex::new(SelectorState {
                last_selected_at: HashMap::new(),
                last_success_at: HashMap::new(),
                model_outcomes: HashMap::new(),
                candidates: HashMap::new(),
                lease_wake: Arc::new(tokio::sync::Notify::new()),
            }),
        }
    }

    pub fn set_build_dispatch_source(&mut self, source: Arc<dyn BuildDispatchSource>) {
        self.build_dispatch = Some(source);
    }

    pub fn set_web_dispatch_source(&mut self, source: Arc<dyn WebDispatchSource>) {
        self.web_dispatch = Some(source);
    }

    pub fn set_chrome_ticket_source(&mut self, source: Arc<dyn ChromeTicketSource>) {
        self.chrome_tickets = Some(source);
    }

    pub fn set_tier_orders(&mut self, source: Arc<dyn TierOrderSource>) {
        self.tier_orders = Some(source);
    }

    pub fn set_exploration_epsilon(&mut self, epsilon: f64) {
        self.exploration_epsilon = epsilon;
    }

    pub fn set_disable_cooldown(&mut self, v: bool) {
        self.disable_cooldown = v;
    }

    fn state(&self) -> std::sync::MutexGuard<'_, SelectorState> {
        self.state.lock().unwrap()
    }

    /// 选号（对齐 Go `Acquire`）。
    #[allow(clippy::too_many_arguments)]
    pub async fn acquire(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        prompt_cache_key: &str,
        excluded: &std::collections::HashSet<i64>,
        allow_quota_probe: bool,
    ) -> SelectorResult<SelectionLease> {
        let now = Utc::now();
        let sticky_key = prompt_cache_sticky_key(prompt_cache_key);
        let values = self
            .load_candidates_for_acquire(provider, upstream_model, quota_mode, now)
            .await?;

        let mut normal_candidates = Vec::new();
        let mut probe_candidates = Vec::new();
        let (mut considered, mut supported, mut cooling, mut model_cooling, mut quota) =
            (0usize, 0usize, 0usize, 0usize, 0usize);
        let mut earliest_retry = DateTime::<Utc>::UNIX_EPOCH;

        for candidate in values {
            let value = &candidate.account;
            if excluded.contains(&value.id) || value.auth_status != AuthStatus::Active {
                continue;
            }
            // Build：未通过能力探测的号不进真实流量（验证池）。
            if provider == Provider::GrokBuild
                && value
                    .observed_model
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                continue;
            }
            considered += 1;
            if candidate.model_capability_known && !candidate.supports_model {
                continue;
            }
            supported += 1;
            if candidate_model_quota_blocked(&candidate, now) {
                model_cooling += 1;
                if let Some(b) = &candidate.model_quota_block {
                    earliest_retry = earlier_future(earliest_retry, b.cooldown_until, now);
                }
                continue;
            }
            if !self.disable_cooldown && value.cooldown_until.is_some_and(|until| now < until) {
                cooling += 1;
                if let Some(until) = value.cooldown_until {
                    earliest_retry = earlier_future(earliest_retry, until, now);
                }
                continue;
            }
            if let Some(recovery) = &candidate.recovery {
                if recovery.status != QuotaRecoveryStatus::Active {
                    if allow_quota_probe && recovery.next_probe_at.is_some_and(|at| !(now < at)) {
                        probe_candidates.push(candidate);
                    } else {
                        quota += 1;
                        if let Some(at) = recovery.next_probe_at {
                            earliest_retry = earlier_future(earliest_retry, at, now);
                        }
                    }
                    continue;
                }
            }
            if candidate
                .billing
                .as_ref()
                .is_some_and(|b| b.is_exhausted(value.minimum_remaining as f64))
            {
                quota += 1;
                continue;
            }
            // total=0/remaining=0 是 free-usage-gates 对部分已成功生图账号的真实返回，
            // 表示该免费闸门不适用或上限未知，不能误判为模型额度耗尽。
            if let Some(w) = &candidate.quota {
                if w.total > 0 && w.remaining <= 0 {
                    quota += 1;
                    if let Some(reset) = w.reset_at {
                        earliest_retry = earlier_future(earliest_retry, reset, now);
                    }
                    continue;
                }
            }
            if requires_imagine_quota_admission(upstream_model, quota_mode)
                && !candidate_imagine_quota_admissible(&candidate, now)
            {
                quota += 1;
                if let Some(w) = &candidate.quota {
                    if let Some(synced) = w.synced_at {
                        earliest_retry =
                            earlier_future(earliest_retry, synced + IMAGINE_QUOTA_FRESH_TTL, now);
                    }
                }
                continue;
            }
            normal_candidates.push(candidate);
        }

        if requires_chrome_ticket_admission(upstream_model) {
            normal_candidates = self.filter_chrome_ticket_candidates(normal_candidates);
        }

        if normal_candidates.is_empty() && probe_candidates.is_empty() {
            let reason = if considered > 0 && supported == 0 {
                SelectionUnavailableReason::UnsupportedModel
            } else if model_cooling > 0 {
                SelectionUnavailableReason::ModelCooling
            } else if cooling > 0 {
                SelectionUnavailableReason::Cooling
            } else if quota > 0 {
                SelectionUnavailableReason::QuotaExhausted
            } else {
                SelectionUnavailableReason::NoAccounts
            };
            return Err(SelectionUnavailableError {
                reason,
                retry_after: retry_delay(now, earliest_retry),
            }
            .into());
        }

        // 配额探针候选优先。
        if !probe_candidates.is_empty() {
            let mut sorted = probe_candidates;
            let tier_order = self.resolve_tier_order(provider, upstream_model);
            self.sort_candidates(&mut sorted, now, &tier_order, upstream_model)
                .await?;
            maybe_explore_shuffle(&mut sorted, self.exploration_epsilon, || self.random_unit());
            for candidate in sorted {
                let Some(mut lease) = self
                    .claim_account_slot(&candidate.account, upstream_model)
                    .await?
                else {
                    continue;
                };
                let claimed = self
                    .loader
                    .claim_quota_probe(candidate.account.id, now, now + QUOTA_PROBE_LEASE)
                    .await?;
                if !claimed {
                    lease.take_release();
                    continue;
                }
                lease.quota_probe = true;
                lease.quota_probe_kind = candidate.recovery.as_ref().map(|r| r.kind);
                lease.billing = candidate.billing.clone();
                return Ok(lease);
            }
        }

        // 粘滞路径。
        if !sticky_key.is_empty() {
            if let Some(sticky) = &self.sticky {
                if let Ok(Some(sticky_id)) = sticky.get(&sticky_key, now).await {
                    for candidate in &normal_candidates {
                        if candidate.account.id == sticky_id {
                            if let Some(mut lease) = self
                                .claim_account_slot(&candidate.account, upstream_model)
                                .await?
                            {
                                self.note_selected(provider, &candidate.account, now);
                                lease.billing = candidate.billing.clone();
                                lease.quota_mode = effective_quota_mode(candidate, quota_mode);
                                return Ok(lease);
                            }
                        }
                    }
                }
            }
        }

        // 常规路径（含容量等待）。
        let wait_deadline = Utc::now() + self.capacity_wait;
        loop {
            if provider == Provider::GrokBuild {
                self.order_build_dispatch_candidates(&mut normal_candidates);
                maybe_explore_shuffle(&mut normal_candidates, self.exploration_epsilon, || {
                    self.random_unit()
                });
            } else {
                let current_time = Utc::now();
                let tier_order = self.resolve_tier_order(provider, upstream_model);
                self.sort_candidates(
                    &mut normal_candidates,
                    current_time,
                    &tier_order,
                    upstream_model,
                )
                .await?;
                maybe_explore_shuffle(&mut normal_candidates, self.exploration_epsilon, || {
                    self.random_unit()
                });
            }
            for candidate in &normal_candidates {
                let Some(mut lease) = self
                    .claim_account_slot(&candidate.account, upstream_model)
                    .await?
                else {
                    continue;
                };
                if !sticky_key.is_empty() {
                    if let Some(sticky) = &self.sticky {
                        if let Err(e) = sticky
                            .set(
                                &sticky_key,
                                candidate.account.id,
                                Utc::now() + self.sticky_ttl,
                            )
                            .await
                        {
                            lease.take_release();
                            return Err(e);
                        }
                    }
                }
                self.note_selected(provider, &candidate.account, Utc::now());
                lease.billing = candidate.billing.clone();
                lease.quota_mode = effective_quota_mode(candidate, quota_mode);
                return Ok(lease);
            }
            if self.capacity_wait <= Duration::zero() {
                return Err(SelectionUnavailableError {
                    reason: SelectionUnavailableReason::Saturated,
                    retry_after: Duration::seconds(1),
                }
                .into());
            }
            if !self.await_lease_retry(wait_deadline).await {
                return Err(SelectionUnavailableError {
                    reason: SelectionUnavailableReason::Saturated,
                    retry_after: Duration::seconds(1),
                }
                .into());
            }
        }
    }

    /// 成功选号后更新调度公平序（Build → dispatch 索引；Web → web 索引）。
    fn note_selected(&self, provider: Provider, account: &Account, at: DateTime<Utc>) {
        match provider {
            Provider::GrokBuild => {
                if let Some(source) = &self.build_dispatch {
                    source.note_dispatch_selected(account.id, at);
                }
            }
            Provider::GrokWeb => {
                if let Some(source) = &self.web_dispatch {
                    source.note_web_dispatch_selected(WebLane::Image, account.id, at);
                }
            }
            Provider::GrokConsole => {}
        }
    }

    /// Build 候选按调度索引序重排（Go `orderBuildDispatchCandidates`）。
    fn order_build_dispatch_candidates(&self, values: &mut [RoutingCandidate]) {
        if values.len() <= 1 {
            return;
        }
        let Some(source) = &self.build_dispatch else {
            return;
        };
        let ordered_ids = source.ordered_dispatch_ids(values.len() + 8);
        if ordered_ids.is_empty() {
            return;
        }
        let by_id: HashMap<i64, RoutingCandidate> =
            values.iter().map(|c| (c.account.id, c.clone())).collect();
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::with_capacity(values.len());
        for id in ordered_ids {
            if let Some(candidate) = by_id.get(&id) {
                if seen.insert(id) {
                    result.push(candidate.clone());
                }
            }
        }
        for candidate in values.iter() {
            if !seen.contains(&candidate.account.id) {
                result.push(candidate.clone());
            }
        }
        values.clone_from_slice(&result);
    }

    /// 候选加载（Go `loadCandidatesForAcquire`）：Build/Web 走调度索引，其它走缓存全表。
    async fn load_candidates_for_acquire(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        now: DateTime<Utc>,
    ) -> SelectorResult<Vec<RoutingCandidate>> {
        match provider {
            Provider::GrokBuild => {
                if let Some(source) = &self.build_dispatch {
                    return self
                        .load_build_candidates_by_index(
                            source.as_ref(),
                            upstream_model,
                            quota_mode,
                            now,
                        )
                        .await;
                }
            }
            Provider::GrokWeb => {
                if let Some(source) = &self.web_dispatch {
                    let lane = WebLane::Image; // G3-P4 固定 Image 轨；Chat 轨后续接入
                    return self
                        .load_web_candidates_by_index(
                            source.as_ref(),
                            lane,
                            upstream_model,
                            quota_mode,
                            now,
                        )
                        .await;
                }
            }
            Provider::GrokConsole => {}
        }
        self.load_candidates(provider, upstream_model, quota_mode, now)
            .await
    }

    /// Build 索引水合（Go `loadBuildCandidatesByIndex`）。
    async fn load_build_candidates_by_index(
        &self,
        source: &dyn BuildDispatchSource,
        upstream_model: &str,
        quota_mode: &str,
        now: DateTime<Utc>,
    ) -> SelectorResult<Vec<RoutingCandidate>> {
        let mut batch = BUILD_DISPATCH_HYDRATE_INITIAL;
        loop {
            let dispatch_ids = source.ordered_dispatch_ids(batch);
            let mut ids = merge_build_hydrate_ids(
                &dispatch_ids,
                &source.due_normal_probe_ids(now, BUILD_NORMAL_PROBE_HYDRATE_LIMIT),
            );
            if ids.is_empty() {
                source.ensure_warm().await?;
                let dispatch_ids = source.ordered_dispatch_ids(batch);
                ids = merge_build_hydrate_ids(
                    &dispatch_ids,
                    &source.due_normal_probe_ids(now, BUILD_NORMAL_PROBE_HYDRATE_LIMIT),
                );
                if ids.is_empty() {
                    return Err(SelectionUnavailableError {
                        reason: SelectionUnavailableReason::NoAccounts,
                        retry_after: Duration::zero(),
                    }
                    .into());
                }
            }
            let values = self
                .loader
                .list_routing_candidates_by_ids(
                    Provider::GrokBuild,
                    upstream_model,
                    quota_mode,
                    &ids,
                )
                .await?;
            if !values.is_empty() {
                return Ok(values);
            }
            if dispatch_ids.len() < batch || batch >= BUILD_DISPATCH_HYDRATE_MAX {
                break;
            }
            batch = (batch * 2).min(BUILD_DISPATCH_HYDRATE_MAX);
        }
        Ok(Vec::new())
    }

    /// Web 索引水合（Go `loadWebCandidatesByIndex`）。
    #[allow(clippy::too_many_arguments)]
    async fn load_web_candidates_by_index(
        &self,
        source: &dyn WebDispatchSource,
        lane: WebLane,
        upstream_model: &str,
        quota_mode: &str,
        _now: DateTime<Utc>,
    ) -> SelectorResult<Vec<RoutingCandidate>> {
        let mut batch = BUILD_DISPATCH_HYDRATE_INITIAL;
        loop {
            let mut dispatch_ids = source.ordered_web_dispatch_ids(lane, batch);
            if upstream_model
                .trim()
                .eq_ignore_ascii_case(WEB_LITE_IMAGE_UPSTREAM_MODEL)
            {
                dispatch_ids = merge_dispatch_ids(&self.ticket_holder_ids(), &dispatch_ids);
            }
            if dispatch_ids.is_empty() {
                source.ensure_web_warm().await?;
                dispatch_ids = source.ordered_web_dispatch_ids(lane, batch);
                if dispatch_ids.is_empty() {
                    return Err(SelectionUnavailableError {
                        reason: SelectionUnavailableReason::NoDispatchIndex,
                        retry_after: Duration::zero(),
                    }
                    .into());
                }
            }
            let values = self
                .loader
                .list_routing_candidates_by_ids(
                    Provider::GrokWeb,
                    upstream_model,
                    quota_mode,
                    &dispatch_ids,
                )
                .await?;
            if !values.is_empty() {
                return Ok(values);
            }
            if dispatch_ids.len() < batch || batch >= BUILD_DISPATCH_HYDRATE_MAX {
                break;
            }
            batch = (batch * 2).min(BUILD_DISPATCH_HYDRATE_MAX);
        }
        Err(SelectionUnavailableError {
            reason: SelectionUnavailableReason::PinFiltered,
            retry_after: Duration::zero(),
        }
        .into())
    }

    /// 缓存全表候选（Go `loadCandidates`，TTL 1s）。
    async fn load_candidates(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        now: DateTime<Utc>,
    ) -> SelectorResult<Vec<RoutingCandidate>> {
        let key = (provider, upstream_model.to_string(), quota_mode.to_string());
        let cached = {
            let st = self.state();
            st.candidates.get(&key).and_then(|(values, expires_at)| {
                if now < *expires_at {
                    Some(values.clone())
                } else {
                    None
                }
            })
        };
        if let Some(values) = cached {
            return Ok(values);
        }
        let values = self
            .loader
            .list_routing_candidates(provider, upstream_model, quota_mode)
            .await?;
        let mut st = self.state();
        st.candidates
            .insert(key, (values.clone(), now + CANDIDATE_CACHE_TTL));
        Ok(values)
    }

    /// 使某 provider 的候选缓存失效（Go `invalidateCandidates`）。
    pub fn invalidate_candidates(&self, provider: Provider) {
        let mut st = self.state();
        st.candidates.retain(|(p, _, _), _| *p != provider);
    }

    /// 成功记账（Go `markSuccess`）。
    pub async fn mark_success(&self, account: &Account, quota_probe: bool) {
        let now = Utc::now();
        let mut persist = account.failure_count > 0
            || account.cooldown_until.is_some()
            || !account.last_error.as_deref().unwrap_or("").is_empty();
        {
            let mut st = self.state();
            // 对齐 Go `markSuccess`：无记录或 epoch 视为 `last.IsZero()` → 持久化；
            // 距上次成功超过间隔也持久化（避免每次刷新 DB）。首次成功必须落库。
            let last = st.last_success_at.get(&account.id).copied();
            if last.is_none()
                || last.is_some_and(|t| t == DateTime::<Utc>::UNIX_EPOCH)
                || now - last.unwrap() >= SUCCESS_PERSIST_INTERVAL
            {
                persist = true;
            }
            if persist {
                st.last_success_at.insert(account.id, now);
            }
        }
        if persist {
            let _ = self
                .loader
                .update_health(account.id, 0, None, "", true)
                .await;
        }
        if quota_probe {
            let _ = self.loader.clear_quota_recovery(account.id).await;
        }
        if quota_probe || persist {
            self.invalidate_candidates(account.provider);
        }
    }

    /// 失败记账（Go `MarkFailure`）：指数退避冷却 + 缓存失效；401/402/403/429 清粘滞。
    pub async fn mark_failure(&self, account: &Account, status: i32, retry_after: Duration) {
        if self.disable_cooldown {
            return;
        }
        let failure_count = account.failure_count + 1;
        let mut cooldown = self.cooldown_base;
        let mut i = 1;
        while i < failure_count && cooldown < self.cooldown_max {
            cooldown = cooldown * 2;
            i += 1;
        }
        if cooldown > self.cooldown_max {
            cooldown = self.cooldown_max;
        }
        if retry_after > cooldown {
            cooldown = retry_after;
        }
        let until = Utc::now() + cooldown;
        let _ = self
            .loader
            .update_health(
                account.id,
                failure_count,
                Some(until),
                &format!("upstream status {status}"),
                false,
            )
            .await;
        self.invalidate_candidates(account.provider);
        if matches!(status, 401 | 402 | 403 | 429) {
            if let Some(sticky) = &self.sticky {
                let _ = sticky.delete_by_account(account.id).await;
            }
        }
    }

    /// 模型 soft-stop 退避（Go `MarkModelSoftStop`）。
    pub async fn mark_model_soft_stop(
        &self,
        account_id: i64,
        upstream_model: &str,
    ) -> SelectorResult<()> {
        if self.disable_cooldown {
            return Ok(());
        }
        let upstream_model = upstream_model.trim().to_string();
        if account_id == 0 || upstream_model.is_empty() {
            return Ok(());
        }
        let now = Utc::now();
        let key = (account_id, upstream_model.clone());
        let (consecutive_soft_stop, soft_stop_until) = {
            let mut st = self.state();
            let mut value = st.model_outcomes.get(&key).copied().unwrap_or_default();
            if value.last_soft_stop_at == DateTime::<Utc>::UNIX_EPOCH
                || now - value.last_soft_stop_at > MODEL_OUTCOME_SUCCESS_TTL
                || value.last_success_at > value.last_soft_stop_at
            {
                value.consecutive_soft_stop = 0;
            }
            value.consecutive_soft_stop += 1;
            let mut cooldown = MODEL_SOFT_STOP_BASE_COOLDOWN;
            let mut count = 1;
            while count < value.consecutive_soft_stop && cooldown < MODEL_SOFT_STOP_MAX_COOLDOWN {
                cooldown = cooldown * 2;
                count += 1;
            }
            if cooldown > MODEL_SOFT_STOP_MAX_COOLDOWN {
                cooldown = MODEL_SOFT_STOP_MAX_COOLDOWN;
            }
            value.last_soft_stop_at = now;
            value.soft_stop_until = now + cooldown;
            st.model_outcomes.insert(key, value);
            (value.consecutive_soft_stop, value.soft_stop_until)
        };
        self.loader
            .save_model_state(ModelState {
                account_id,
                upstream_model,
                status: ModelStatus::SoftStop,
                reason: Some("soft_stop".into()),
                consecutive_failures: consecutive_soft_stop,
                last_attempt_at: Some(now),
                cooldown_until: Some(soft_stop_until),
                last_success_at: None,
                updated_at: now,
            })
            .await?;
        Ok(())
    }

    /// 模型成功记账（Go `MarkModelSuccess`）。
    pub async fn mark_model_success(
        &self,
        account_id: i64,
        upstream_model: &str,
    ) -> SelectorResult<()> {
        let upstream_model = upstream_model.trim().to_string();
        if account_id == 0 || upstream_model.is_empty() {
            return Ok(());
        }
        let now = Utc::now();
        self.state().model_outcomes.insert(
            (account_id, upstream_model.clone()),
            ModelOutcome {
                last_success_at: now,
                ..Default::default()
            },
        );
        self.loader
            .save_model_state(ModelState {
                account_id,
                upstream_model,
                status: ModelStatus::Available,
                reason: Some("image_generated".into()),
                consecutive_failures: 0,
                last_attempt_at: Some(now),
                cooldown_until: None,
                last_success_at: Some(now),
                updated_at: now,
            })
            .await?;
        Ok(())
    }

    /// 本地额度变化应用到候选快照（Go `ConsumeQuota`）。
    pub fn consume_quota(&self, provider: Provider, account_id: i64, mode: &str, amount: i64) {
        if account_id == 0 || mode.is_empty() || mode == "weekly" || amount <= 0 {
            return;
        }
        let mut st = self.state();
        for ((p, _, _), (values, _expires)) in st.candidates.iter_mut() {
            if *p != provider {
                continue;
            }
            for candidate in values.iter_mut() {
                if candidate.account.id != account_id {
                    continue;
                }
                if let Some(w) = &mut candidate.quota {
                    if w.mode == mode {
                        w.remaining = (w.remaining - amount).max(0);
                        w.updated_at = Utc::now();
                    }
                }
            }
        }
    }

    /// 票持有者 id 列表（Go `ticketHolderIDs`）。
    pub fn ticket_holder_ids(&self) -> Vec<i64> {
        let Some(source) = &self.chrome_tickets else {
            return Vec::new();
        };
        let mut ids: Vec<i64> = source
            .available_counts()
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// 持票候选优先，票池为空或无一持票时回退原集（Go `filterChromeTicketCandidates`）。
    fn filter_chrome_ticket_candidates(
        &self,
        candidates: Vec<RoutingCandidate>,
    ) -> Vec<RoutingCandidate> {
        let Some(source) = &self.chrome_tickets else {
            return candidates;
        };
        let counts = source.available_counts();
        if counts.is_empty() {
            return candidates;
        }
        let filtered: Vec<_> = candidates
            .iter()
            .filter(|c| counts.get(&c.account.id).copied().unwrap_or(0) > 0)
            .cloned()
            .collect();
        if filtered.is_empty() {
            candidates
        } else {
            filtered
        }
    }

    async fn claim_account_slot(
        &self,
        account: &Account,
        upstream_model: &str,
    ) -> SelectorResult<Option<SelectionLease>> {
        let limit = account_concurrency_limit(account, upstream_model);
        let key = format!("account:{}", account.id);
        let Some(release) = self.concurrency.acquire(&key, limit).await? else {
            return Ok(None);
        };
        let wake = {
            let mut st = self.state();
            st.last_selected_at.insert(account.id, Utc::now());
            st.lease_wake.clone()
        };
        Ok(Some(SelectionLease {
            account: account.clone(),
            quota_probe: false,
            quota_probe_kind: None,
            billing: None,
            quota_mode: None,
            release: Some(Box::new(move || {
                release();
                wake.notify_one();
            })),
        }))
    }

    async fn await_lease_retry(&self, deadline: DateTime<Utc>) -> bool {
        let remaining = deadline - Utc::now();
        if remaining <= Duration::zero() {
            return false;
        }
        let wake = self.state().lease_wake.clone();
        let timeout = remaining.min(Duration::milliseconds(100));
        tokio::select! {
            _ = wake.notified() => true,
            _ = tokio::time::sleep(timeout.to_std().unwrap_or(std::time::Duration::from_millis(100))) => {
                Utc::now() < deadline
            }
        }
    }

    fn resolve_tier_order(&self, provider: Provider, upstream_model: &str) -> Vec<WebTier> {
        self.tier_orders
            .as_ref()
            .map(|t| t.tier_order(provider, upstream_model))
            .unwrap_or_default()
    }

    fn random_unit(&self) -> f64 {
        use rand::Rng;
        rand::thread_rng().gen::<f64>()
    }

    /// 排序（对齐 Go `sortCandidates`）：收集快照 → stable 排序。
    pub async fn sort_candidates(
        &self,
        values: &mut [RoutingCandidate],
        now: DateTime<Utc>,
        tier_order: &[WebTier],
        upstream_model: &str,
    ) -> SelectorResult<()> {
        let mut ctx = SortContext::default();
        let upstream_model = upstream_model.trim().to_string();
        // 先持久化 ModelState（跨重启）排名，后内存 outcomes 覆盖——对齐 Go `sortCandidates`：
        // 最近一次成功/soft-stop 以内存为准（Go 先遍历 candidate.ModelState，再遍历 s.modelOutcomes）。
        for candidate in values.iter() {
            let Some(state) = &candidate.model_state else {
                continue;
            };
            if state.upstream_model != upstream_model {
                continue;
            }
            if state.status == ModelStatus::SoftStop
                && state.cooldown_until.is_some_and(|until| now < until)
            {
                ctx.model_ranks.insert(state.account_id, 2);
            } else if state.status == ModelStatus::Available
                && state
                    .last_success_at
                    .is_some_and(|at| now - at <= MODEL_OUTCOME_SUCCESS_TTL)
            {
                ctx.model_ranks.insert(state.account_id, 0);
            }
        }
        {
            let st = self.state();
            ctx.last_selected_at = st.last_selected_at.clone();
            for (id, outcome) in st.model_outcomes.iter() {
                let latest = outcome.last_success_at.max(outcome.last_soft_stop_at);
                if latest != DateTime::<Utc>::UNIX_EPOCH && now - latest > MODEL_OUTCOME_RETENTION {
                    continue; // 过期条目跳过（不删，避免锁内删除复杂度；保留期外不参与排序）
                }
                if id.1 != upstream_model {
                    continue;
                }
                if now < outcome.soft_stop_until {
                    ctx.model_ranks.insert(id.0, 2);
                } else if outcome.last_success_at != DateTime::<Utc>::UNIX_EPOCH
                    && now - outcome.last_success_at <= MODEL_OUTCOME_SUCCESS_TTL
                {
                    ctx.model_ranks.insert(id.0, 0);
                } else {
                    ctx.model_ranks.insert(id.0, 1);
                }
            }
        }
        if upstream_model.eq_ignore_ascii_case(WEB_LITE_IMAGE_UPSTREAM_MODEL) {
            if let Some(source) = &self.chrome_tickets {
                ctx.ticket_counts = source.available_counts();
            }
        }
        let ids: Vec<i64> = values.iter().map(|c| c.account.id).collect();
        ctx.in_flight = self.concurrency.current_many(&ids).await?;
        for candidate in values.iter() {
            let id = candidate.account.id;
            if let Some(billing) = &candidate.billing {
                ctx.billing_remaining.insert(id, billing.remaining());
                ctx.billing_fresh.insert(
                    id,
                    billing
                        .synced_at
                        .is_some_and(|at| now - at <= BILLING_FRESH_TTL),
                );
            }
            if let Some(w) = &candidate.quota {
                if w.mode == "imagine" {
                    ctx.imagine_remaining.insert(id, w.remaining);
                    ctx.imagine_fresh
                        .insert(id, candidate_imagine_quota_admissible(candidate, now));
                }
            }
        }
        values.sort_by(|l, r| compare_candidates(l, r, &ctx, tier_order, &upstream_model));
        Ok(())
    }
}
