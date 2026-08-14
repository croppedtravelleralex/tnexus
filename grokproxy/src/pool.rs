//! Scheduling: hand out a ready account, report what happened to it.
//!
//! Requests are served from an in-memory ready pool rather than a query per
//! request. Three things fall out of that which the per-request query could not
//! give us: a claimed account is invisible to other requests for the duration
//! (so two callers never share one account's rate limit), token refresh happens
//! off the request path, and ranking can weigh remaining budget instead of only
//! last-used order.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use tracing::{debug, warn};

use crate::model::{Account, Health, Provider};
use crate::probe::{Probe, ProbeReport, Prober, REFRESH_SKEW_SECS};
use crate::store::Store;
use crate::upstream::{Failure, Upstream, UpstreamError};

/// How long to rest an account whose reported quota hit zero.
const QUOTA_COOLDOWN_SECS: i64 = 3_600;
/// How many accounts to pull from the database per refill.
const REFILL_BATCH: usize = 64;
/// Unmeasured accounts ride at the back of each refill so live traffic can
/// still discover entitlements, without a burst of chat-denied unknowns
/// failing a request before a proven account is tried.
const EXPLORE_PER_REFILL: usize = 8;
/// Skip a token-only keepalive if the account served traffic this recently:
/// a live request already proved the credential.
const KEEPALIVE_FRESH_SECS: i64 = 1_800;
/// Adaptive sweeps chat-probe at most this many unmeasured accounts. The rest
/// of the batch is a free token check, so upkeep cannot drain the pool.
const MAX_CHAT_PROBES_PER_SWEEP: usize = 80;

/// Accounts ready to serve, plus the ones currently serving.
#[derive(Default)]
struct Ready {
    queue: VecDeque<Account>,
    /// Ids handed out and not yet returned. Kept separate from the queue so a
    /// refill cannot re-admit an account that is already in flight.
    leased: HashSet<i64>,
}

pub struct Pool {
    store: Store,
    upstream: Upstream,
    max_attempts: usize,
    ready: Mutex<Ready>,
}

/// An account claimed for one request, with a usable access token.
///
/// Dropping the lease returns the account to the pool. That happens on every
/// path — success, upstream error, panic — so a request that dies mid-flight
/// cannot strand an account outside the rotation.
pub struct Lease<'a> {
    pub account: Account,
    pool: &'a Pool,
}

impl std::fmt::Debug for Lease<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("account", &self.account.email)
            .finish()
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.pool.release(self.account.id);
    }
}

impl Pool {
    pub fn new(store: Store, upstream: Upstream, max_attempts: usize) -> Self {
        Pool {
            store,
            upstream,
            max_attempts: max_attempts.max(1),
            ready: Mutex::new(Ready::default()),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn upstream(&self) -> &Upstream {
        &self.upstream
    }

    pub fn prober(&self) -> Prober<'_> {
        Prober {
            upstream: &self.upstream,
            store: &self.store,
        }
    }

    /// How many accounts are queued and how many are in flight.
    pub fn ready_depth(&self) -> (usize, usize) {
        let ready = self.ready.lock().unwrap();
        (ready.queue.len(), ready.leased.len())
    }

    /// Claim a Build account and guarantee a non-expired access token.
    ///
    /// A refresh failure is charged to that account and the next one is tried,
    /// so one revoked credential cannot fail the whole request.
    pub async fn acquire_build(&self) -> Result<Lease<'_>> {
        let mut last_error = String::from("no schedulable build account");
        for _ in 0..self.max_attempts {
            let Some(mut account) = self.take_ready()? else {
                break;
            };

            if !account.needs_refresh(crate::now(), REFRESH_SKEW_SECS) {
                return Ok(self.lease(account));
            }

            match self
                .upstream
                .refresh_token(&account.refresh_token, &account.proxy_url)
                .await
            {
                Ok(pair) => {
                    // Persist before use: xAI already revoked the old token, so
                    // losing this write would kill the account permanently.
                    self.store.save_refreshed(
                        account.id,
                        &pair.access_token,
                        &pair.refresh_token,
                        pair.expires_at,
                        crate::now(),
                    )?;
                    account.access_token = pair.access_token;
                    if !pair.refresh_token.is_empty() {
                        account.refresh_token = pair.refresh_token;
                    }
                    account.expires_at = pair.expires_at;
                    return Ok(self.lease(account));
                }
                Err(err) => {
                    let failure = downcast_failure(&err);
                    last_error = format!("{}: {err}", account.email);
                    warn!(account = %account.email, error = %err, "refresh failed");
                    self.report_failure(&account, &failure, &err.to_string())?;
                    self.release(account.id);
                }
            }
        }
        Err(anyhow!(last_error))
    }

    fn lease(&self, account: Account) -> Lease<'_> {
        Lease {
            account,
            pool: self,
        }
    }

    /// Next account off the ready queue, refilling from the database when it
    /// runs dry. Marks the account leased before returning it.
    fn take_ready(&self) -> Result<Option<Account>> {
        {
            let mut ready = self.ready.lock().unwrap();
            if let Some(account) = ready.queue.pop_front() {
                ready.leased.insert(account.id);
                return Ok(Some(account));
            }
        }
        self.refill()?;
        let mut ready = self.ready.lock().unwrap();
        Ok(ready.queue.pop_front().inspect(|account| {
            ready.leased.insert(account.id);
        }))
    }

    /// Pull a batch of schedulable accounts and rank them.
    ///
    /// Measured accounts with budget left carry the load. A small slice of
    /// unmeasured accounts rides at the back so live traffic still fills in
    /// entitlements, without a NewAPI request burning three unknown 403s
    /// before it ever reaches a proven account.
    fn refill(&self) -> Result<()> {
        let now = crate::now();
        let mut ready = self.ready.lock().unwrap();
        if !ready.queue.is_empty() {
            return Ok(());
        }
        let min_budget = Probe::Chat.budget_cost();
        let mut unknown = Vec::new();
        let mut known = Vec::new();
        for account in self.store.list(Some(Provider::Build))? {
            if !account.is_available(now) || ready.leased.contains(&account.id) {
                continue;
            }
            if account.limit_tokens <= 0 {
                unknown.push(account);
            } else if remaining_budget(&account) > min_budget {
                known.push(account);
            }
        }
        unknown.sort_by_key(|account| account.last_used_at);
        known.sort_by(|a, b| {
            remaining_budget(b)
                .cmp(&remaining_budget(a))
                .then(a.last_used_at.cmp(&b.last_used_at))
        });
        let explore_n = unknown.len().min(EXPLORE_PER_REFILL).min(REFILL_BATCH / 4);
        let mut queue: Vec<Account> = known
            .into_iter()
            .take(REFILL_BATCH.saturating_sub(explore_n))
            .collect();
        queue.extend(unknown.into_iter().take(explore_n));
        debug!(
            queued = queue.len(),
            exploring = explore_n,
            "ready pool refilled"
        );
        ready.queue = queue.into();
        Ok(())
    }

    fn release(&self, id: i64) {
        self.ready.lock().unwrap().leased.remove(&id);
    }

    /// Drop an account from the ready queue after it turned out to be unusable,
    /// so the next request does not pick it straight back up.
    fn evict(&self, id: i64) {
        let mut ready = self.ready.lock().unwrap();
        ready.queue.retain(|account| account.id != id);
    }

    /// Success plus the token/cost accounting and quota the upstream returned.
    ///
    /// When the headers say nothing is left, the account is cooled immediately
    /// instead of waiting for the next request to discover it the hard way.
    pub fn report_success_with_usage(
        &self,
        account: &Account,
        model: &str,
        outcome: &crate::upstream::ChatOutcome,
    ) -> Result<()> {
        let usage = crate::store::Usage::from_response(&outcome.body);
        let now = crate::now();
        let budget = self.store.record_success_with_usage(
            account.id,
            model,
            now,
            &usage,
            Some(&outcome.rate_limit),
        )?;
        // The upstream advertises the entitlement but never counts it down, so
        // exhaustion is decided against our own running total.
        if budget.spent() {
            debug!(
                account = %account.email,
                spent = budget.spent_tokens,
                limit = budget.limit_tokens,
                "token budget spent, retiring account"
            );
            self.store.mark_health(
                account.id,
                Health::Cooling,
                now + QUOTA_COOLDOWN_SECS,
                &format!(
                    "token budget spent ({}/{})",
                    budget.spent_tokens, budget.limit_tokens
                ),
                now,
            )?;
        }
        Ok(())
    }

    pub fn report_failure(&self, account: &Account, failure: &Failure, error: &str) -> Result<()> {
        let now = crate::now();
        let cooling_until = match failure.cooling_secs() {
            0 => 0,
            secs => now + secs,
        };
        debug!(
            account = %account.email,
            health = failure.health().as_str(),
            "marking failure"
        );
        self.store.record_failure(
            account.id,
            failure.health(),
            cooling_until,
            &crate::upstream::truncate(error, 300),
            now,
        )?;
        // Whatever went wrong, this account is not fit to serve the next
        // request off the queue.
        self.evict(account.id);
        Ok(())
    }

    /// Run a probe across a slice of the pool.
    ///
    /// `probe` of `None` picks the cheapest probe that still teaches us
    /// something about each account, which keeps a routine sweep from spending
    /// the budget of accounts we have already measured.
    pub async fn probe_pool(
        &self,
        probe: Option<Probe>,
        limit: usize,
        concurrency: usize,
    ) -> Result<ProbeReport> {
        let now = crate::now();
        let mut candidates: Vec<Account> = self
            .store
            .list(Some(Provider::Build))?
            .into_iter()
            .filter(|account| probe_eligible(account, probe, now))
            .collect();
        // Unmeasured first, then tokens about to expire, then least recently
        // used. Re-probing a measured live account teaches nothing.
        candidates.sort_by(|a, b| {
            let rank = |account: &Account| {
                let unmeasured = account.limit_tokens <= 0 || account.verified_at == 0;
                let stale = account.needs_refresh(now, REFRESH_SKEW_SECS);
                (!unmeasured, !stale, account.last_used_at)
            };
            rank(a).cmp(&rank(b))
        });

        let forced_chat = matches!(probe, Some(Probe::Chat));
        let mut chat_left = if forced_chat {
            usize::MAX
        } else {
            MAX_CHAT_PROBES_PER_SWEEP
        };
        let want = if limit == 0 {
            candidates.len()
        } else {
            limit.min(candidates.len())
        };
        let mut selected: Vec<(Account, Probe)> = Vec::with_capacity(want);
        for account in candidates {
            let mut chosen = probe.unwrap_or_else(|| Probe::cheapest_useful_for(&account));
            if chosen == Probe::Chat && chat_left == 0 {
                chosen = Probe::Token;
            }
            if chosen == Probe::Chat {
                chat_left = chat_left.saturating_sub(1);
            } else if probe.is_none() && !worth_keepalive(&account, now) {
                continue;
            }
            selected.push((account, chosen));
            if selected.len() >= want {
                break;
            }
        }

        let prober = self.prober();
        let mut report = ProbeReport::default();
        let width = concurrency.clamp(1, 16);
        for chunk in selected.chunks(width) {
            let outcomes = futures::future::join_all(chunk.iter().map(|(account, chosen)| {
                let prober = &prober;
                async move { (account, prober.run(account, *chosen).await) }
            }))
            .await;
            for (account, outcome) in outcomes {
                let outcome = outcome?;
                if !outcome.alive() {
                    self.evict(account.id);
                }
                report.absorb(&outcome);
            }
        }
        debug!(?report, "probe run complete");
        Ok(report)
    }

    /// Remove accounts that cannot serve. The in-memory ready queue is dropped
    /// so the next request refills from whatever is still in the store.
    pub fn purge_unusable(&self) -> Result<usize> {
        let deleted = self.store.purge_unusable()?;
        if deleted > 0 {
            self.ready.lock().unwrap().queue.clear();
        }
        Ok(deleted)
    }

    /// Models advertised across the pool, newest first.
    ///
    /// Served from the last observed value per account so `/v1/models` stays
    /// cheap; a fresh pool falls back to the upstream default.
    pub fn advertised_models(&self) -> Result<Vec<String>> {
        let mut seen: Vec<String> = Vec::new();
        for account in self.store.list(Some(Provider::Build))? {
            if !account.last_model.is_empty() && !seen.contains(&account.last_model) {
                seen.push(account.last_model);
            }
        }
        if seen.is_empty() {
            seen.push(crate::upstream::FALLBACK_MODEL.to_string());
        }
        seen.sort_by(|a, b| b.cmp(a));
        Ok(seen)
    }

    pub fn healthy_count(&self) -> Result<usize> {
        let now = crate::now();
        Ok(self
            .store
            .list(Some(Provider::Build))?
            .iter()
            .filter(|account| account.is_available(now))
            .count())
    }
}

fn probe_eligible(account: &Account, probe: Option<Probe>, now: i64) -> bool {
    if !account.is_available(now) {
        return false;
    }
    let unmeasured = account.limit_tokens <= 0 || account.verified_at == 0;
    match probe {
        // Quota discovery: never spend a measured account's budget proving
        // what we already know.
        Some(Probe::Chat) => unmeasured,
        // Keepalive: skip unmeasured accounts that still have a live access
        // token; the measure loop chats those instead.
        Some(Probe::Token) => !unmeasured || account.needs_refresh(now, REFRESH_SKEW_SECS),
        Some(Probe::Models) | None => true,
    }
}

/// Tokens an account still has to spend.
///
/// Unmeasured accounts are counted at the typical Build entitlement: unknown
/// is not zero, and serving one is how the number becomes known. They therefore
/// rank alongside a full measured account rather than jumping the queue or
/// sitting at the back forever.
fn remaining_budget(account: &Account) -> i64 {
    if account.limit_tokens <= 0 {
        return crate::model::TYPICAL_TOKEN_ENTITLEMENT;
    }
    (account.limit_tokens - account.total_tokens).max(0)
}

fn worth_keepalive(account: &Account, now: i64) -> bool {
    if account.needs_refresh(now, REFRESH_SKEW_SECS) {
        return true;
    }
    if account.last_used_at == 0 {
        return true;
    }
    now.saturating_sub(account.last_used_at) >= KEEPALIVE_FRESH_SECS
}

pub fn downcast_failure(err: &anyhow::Error) -> Failure {
    err.downcast_ref::<UpstreamError>()
        .map(UpstreamError::failure)
        .unwrap_or(Failure::Transient)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AccountImport;
    use crate::store::Usage;

    fn store_with(accounts: &[(&str, &str, i64)]) -> Store {
        let store = Store::open_in_memory().unwrap();
        let items: Vec<AccountImport> = accounts
            .iter()
            .map(|(email, refresh, expires)| {
                serde_json::from_value(serde_json::json!({
                    "email": email,
                    "refresh_token": refresh,
                    "access_token": "access",
                    "expires_at": expires,
                }))
                .unwrap()
            })
            .collect();
        store.import(Some(Provider::Build), &items, 1).unwrap();
        store
    }

    fn pool(store: Store) -> Pool {
        Pool::new(
            store,
            Upstream::new(crate::upstream::DEFAULT_BASE_URL, 5),
            3,
        )
    }

    #[tokio::test]
    async fn empty_pool_reports_no_account_rather_than_hanging() {
        let pool = pool(Store::open_in_memory().unwrap());
        let err = pool.acquire_build().await.unwrap_err();
        assert!(err.to_string().contains("no schedulable"));
    }

    #[tokio::test]
    async fn a_still_valid_token_is_used_without_touching_the_network() {
        let far_future = crate::now() + 100_000;
        let pool = pool(store_with(&[("a@b.c", "r1", far_future)]));
        let lease = pool.acquire_build().await.unwrap();
        assert_eq!(lease.account.email, "a@b.c");
        assert_eq!(lease.account.access_token, "access");
    }

    #[tokio::test]
    async fn two_concurrent_requests_never_share_one_account() {
        // Sharing would have them race the same account's rate limit while the
        // rest of the pool sits idle.
        let far = crate::now() + 100_000;
        let pool = pool(store_with(&[("a@b.c", "r1", far), ("b@b.c", "r2", far)]));
        let first = pool.acquire_build().await.unwrap();
        let second = pool.acquire_build().await.unwrap();
        assert_ne!(first.account.id, second.account.id);
        assert_eq!(pool.ready_depth().1, 2, "both are in flight");
    }

    #[tokio::test]
    async fn a_finished_request_returns_its_account_to_the_pool() {
        let far = crate::now() + 100_000;
        let pool = pool(store_with(&[("only@b.c", "r1", far)]));
        let id = {
            let lease = pool.acquire_build().await.unwrap();
            assert_eq!(pool.ready_depth().1, 1);
            lease.account.id
        };
        assert_eq!(pool.ready_depth().1, 0, "lease released on drop");

        // And it is schedulable again, rather than stranded outside rotation.
        let again = pool.acquire_build().await.unwrap();
        assert_eq!(again.account.id, id);
    }

    #[tokio::test]
    async fn a_single_account_pool_does_not_hand_it_out_twice_at_once() {
        let far = crate::now() + 100_000;
        let pool = pool(store_with(&[("only@b.c", "r1", far)]));
        let _held = pool.acquire_build().await.unwrap();
        let err = pool.acquire_build().await.unwrap_err();
        assert!(err.to_string().contains("no schedulable"));
    }

    #[test]
    fn ranking_mixes_unmeasured_accounts_and_skips_the_nearly_empty() {
        // Proven-first parked every fresh import behind the measured slice, so
        // entitlements never got discovered by live traffic. A drained account
        // cannot answer a request anyway.
        let far = crate::now() + 100_000;
        let store = store_with(&[
            ("drained@b.c", "r1", far),
            ("rich@b.c", "r2", far),
            ("unproven@b.c", "r3", far),
        ]);
        let quota = crate::upstream::RateLimit {
            limit_tokens: 1_000,
            remaining_tokens: 1_000,
            limit_requests: 21,
            remaining_requests: 21,
        };
        let by_email = |email: &str| {
            store
                .list(None)
                .unwrap()
                .into_iter()
                .find(|a| a.email == email)
                .unwrap()
        };
        let spend = |email: &str, tokens: i64| {
            store
                .record_success_with_usage(
                    by_email(email).id,
                    "grok-4.6",
                    crate::now(),
                    &crate::store::Usage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: tokens,
                        cost_ticks: 0,
                    },
                    Some(&quota),
                )
                .unwrap();
        };
        spend("drained@b.c", 990);
        spend("rich@b.c", 10);

        let pool = pool(store);
        pool.refill().unwrap();
        let order: Vec<String> = pool
            .ready
            .lock()
            .unwrap()
            .queue
            .iter()
            .map(|a| a.email.clone())
            .collect();
        assert_eq!(
            order,
            vec!["rich@b.c", "unproven@b.c"],
            "serve the rich first, explore the unknown after failover, skip the nearly empty"
        );
    }

    #[test]
    fn an_unmeasured_account_ranks_like_a_full_typical_one() {
        let full = Account {
            limit_tokens: crate::model::TYPICAL_TOKEN_ENTITLEMENT,
            total_tokens: 0,
            ..Default::default()
        };
        let drained = Account {
            limit_tokens: 1_000,
            total_tokens: 1_000,
            ..Default::default()
        };
        let unknown = Account {
            limit_tokens: -1,
            total_tokens: 0,
            ..Default::default()
        };
        assert_eq!(remaining_budget(&unknown), remaining_budget(&full));
        assert!(remaining_budget(&unknown) > remaining_budget(&drained));
    }

    #[test]
    fn keepalive_skips_an_account_that_just_served_traffic() {
        let now = 10_000;
        let fresh = Account {
            last_used_at: now - 60,
            expires_at: now + 100_000,
            access_token: "a".into(),
            provider: Provider::Build,
            ..Default::default()
        };
        let stale = Account {
            last_used_at: now - 10_000,
            expires_at: now + 100_000,
            access_token: "a".into(),
            provider: Provider::Build,
            ..Default::default()
        };
        let expiring = Account {
            last_used_at: now - 60,
            expires_at: now + 10,
            access_token: "a".into(),
            provider: Provider::Build,
            ..Default::default()
        };
        assert!(!worth_keepalive(&fresh, now));
        assert!(worth_keepalive(&stale, now));
        assert!(worth_keepalive(&expiring, now));
        assert!(worth_keepalive(
            &Account {
                last_used_at: 0,
                provider: Provider::Build,
                ..Default::default()
            },
            now
        ));
    }

    #[test]
    fn dead_and_still_cooling_accounts_never_reach_the_queue() {
        let store = store_with(&[("dead@b.c", "r1", 0), ("cooling@b.c", "r2", 0)]);
        let accounts = store.list(None).unwrap();
        let now = crate::now();
        store
            .mark_health(accounts[0].id, Health::NeedsReauth, 0, "dead", now)
            .unwrap();
        store
            .mark_health(accounts[1].id, Health::Cooling, now + 500, "slow down", now)
            .unwrap();

        let pool = pool(store);
        pool.refill().unwrap();
        assert_eq!(pool.ready_depth().0, 0, "neither is schedulable yet");
    }

    #[test]
    fn a_cooled_account_returns_once_its_window_passes() {
        let store = store_with(&[("cooling@b.c", "r1", 0)]);
        let id = store.list(None).unwrap()[0].id;
        // Already expired: cooling is a deadline, not a flag.
        store
            .mark_health(id, Health::Cooling, crate::now() - 1, "was busy", 0)
            .unwrap();

        let pool = pool(store);
        pool.refill().unwrap();
        assert_eq!(pool.ready_depth().0, 1);
    }

    #[test]
    fn web_accounts_are_never_scheduled_on_the_build_path() {
        let store = Store::open_in_memory().unwrap();
        let web: AccountImport =
            serde_json::from_value(serde_json::json!({"email":"w@b.c","sso_token":"s"})).unwrap();
        store.import(Some(Provider::Web), &[web], 1).unwrap();

        let pool = pool(store);
        pool.refill().unwrap();
        assert_eq!(pool.ready_depth().0, 0);
    }

    #[test]
    fn traffic_spreads_instead_of_grinding_one_account_down() {
        // Equal standing, so least-recently-used decides and the pool rotates.
        let far = crate::now() + 100_000;
        let store = store_with(&[("a@b.c", "r1", far), ("b@b.c", "r2", far)]);
        let accounts = store.list(None).unwrap();
        store
            .clear_failure(accounts[0].id, "grok-4.6", 500)
            .unwrap();
        store
            .record_failure(accounts[0].id, Health::Active, 0, "", 500)
            .unwrap();
        store
            .record_failure(accounts[1].id, Health::Active, 0, "", 100)
            .unwrap();

        let pool = pool(store);
        pool.refill().unwrap();
        let order: Vec<String> = pool
            .ready
            .lock()
            .unwrap()
            .queue
            .iter()
            .map(|a| a.email.clone())
            .collect();
        assert_eq!(order, vec!["b@b.c", "a@b.c"], "oldest use goes first");
    }

    #[test]
    fn a_failed_account_leaves_the_ready_queue_immediately() {
        let far = crate::now() + 100_000;
        let store = store_with(&[("bad@b.c", "r1", far), ("good@b.c", "r2", far)]);
        let pool = pool(store.clone());
        pool.refill().unwrap();
        assert_eq!(pool.ready_depth().0, 2);

        let bad = store
            .list(None)
            .unwrap()
            .into_iter()
            .find(|a| a.email == "bad@b.c")
            .unwrap();
        pool.report_failure(&bad, &Failure::Revoked, "gone")
            .unwrap();

        let queued: Vec<String> = pool
            .ready
            .lock()
            .unwrap()
            .queue
            .iter()
            .map(|a| a.email.clone())
            .collect();
        assert_eq!(queued, vec!["good@b.c"], "the revoked one was evicted");
    }

    #[test]
    fn failure_classification_drives_cooldown_length() {
        let store = store_with(&[("a@b.c", "r1", 0)]);
        let pool = pool(store.clone());
        let account = store.list(None).unwrap().remove(0);

        pool.report_failure(&account, &Failure::Forbidden, "denied")
            .unwrap();
        let after = store.get(account.id).unwrap().unwrap();
        assert_eq!(after.health, Health::Forbidden);
        assert_eq!(after.cooling_until, 0);

        pool.report_failure(&account, &Failure::Cooling(600), "429")
            .unwrap();
        let after = store.get(account.id).unwrap().unwrap();
        assert_eq!(after.health, Health::Cooling);
        assert!(after.cooling_until > crate::now());
    }

    #[test]
    fn advertised_models_prefer_observed_over_fallback() {
        let store = store_with(&[("a@b.c", "r1", 0)]);
        let pool = pool(store.clone());
        assert_eq!(
            pool.advertised_models().unwrap(),
            vec![crate::upstream::FALLBACK_MODEL.to_string()]
        );

        let id = store.list(None).unwrap()[0].id;
        store
            .record_success_with_usage(id, "grok-4.6", 10, &Usage::default(), None)
            .unwrap();
        assert_eq!(pool.advertised_models().unwrap(), vec!["grok-4.6"]);
    }

    #[test]
    fn every_failure_kind_has_its_own_quota_bucket() {
        // Revoked once fell into the `unreachable` catch-all, which made a pool
        // of dead credentials read as a network problem. Each kind must land in
        // a distinct bucket so the report cannot mislead that way again.
        fn bucket(failure: &Failure) -> &'static str {
            match failure {
                Failure::Revoked => "revoked",
                Failure::Forbidden => "no_permission",
                Failure::Cooling(secs) if *secs >= 1800 => "no_credit",
                Failure::Cooling(_) => "rate_limited",
                Failure::Transient => "unreachable",
            }
        }
        assert_eq!(bucket(&Failure::Revoked), "revoked");
        assert_eq!(bucket(&Failure::Forbidden), "no_permission");
        assert_eq!(bucket(&Failure::Cooling(1_800)), "no_credit");
        assert_eq!(bucket(&Failure::Cooling(600)), "rate_limited");
        assert_eq!(bucket(&Failure::Transient), "unreachable");
    }

    #[test]
    fn healthy_count_excludes_dead_accounts() {
        let store = store_with(&[("a@b.c", "r1", 0), ("b@b.c", "r2", 0)]);
        let pool = pool(store.clone());
        assert_eq!(pool.healthy_count().unwrap(), 2);

        let id = store.list(None).unwrap()[0].id;
        store
            .mark_health(id, Health::NeedsReauth, 0, "revoked", 2)
            .unwrap();
        assert_eq!(pool.healthy_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_revoked_pool_is_not_probed() {
        // Sweeping needs_reauth accounts spent the chat budget proving they
        // were still dead, so unmeasured live accounts never got measured.
        let store = store_with(&[("dead@b.c", "r1", 0)]);
        let id = store.list(None).unwrap()[0].id;
        store
            .mark_health(id, Health::NeedsReauth, 0, "revoked", 1)
            .unwrap();
        let pool = pool(store);
        let report = pool.probe_pool(None, 200, 1).await.unwrap();
        assert_eq!(report.checked, 0);
    }

    #[test]
    fn chat_probes_skip_accounts_we_already_measured() {
        let now = 10_000;
        let measured = Account {
            limit_tokens: 1_000_000,
            verified_at: now,
            provider: Provider::Build,
            health: Health::Active,
            ..Default::default()
        };
        let unknown = Account {
            limit_tokens: -1,
            verified_at: 0,
            provider: Provider::Build,
            health: Health::Active,
            access_token: "a".into(),
            expires_at: now + 100_000,
            ..Default::default()
        };
        assert!(!probe_eligible(&measured, Some(Probe::Chat), now));
        assert!(probe_eligible(&unknown, Some(Probe::Chat), now));
        assert!(probe_eligible(&measured, Some(Probe::Token), now));
        assert!(
            !probe_eligible(&unknown, Some(Probe::Token), now),
            "a live unmeasured token is the measure loop's job, not keepalive"
        );
    }
}
