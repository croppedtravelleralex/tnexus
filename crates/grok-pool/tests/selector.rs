//! G3-P4 Selector 集成测试（迁移 Go `gateway/selector_test.go`）。
//!
//! 覆盖：配额探针优先 / 到期前跳过 / paid 探针 / weekly-over-fast 闸门 / 额度模式隔离 /
//! 档位序 / Build 索引水合（不全表扫）/ normal probe 合并 / model block / imagine 0/0 /
//! lite image 串行 / 容量等待 / 并发存储失败 / 排序（model outcome 排名、批量并发快照）/
//! 探索洗牌 / 粘滞键 / ConsumeQuota。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use grok_domain::{
    Account, AuthStatus, ModelQuotaBlock, ModelState, ModelStatus, Provider, QuotaRecovery,
    QuotaRecoveryKind, QuotaRecoveryStatus, QuotaSource, QuotaWindow, RoutingCandidate, WebTier,
};
use grok_pool::selector::*;

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn build_account(id: i64, priority: i32, max_concurrent: i32) -> Account {
    Account {
        id,
        identity_key: format!("acc-{id}"),
        provider: Provider::GrokBuild,
        enabled: true,
        auth_status: AuthStatus::Active,
        priority,
        observed_model: Some("grok-4.5-build-free".into()),
        max_concurrent,
        ..Default::default()
    }
}

fn web_account(id: i64, tier: WebTier) -> Account {
    Account {
        id,
        identity_key: format!("web-{id}"),
        provider: Provider::GrokWeb,
        enabled: true,
        auth_status: AuthStatus::Active,
        web_tier: tier,
        max_concurrent: 4,
        ..Default::default()
    }
}

fn candidate(account: Account) -> RoutingCandidate {
    RoutingCandidate {
        account,
        ..Default::default()
    }
}

fn fresh_window(mode: &str, remaining: i64, total: i64) -> QuotaWindow {
    QuotaWindow {
        account_id: 0,
        mode: mode.into(),
        remaining,
        total,
        synced_at: Some(now()),
        source: QuotaSource::Upstream,
        updated_at: now(),
        ..Default::default()
    }
}

// ── fakes ─────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeLoader {
    /// provider → (upstream_model, quota_mode) → candidates
    by_provider: Mutex<HashMap<Provider, Vec<RoutingCandidate>>>,
    /// model 相关 block（Go `UpsertModelQuotaBlock` 语义由 loader 按 model 注入）
    by_model: Mutex<HashMap<String, Vec<RoutingCandidate>>>,
    recoveries: Mutex<HashMap<i64, QuotaRecovery>>,
    claimed_probes: Mutex<HashSet<i64>>,
    health: Mutex<HashMap<i64, HealthSet>>,
    model_states: Mutex<Vec<ModelState>>,
    list_calls: Mutex<usize>,
    list_by_ids_calls: Mutex<usize>,
}

/// (failure_count, cooldown_until)
type HealthSet = (i32, Option<DateTime<Utc>>);

impl FakeLoader {
    fn seed(&self, provider: Provider, values: Vec<RoutingCandidate>) {
        self.by_provider.lock().unwrap().insert(provider, values);
    }

    fn seed_model(&self, model: &str, values: Vec<RoutingCandidate>) {
        self.by_model
            .lock()
            .unwrap()
            .insert(model.to_string(), values);
    }

    fn seed_recovery(&self, recovery: QuotaRecovery) {
        self.recoveries
            .lock()
            .unwrap()
            .insert(recovery.account_id, recovery);
    }

    fn list_calls(&self) -> usize {
        *self.list_calls.lock().unwrap()
    }

    fn list_by_ids_calls(&self) -> usize {
        *self.list_by_ids_calls.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl CandidateLoader for FakeLoader {
    async fn list_routing_candidates(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
    ) -> SelectorResult<Vec<RoutingCandidate>> {
        *self.list_calls.lock().unwrap() += 1;
        let values = self
            .by_provider
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .unwrap_or_default();
        Ok(self.apply_model_filter(values, upstream_model, quota_mode))
    }

    async fn list_routing_candidates_by_ids(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        ids: &[i64],
    ) -> SelectorResult<Vec<RoutingCandidate>> {
        *self.list_by_ids_calls.lock().unwrap() += 1;
        let values = self
            .by_provider
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .unwrap_or_default();
        let wanted: HashSet<i64> = ids.iter().copied().collect();
        let picked: Vec<RoutingCandidate> = values
            .into_iter()
            .filter(|c| wanted.contains(&c.account.id))
            .collect();
        Ok(self.attach_recoveries(picked, upstream_model, quota_mode))
    }

    async fn claim_quota_probe(
        &self,
        account_id: i64,
        _now: DateTime<Utc>,
        _until: DateTime<Utc>,
    ) -> SelectorResult<bool> {
        Ok(self.claimed_probes.lock().unwrap().insert(account_id))
    }

    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        _reason: &str,
        _reset_last_success: bool,
    ) -> SelectorResult<()> {
        self.health
            .lock()
            .unwrap()
            .insert(id, (failure_count, cooldown_until));
        Ok(())
    }

    async fn clear_quota_recovery(&self, id: i64) -> SelectorResult<()> {
        self.recoveries.lock().unwrap().remove(&id);
        Ok(())
    }

    async fn save_quota_recovery(&self, recovery: QuotaRecovery) -> SelectorResult<()> {
        self.recoveries
            .lock()
            .unwrap()
            .insert(recovery.account_id, recovery);
        Ok(())
    }

    async fn save_model_state(&self, state: ModelState) -> SelectorResult<()> {
        self.model_states.lock().unwrap().push(state);
        Ok(())
    }

    async fn save_model_quota_block(&self, _block: ModelQuotaBlock) -> SelectorResult<()> {
        Ok(())
    }
}

impl FakeLoader {
    fn attach_recoveries(
        &self,
        mut values: Vec<RoutingCandidate>,
        _upstream_model: &str,
        _quota_mode: &str,
    ) -> Vec<RoutingCandidate> {
        let recoveries = self.recoveries.lock().unwrap().clone();
        for candidate in values.iter_mut() {
            if candidate.recovery.is_none() {
                candidate.recovery = recoveries.get(&candidate.account.id).cloned();
            }
        }
        values
    }

    fn apply_model_filter(
        &self,
        mut values: Vec<RoutingCandidate>,
        upstream_model: &str,
        quota_mode: &str,
    ) -> Vec<RoutingCandidate> {
        // model 专属候选（带 block/不支持标记）替换同名账号
        if let Some(model_values) = self.by_model.lock().unwrap().get(upstream_model).cloned() {
            let ids: HashSet<i64> = model_values.iter().map(|c| c.account.id).collect();
            values.retain(|c| !ids.contains(&c.account.id));
            values.extend(model_values);
        }
        self.attach_recoveries(values, upstream_model, quota_mode)
    }
}

#[derive(Default)]
struct InMemoryLimiter {
    counts: Arc<Mutex<HashMap<String, i32>>>,
}

#[async_trait::async_trait]
impl ConcurrencyLimiter for InMemoryLimiter {
    async fn acquire(
        &self,
        key: &str,
        limit: i32,
    ) -> SelectorResult<Option<Box<dyn FnOnce() + Send>>> {
        let mut counts = self.counts.lock().unwrap();
        let current = counts.entry(key.to_string()).or_insert(0);
        if *current >= limit {
            return Ok(None);
        }
        *current += 1;
        let key = key.to_string();
        let counts = Arc::clone(&self.counts);
        Ok(Some(Box::new(move || {
            if let Some(c) = counts.lock().unwrap().get_mut(&key) {
                *c -= 1;
            }
        })))
    }

    async fn current(&self, key: &str) -> SelectorResult<i32> {
        Ok(self.counts.lock().unwrap().get(key).copied().unwrap_or(0))
    }

    async fn current_many(&self, ids: &[i64]) -> SelectorResult<HashMap<i64, i32>> {
        let counts = self.counts.lock().unwrap();
        Ok(ids
            .iter()
            .map(|id| {
                (
                    *id,
                    counts.get(&format!("account:{id}")).copied().unwrap_or(0),
                )
            })
            .collect())
    }
}

struct FailingLimiter;

#[async_trait::async_trait]
impl ConcurrencyLimiter for FailingLimiter {
    async fn acquire(
        &self,
        _key: &str,
        _limit: i32,
    ) -> SelectorResult<Option<Box<dyn FnOnce() + Send>>> {
        Err(SelectorError::Concurrency(
            "runtime store unavailable".into(),
        ))
    }
    async fn current(&self, _key: &str) -> SelectorResult<i32> {
        Ok(0)
    }
    async fn current_many(&self, _ids: &[i64]) -> SelectorResult<HashMap<i64, i32>> {
        Ok(HashMap::new())
    }
}

#[derive(Default)]
struct InMemorySticky {
    entries: Mutex<HashMap<String, (i64, DateTime<Utc>)>>,
}

#[async_trait::async_trait]
impl StickyStore for InMemorySticky {
    async fn get(&self, key: &str, now: DateTime<Utc>) -> SelectorResult<Option<i64>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .get(key)
            .filter(|(_, expiry)| *expiry > now)
            .map(|(id, _)| *id))
    }
    async fn set(
        &self,
        key: &str,
        account_id: i64,
        expires_at: DateTime<Utc>,
    ) -> SelectorResult<()> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_string(), (account_id, expires_at));
        Ok(())
    }
    async fn delete_by_account(&self, account_id: i64) -> SelectorResult<()> {
        self.entries
            .lock()
            .unwrap()
            .retain(|_, (id, _)| *id != account_id);
        Ok(())
    }
}

struct StubBuildDispatch {
    dispatch_ids: Vec<i64>,
    normal_probe_ids: Vec<i64>,
    warm_calls: Mutex<usize>,
}

impl StubBuildDispatch {
    fn new(dispatch_ids: Vec<i64>) -> Self {
        Self {
            dispatch_ids,
            normal_probe_ids: Vec::new(),
            warm_calls: Mutex::new(0),
        }
    }
}

#[async_trait::async_trait]
impl BuildDispatchSource for StubBuildDispatch {
    fn ordered_dispatch_ids(&self, limit: usize) -> Vec<i64> {
        if limit == 0 || self.dispatch_ids.is_empty() {
            return Vec::new();
        }
        self.dispatch_ids.iter().take(limit).copied().collect()
    }
    fn due_normal_probe_ids(&self, _now: DateTime<Utc>, _limit: usize) -> Vec<i64> {
        self.normal_probe_ids.clone()
    }
    fn note_dispatch_selected(&self, _id: i64, _at: DateTime<Utc>) {}
    async fn ensure_warm(&self) -> SelectorResult<()> {
        *self.warm_calls.lock().unwrap() += 1;
        Ok(())
    }
}

struct StaticTierOrder(Vec<WebTier>);

impl TierOrderSource for StaticTierOrder {
    fn tier_order(&self, _provider: Provider, _upstream_model: &str) -> Vec<WebTier> {
        self.0.clone()
    }
}

fn new_selector(
    loader: Arc<FakeLoader>,
    limiter: Arc<dyn ConcurrencyLimiter>,
    capacity_wait: Duration,
) -> Selector {
    let mut selector = Selector::new(
        loader,
        limiter,
        Some(Arc::new(InMemorySticky::default())),
        None,
        Duration::hours(1),
        Duration::seconds(1),
        Duration::minutes(1),
        capacity_wait,
    );
    selector.set_exploration_epsilon(0.0);
    selector
}

async fn acquire_ok(
    selector: &Selector,
    provider: Provider,
    model: &str,
    quota_mode: &str,
) -> SelectionLease {
    selector
        .acquire(provider, model, quota_mode, "", &HashSet::new(), false)
        .await
        .expect("acquire ok")
}

// ── 纯函数 ────────────────────────────────────────────────────────

#[test]
fn prompt_cache_sticky_key_is_fixed_length_and_stable() {
    let first = prompt_cache_sticky_key("cache-key");
    assert_eq!(first.len(), 64);
    assert_eq!(first, prompt_cache_sticky_key("cache-key"));
    assert_ne!(first, prompt_cache_sticky_key("another-key"));
    assert_eq!(prompt_cache_sticky_key(""), "");
}

#[test]
fn account_concurrency_limit_only_serializes_web_lite_image() {
    let a = Account {
        max_concurrent: 4,
        ..Default::default()
    };
    assert_eq!(account_concurrency_limit(&a, "grok-imagine-image"), 1);
    assert_eq!(account_concurrency_limit(&a, "grok-chat-fast"), 4);
    assert_eq!(
        account_concurrency_limit(&Account::default(), "grok-chat-fast"),
        8
    );
}

#[test]
fn exploration_shuffle_behavior() {
    let mut values = vec![
        candidate(build_account(1, 10, 1)),
        candidate(build_account(2, 1, 1)),
    ];
    maybe_explore_shuffle(&mut values, 0.0, || 0.0);
    assert_eq!(values[0].account.id, 1, "epsilon=0 should preserve order");

    let mut values = vec![
        candidate(build_account(1, 100, 1)),
        candidate(build_account(2, 1, 1)),
        candidate(build_account(3, 1, 1)),
    ];
    // epsilon=1 恒触发；random 恒 0 → Fisher-Yates 每次取首（必乱序）
    maybe_explore_shuffle(&mut values, 1.0, || 0.0);
    assert_ne!(values[0].account.id, 1, "epsilon=1 should shuffle");

    let mut values = vec![
        candidate(build_account(10, 1, 1)),
        candidate(build_account(20, 1, 1)),
        candidate(build_account(30, 1, 1)),
    ];
    maybe_explore_shuffle(&mut values, 1.0, || 0.42);
    let seen: HashSet<i64> = values.iter().map(|c| c.account.id).collect();
    assert_eq!(seen.len(), 3);
}

// ── 选号 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn prioritizes_due_quota_probe_once() {
    let loader = Arc::new(FakeLoader::default());
    let probe = build_account(1, 10, 1);
    let active = build_account(2, 200, 1);
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(probe.clone()), candidate(active.clone())],
    );
    let t = now();
    loader.seed_recovery(QuotaRecovery {
        account_id: 1,
        kind: QuotaRecoveryKind::Free,
        status: QuotaRecoveryStatus::Exhausted,
        confirmed_used: 1_065_387,
        confirmed_limit: 1_000_000,
        exhausted_at: Some(t),
        next_probe_at: Some(t - Duration::minutes(1)),
        last_confirmed_at: Some(t),
        updated_at: t,
    });
    let selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );

    let lease = selector
        .acquire(
            Provider::GrokBuild,
            "grok-test",
            "",
            "",
            &HashSet::new(),
            true,
        )
        .await
        .expect("due probe leased");
    assert_eq!(lease.account.id, 1);
    assert!(lease.quota_probe);
    lease.release();

    // 探针账号被排除后 → 活跃账号；且第二次不允许 quota probe。
    let excluded: HashSet<i64> = HashSet::from([1]);
    let lease = selector
        .acquire(Provider::GrokBuild, "grok-test", "", "", &excluded, false)
        .await
        .expect("active leased");
    assert_eq!(lease.account.id, 2);
    assert!(!lease.quota_probe);
    lease.release();

    // MarkSuccess 清 recovery（Go：QuotaRecovery → ErrNotFound）
    selector.mark_success(&probe, true).await;
    assert!(
        !loader.recoveries.lock().unwrap().contains_key(&1),
        "recovery cleared"
    );
}

#[tokio::test]
async fn skips_quota_probe_before_due() {
    let loader = Arc::new(FakeLoader::default());
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(build_account(1, 100, 1))],
    );
    let t = now();
    loader.seed_recovery(QuotaRecovery {
        account_id: 1,
        kind: QuotaRecoveryKind::Free,
        status: QuotaRecoveryStatus::Exhausted,
        next_probe_at: Some(t + Duration::hours(1)),
        updated_at: t,
        ..Default::default()
    });
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    let err = selector
        .acquire(
            Provider::GrokBuild,
            "grok-test",
            "",
            "",
            &HashSet::new(),
            true,
        )
        .await
        .expect_err("no account before next probe");
    assert!(matches!(
        err,
        SelectorError::Unavailable(e)
            if matches!(
                e.reason,
                SelectionUnavailableReason::NoAccounts
                    | SelectionUnavailableReason::QuotaExhausted
            )
    ));
}

#[tokio::test]
async fn claims_paid_billing_probe_after_period_end() {
    let loader = Arc::new(FakeLoader::default());
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(build_account(1, 100, 1))],
    );
    let t = now();
    loader.seed_recovery(QuotaRecovery {
        account_id: 1,
        kind: QuotaRecoveryKind::Paid,
        status: QuotaRecoveryStatus::Exhausted,
        next_probe_at: Some(t - Duration::minutes(1)),
        updated_at: t,
        ..Default::default()
    });
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    let lease = selector
        .acquire(Provider::GrokBuild, "", "", "", &HashSet::new(), true)
        .await
        .expect("paid probe leased");
    assert!(lease.quota_probe);
    assert_eq!(lease.quota_probe_kind, Some(QuotaRecoveryKind::Paid));
}

#[tokio::test]
async fn uses_paid_weekly_pool_as_web_quota_gate() {
    let loader = Arc::new(FakeLoader::default());
    let mut a = web_account(1, WebTier::Super);
    a.max_concurrent = 1;
    loader.seed(Provider::GrokWeb, vec![candidate(a)]);
    let selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );

    // weekly 耗尽 → fast 请求被 weekly 闸门拦截（候选窗口即 weekly 门）
    let t = now();
    let reset = t + Duration::days(7);
    loader.seed_model(
        "grok-chat",
        vec![RoutingCandidate {
            account: web_account(1, WebTier::Super),
            quota: Some(QuotaWindow {
                account_id: 0,
                mode: "weekly".into(),
                remaining: 0,
                total: 10000,
                reset_at: Some(reset),
                synced_at: Some(now()),
                source: QuotaSource::Upstream,
                updated_at: now(),
            }),
            ..Default::default()
        }],
    );
    let err = selector
        .acquire(
            Provider::GrokWeb,
            "grok-chat",
            "fast",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect_err("exhausted weekly blocks stale fast");
    assert!(matches!(
        err,
        SelectorError::Unavailable(e) if e.reason == SelectionUnavailableReason::QuotaExhausted
    ));

    // weekly 恢复 → fast 0 也走 weekly 模式
    loader.seed_model(
        "grok-chat",
        vec![RoutingCandidate {
            account: web_account(1, WebTier::Super),
            quota: Some(QuotaWindow {
                account_id: 0,
                mode: "weekly".into(),
                remaining: 8900,
                total: 10000,
                reset_at: Some(now() + Duration::days(7)),
                synced_at: Some(now()),
                source: QuotaSource::Upstream,
                updated_at: now(),
            }),
            ..Default::default()
        }],
    );
    selector.invalidate_candidates(Provider::GrokWeb);
    let lease = acquire_ok(&selector, Provider::GrokWeb, "grok-chat", "fast").await;
    assert_eq!(lease.quota_mode.as_deref(), Some("weekly"));
}

#[tokio::test]
async fn keeps_web_quota_modes_isolated() {
    let loader = Arc::new(FakeLoader::default());
    let mut a = web_account(1, WebTier::Super);
    a.max_concurrent = 2;
    let mut fast = fresh_window("fast", 0, 20);
    fast.reset_at = Some(now() + Duration::hours(1));
    let mut auto = fresh_window("auto", 5, 10);
    auto.reset_at = Some(now() + Duration::hours(1));
    loader.seed(
        Provider::GrokWeb,
        vec![RoutingCandidate {
            account: a,
            quota: Some(fast),
            ..Default::default()
        }],
    );
    let selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    let err = selector
        .acquire(
            Provider::GrokWeb,
            "grok-chat",
            "fast",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect_err("exhausted fast blocked");
    assert!(matches!(
        err,
        SelectorError::Unavailable(e) if e.reason == SelectionUnavailableReason::QuotaExhausted
    ));
    // auto 模式单独有额度 → 放行
    loader.seed(
        Provider::GrokWeb,
        vec![RoutingCandidate {
            account: web_account(1, WebTier::Super),
            quota: Some(auto),
            ..Default::default()
        }],
    );
    selector.invalidate_candidates(Provider::GrokWeb);
    let lease = acquire_ok(&selector, Provider::GrokWeb, "grok-chat-auto", "auto").await;
    assert_eq!(lease.account.id, 1);
    assert_eq!(lease.quota_mode.as_deref(), Some("auto"));
}

#[tokio::test]
async fn honors_web_tier_pool_order_before_account_priority() {
    let loader = Arc::new(FakeLoader::default());
    let mut accounts = Vec::new();
    for (idx, tier) in [WebTier::Basic, WebTier::Super, WebTier::Heavy]
        .iter()
        .enumerate()
    {
        let mut a = web_account(idx as i64 + 1, *tier);
        a.priority = 300 - idx as i32 * 100;
        a.max_concurrent = 1;
        accounts.push(candidate(a));
    }
    loader.seed(Provider::GrokWeb, accounts);
    let mut selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector.set_tier_orders(Arc::new(StaticTierOrder(vec![
        WebTier::Heavy,
        WebTier::Super,
        WebTier::Basic,
    ])));
    // 需要给 Selector 注入 tierOrders —— 上面用 new_selector(None)；改用 builder
    let lease = selector
        .acquire(
            Provider::GrokWeb,
            "fast-prefer-best",
            "fast",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect("tier-ordered lease");
    assert_eq!(lease.account.web_tier, WebTier::Heavy);
}

#[tokio::test]
async fn build_acquire_avoids_full_table_list() {
    let loader = Arc::new(FakeLoader::default());
    loader.seed(
        Provider::GrokBuild,
        vec![
            candidate(build_account(1, 100, 1)),
            candidate(build_account(2, 1, 1)),
        ],
    );
    let dispatch = Arc::new(StubBuildDispatch::new(vec![1]));
    let mut selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector.set_build_dispatch_source(dispatch);

    let lease = acquire_ok(&selector, Provider::GrokBuild, "grok-test", "").await;
    assert_eq!(lease.account.id, 1);
    assert_eq!(loader.list_calls(), 0, "no full table list");
    assert_eq!(loader.list_by_ids_calls(), 1, "hydrate by ids");
}

#[tokio::test]
async fn build_acquire_merges_due_normal_probe_ids() {
    let loader = Arc::new(FakeLoader::default());
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(build_account(1, 10, 1))],
    );
    let t = now();
    loader.seed_recovery(QuotaRecovery {
        account_id: 1,
        kind: QuotaRecoveryKind::Free,
        status: QuotaRecoveryStatus::Exhausted,
        next_probe_at: Some(t - Duration::minutes(1)),
        updated_at: t,
        ..Default::default()
    });
    let dispatch = Arc::new(StubBuildDispatch {
        dispatch_ids: vec![],
        normal_probe_ids: vec![1],
        warm_calls: Mutex::new(0),
    });
    let mut selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector.set_build_dispatch_source(dispatch);

    let lease = selector
        .acquire(
            Provider::GrokBuild,
            "grok-test",
            "",
            "",
            &HashSet::new(),
            true,
        )
        .await
        .expect("due normal probe leased");
    assert_eq!(lease.account.id, 1);
    assert!(lease.quota_probe);
    assert_eq!(loader.list_calls(), 0);
    assert_eq!(loader.list_by_ids_calls(), 1);
}

#[tokio::test]
async fn applies_persisted_cooldown_only_to_matching_model() {
    let loader = Arc::new(FakeLoader::default());
    let blocked_until = now() + Duration::hours(1);
    loader.seed_model(
        "limited-model",
        vec![RoutingCandidate {
            account: build_account(1, 100, 1),
            model_quota_block: Some(ModelQuotaBlock {
                account_id: 1,
                upstream_model: "limited-model".into(),
                reason: "test".into(),
                cooldown_until: blocked_until,
                updated_at: now(),
            }),
            ..Default::default()
        }],
    );
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(build_account(1, 100, 1))],
    );
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );

    let err = selector
        .acquire(
            Provider::GrokBuild,
            "limited-model",
            "",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect_err("matching model cooldown ignored");
    match err {
        SelectorError::Unavailable(e) => {
            assert_eq!(e.reason, SelectionUnavailableReason::ModelCooling);
            assert!(
                e.retry_after >= Duration::minutes(30),
                "retry_after = {}",
                e.retry_after
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    let lease = acquire_ok(&selector, Provider::GrokBuild, "other-model", "").await;
    assert_eq!(lease.account.id, 1);
}

#[tokio::test]
async fn treats_zero_total_model_quota_as_unknown() {
    let loader = Arc::new(FakeLoader::default());
    let mut a = web_account(1, WebTier::Basic);
    a.max_concurrent = 1;
    loader.seed(
        Provider::GrokWeb,
        vec![RoutingCandidate {
            account: a,
            quota: Some(fresh_window("imagine", 0, 0)),
            ..Default::default()
        }],
    );
    let selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );

    // 0/0：闸门不可靠，无正向证据 → blocked
    let err = selector
        .acquire(
            Provider::GrokWeb,
            "grok-imagine-image",
            "imagine",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect_err("0/0 imagine blocked");
    assert!(matches!(
        err,
        SelectorError::Unavailable(e) if e.reason == SelectionUnavailableReason::QuotaExhausted
    ));

    // 0/10：明确的耗尽 → blocked
    loader.seed(
        Provider::GrokWeb,
        vec![RoutingCandidate {
            account: web_account(1, WebTier::Basic),
            quota: Some(fresh_window("imagine", 0, 10)),
            ..Default::default()
        }],
    );
    selector.invalidate_candidates(Provider::GrokWeb);
    let err = selector
        .acquire(
            Provider::GrokWeb,
            "grok-imagine-image",
            "imagine",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect_err("0/10 imagine blocked");
    assert!(matches!(
        err,
        SelectorError::Unavailable(e) if e.reason == SelectionUnavailableReason::QuotaExhausted
    ));

    // 5/10 + model block：正额度优先于临时 block → 放行
    loader.seed(
        Provider::GrokWeb,
        vec![RoutingCandidate {
            account: web_account(1, WebTier::Super),
            quota: Some(fresh_window("imagine", 5, 10)),
            model_quota_block: Some(ModelQuotaBlock {
                account_id: 1,
                upstream_model: "grok-imagine-image".into(),
                reason: "old_usage_limit".into(),
                cooldown_until: now() + Duration::hours(1),
                updated_at: now(),
            }),
            ..Default::default()
        }],
    );
    selector.invalidate_candidates(Provider::GrokWeb);
    let lease = acquire_ok(
        &selector,
        Provider::GrokWeb,
        "grok-imagine-image",
        "imagine",
    )
    .await;
    assert_eq!(lease.account.id, 1);
}

#[tokio::test]
async fn serializes_web_lite_image_per_account() {
    let loader = Arc::new(FakeLoader::default());
    let preferred = web_account(1, WebTier::Basic);
    let alternate = web_account(2, WebTier::Basic);
    loader.seed(
        Provider::GrokWeb,
        vec![
            RoutingCandidate {
                account: preferred.clone(),
                quota: Some(fresh_window("imagine", 8, 10)),
                ..Default::default()
            },
            RoutingCandidate {
                account: alternate.clone(),
                quota: Some(fresh_window("imagine", 6, 10)),
                ..Default::default()
            },
        ],
    );
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector
        .mark_model_success(1, "grok-imagine-image")
        .await
        .expect("mark success");

    let first = acquire_ok(
        &selector,
        Provider::GrokWeb,
        "grok-imagine-image",
        "imagine",
    )
    .await;
    assert_eq!(first.account.id, 1, "recent success preferred");
    let second = acquire_ok(
        &selector,
        Provider::GrokWeb,
        "grok-imagine-image",
        "imagine",
    )
    .await;
    assert_eq!(
        second.account.id, 2,
        "alternate on second (limit 1 per account)"
    );
    first.release();
    second.release();
}

#[tokio::test]
async fn waits_briefly_for_account_capacity() {
    let loader = Arc::new(FakeLoader::default());
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(build_account(1, 100, 1))],
    );
    let selector = Arc::new(new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::milliseconds(300),
    ));
    let first = selector
        .acquire(Provider::GrokBuild, "model", "", "", &HashSet::new(), false)
        .await
        .expect("first lease");

    let selector2 = selector.clone();
    let handle = tokio::spawn(async move {
        selector2
            .acquire(Provider::GrokBuild, "model", "", "", &HashSet::new(), false)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        !handle.is_finished(),
        "second acquire must wait for capacity"
    );
    first.release();
    // timeout(Elapsed) → JoinHandle(JoinError) → acquire(SelectorError) 三层
    let second: SelectionLease =
        match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
            Ok(Ok(Ok(lease))) => lease,
            Ok(Ok(Err(e))) => panic!("second acquire failed: {e}"),
            Ok(Err(e)) => panic!("join failed: {e}"),
            Err(_) => panic!("second acquire timed out"),
        };
    second.release();
}

#[tokio::test]
async fn propagates_concurrency_store_failure() {
    let loader = Arc::new(FakeLoader::default());
    loader.seed(
        Provider::GrokBuild,
        vec![candidate(build_account(1, 100, 1))],
    );
    let selector = new_selector(loader, Arc::new(FailingLimiter), Duration::zero());
    let err = selector
        .acquire(Provider::GrokBuild, "", "", "", &HashSet::new(), true)
        .await
        .expect_err("runtime error propagated");
    assert!(matches!(err, SelectorError::Concurrency(_)));
}

// ── 排序 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn ranks_recent_model_success_before_unknown_and_soft_stop() {
    let loader = Arc::new(FakeLoader::default());
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector
        .mark_model_soft_stop(1, "grok-imagine-image")
        .await
        .expect("soft stop");
    selector
        .mark_model_success(2, "grok-imagine-image")
        .await
        .expect("success");

    let mut values = vec![
        candidate(build_account(1, 1, 1)),
        candidate(build_account(2, 1, 1)),
        candidate(build_account(3, 1, 1)),
    ];
    selector
        .sort_candidates(&mut values, now(), &[], "grok-imagine-image")
        .await
        .expect("sort");
    let got: Vec<i64> = values.iter().map(|c| c.account.id).collect();
    assert_eq!(got, vec![2, 3, 1], "success(0) < unknown(1) < soft_stop(2)");
}

#[tokio::test]
async fn model_outcome_does_not_affect_other_models() {
    let loader = Arc::new(FakeLoader::default());
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector
        .mark_model_soft_stop(1, "grok-imagine-image")
        .await
        .expect("soft stop");
    let mut values = vec![
        candidate(build_account(1, 1, 1)),
        candidate(build_account(2, 1, 1)),
    ];
    selector
        .sort_candidates(&mut values, now(), &[], "grok-fast")
        .await
        .expect("sort");
    assert_eq!(values[0].account.id, 1, "other model order unchanged");
}

#[tokio::test]
async fn uses_batch_concurrency_snapshot() {
    let loader = Arc::new(FakeLoader::default());
    let limiter = InMemoryLimiter::default();
    {
        let mut counts = limiter.counts.lock().unwrap();
        counts.insert("account:1".into(), 2);
        counts.insert("account:2".into(), 1);
    }
    let selector = new_selector(loader, Arc::new(limiter), Duration::zero());
    let mut values = vec![
        candidate(build_account(1, 1, 1)),
        candidate(build_account(2, 1, 1)),
    ];
    selector
        .sort_candidates(&mut values, now(), &[], "model")
        .await
        .expect("sort");
    assert_eq!(values[0].account.id, 2, "lower in-flight first");
}

#[tokio::test]
async fn consumes_only_matching_quota_snapshot() {
    let loader = Arc::new(FakeLoader::default());
    let mut a = web_account(7, WebTier::Basic);
    a.max_concurrent = 1;
    loader.seed(
        Provider::GrokWeb,
        vec![RoutingCandidate {
            account: a,
            quota: Some(QuotaWindow {
                account_id: 7,
                mode: "fast".into(),
                remaining: 3,
                total: 10,
                ..Default::default()
            }),
            ..Default::default()
        }],
    );
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    let first = acquire_ok(&selector, Provider::GrokWeb, "grok-chat", "fast").await;
    assert_eq!(first.account.id, 7);
    first.release();

    // 本地扣减应用到候选快照：3-3=0 → 下次 acquire 命中耗尽闸门
    selector.consume_quota(Provider::GrokWeb, 7, "fast", 3);
    let err = selector
        .acquire(
            Provider::GrokWeb,
            "grok-chat",
            "fast",
            "",
            &HashSet::new(),
            false,
        )
        .await
        .expect_err("consumed quota blocks next acquire");
    assert!(matches!(
        err,
        SelectorError::Unavailable(e) if e.reason == SelectionUnavailableReason::QuotaExhausted
    ));
    // 不匹配的 mode 不扣减
    selector.consume_quota(Provider::GrokWeb, 7, "auto", 3);
    selector.invalidate_candidates(Provider::GrokWeb);
    let lease = acquire_ok(&selector, Provider::GrokWeb, "grok-chat", "fast").await;
    lease.release();
}

#[tokio::test]
async fn in_memory_outcome_overrides_persisted_state() {
    let loader = Arc::new(FakeLoader::default());
    let selector = new_selector(
        loader,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    // 持久化 ModelState：soft-stop 未过期（rank 2）
    let persisted_soft_stop = RoutingCandidate {
        account: build_account(1, 1, 1),
        model_state: Some(ModelState {
            account_id: 1,
            upstream_model: "grok-imagine-image".into(),
            status: ModelStatus::SoftStop,
            reason: Some("soft_stop".into()),
            consecutive_failures: 1,
            last_attempt_at: Some(now()),
            cooldown_until: Some(now() + Duration::hours(1)),
            last_success_at: None,
            updated_at: now(),
        }),
        ..Default::default()
    };
    // 内存 outcome：成功（rank 0）→ 覆盖持久化 soft-stop（对齐 Go：modelOutcomes 后写）
    selector
        .mark_model_success(1, "grok-imagine-image")
        .await
        .expect("mark success");
    let mut values = vec![
        persisted_soft_stop,
        candidate(build_account(2, 1, 1)),
        candidate(build_account(3, 1, 1)),
    ];
    selector
        .sort_candidates(&mut values, now(), &[], "grok-imagine-image")
        .await
        .expect("sort");
    assert_eq!(
        values[0].account.id, 1,
        "in-memory success overrides persisted soft-stop"
    );

    // 反向：持久化 success + 内存 soft-stop → 内存覆盖 → rank 2 排最后
    let loader2 = Arc::new(FakeLoader::default());
    let selector2 = new_selector(
        loader2,
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    selector2
        .mark_model_soft_stop(1, "grok-imagine-image")
        .await
        .expect("soft stop");
    let persisted_success = RoutingCandidate {
        account: build_account(1, 1, 1),
        model_state: Some(ModelState {
            account_id: 1,
            upstream_model: "grok-imagine-image".into(),
            status: ModelStatus::Available,
            reason: Some("image_generated".into()),
            consecutive_failures: 0,
            last_attempt_at: Some(now()),
            cooldown_until: None,
            last_success_at: Some(now()),
            updated_at: now(),
        }),
        ..Default::default()
    };
    let mut values = vec![
        persisted_success,
        candidate(build_account(2, 1, 1)),
        candidate(build_account(3, 1, 1)),
    ];
    selector2
        .sort_candidates(&mut values, now(), &[], "grok-imagine-image")
        .await
        .expect("sort");
    assert_eq!(
        values[2].account.id, 1,
        "in-memory soft-stop overrides persisted success"
    );
}

#[tokio::test]
async fn mark_success_persists_on_first_success() {
    let loader = Arc::new(FakeLoader::default());
    let selector = new_selector(
        loader.clone(),
        Arc::new(InMemoryLimiter::default()),
        Duration::zero(),
    );
    let a = build_account(1, 1, 1); // 无 failure/cooldown/last_error
    selector.mark_success(&a, false).await;
    assert!(
        loader.health.lock().unwrap().contains_key(&1),
        "first success must persist health (Go last.IsZero() → persist)"
    );
}
