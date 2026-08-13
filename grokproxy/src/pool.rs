//! Scheduling: claim an account, make sure its token is fresh, report outcomes.

use anyhow::{anyhow, Result};
use tracing::{debug, warn};

use crate::model::{Account, Health, Provider};
use crate::store::Store;
use crate::upstream::{Failure, Upstream, UpstreamError};

/// Refresh this many seconds before the token actually expires.
const REFRESH_SKEW_SECS: i64 = 300;

pub struct Pool {
    store: Store,
    upstream: Upstream,
    max_attempts: usize,
}

/// An account claimed for one request, with a usable access token.
#[derive(Debug)]
pub struct Lease {
    pub account: Account,
}

impl Pool {
    pub fn new(store: Store, upstream: Upstream, max_attempts: usize) -> Self {
        Pool {
            store,
            upstream,
            max_attempts: max_attempts.max(1),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn upstream(&self) -> &Upstream {
        &self.upstream
    }

    /// Claim the next Build account and guarantee a non-expired access token.
    ///
    /// A refresh failure is charged to that account and the next one is tried,
    /// so one revoked credential cannot fail the whole request.
    pub async fn acquire_build(&self) -> Result<Lease> {
        let mut last_error = String::from("no schedulable build account");
        for _ in 0..self.max_attempts {
            let now = crate::now();
            let Some(mut account) = self.store.claim_next(Provider::Build, now)? else {
                break;
            };

            if !account.needs_refresh(now, REFRESH_SKEW_SECS) {
                return Ok(Lease { account });
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
                    return Ok(Lease { account });
                }
                Err(err) => {
                    let failure = downcast_failure(&err);
                    last_error = format!("{}: {err}", account.email);
                    warn!(account = %account.email, error = %err, "refresh failed");
                    self.report_failure(&account, &failure, &err.to_string())?;
                }
            }
        }
        Err(anyhow!(last_error))
    }

    pub fn report_success(&self, account: &Account, model: &str) -> Result<()> {
        self.store.record_success(account.id, model, crate::now())?;
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
        Ok(())
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

    /// Refresh every Build account once and record what the upstream says.
    ///
    /// A pool imported from an old archive is mostly dead credentials. Without
    /// a sweep, every user request pays to rediscover that, and with a bounded
    /// attempt budget the request just fails. Running this once after a bulk
    /// import moves the dead ones to `needs_reauth` so the scheduler skips them.
    /// Refresh + a real chat probe, so the report reflects entitlement, not just
    /// whether the credential can still mint a token.
    ///
    /// `sweep` alone answers "is this token alive"; plenty of alive tokens are
    /// refused at the chat endpoint. Quota state only shows up when something
    /// actually asks the upstream to generate.
    pub async fn probe_quota(&self, limit: usize, concurrency: usize) -> Result<QuotaReport> {
        let accounts = self.store.list(Some(Provider::Build))?;
        let now = crate::now();
        let candidates: Vec<Account> = accounts
            .into_iter()
            .filter(|account| account.is_available(now))
            .take(if limit == 0 { usize::MAX } else { limit })
            .collect();

        let mut report = QuotaReport {
            checked: 0,
            usable: 0,
            no_permission: 0,
            no_credit: 0,
            rate_limited: 0,
            unreachable: 0,
            model: String::new(),
        };
        let width = concurrency.clamp(1, 16);
        for chunk in candidates.chunks(width) {
            let mut futures = Vec::with_capacity(chunk.len());
            for account in chunk {
                futures.push(async move {
                    let outcome = self.probe_one_quota(account).await;
                    (account, outcome)
                });
            }
            for (account, outcome) in futures::future::join_all(futures).await {
                report.checked += 1;
                match outcome {
                    Ok(model) => {
                        report.usable += 1;
                        if report.model.is_empty() {
                            report.model = model.clone();
                        }
                        self.report_success(account, &model)?;
                    }
                    Err(err) => {
                        let failure = downcast_failure(&err);
                        match &failure {
                            Failure::Forbidden => report.no_permission += 1,
                            Failure::Cooling(secs) if *secs >= 1800 => report.no_credit += 1,
                            Failure::Cooling(_) => report.rate_limited += 1,
                            _ => report.unreachable += 1,
                        }
                        self.report_failure(account, &failure, &err.to_string())?;
                    }
                }
            }
        }
        Ok(report)
    }

    /// One account: make sure the token is fresh, then actually ask for a reply.
    async fn probe_one_quota(&self, account: &Account) -> Result<String> {
        let mut access = account.access_token.clone();
        if account.needs_refresh(crate::now(), REFRESH_SKEW_SECS) {
            let pair = self
                .upstream
                .refresh_token(&account.refresh_token, &account.proxy_url)
                .await?;
            self.store.save_refreshed(
                account.id,
                &pair.access_token,
                &pair.refresh_token,
                pair.expires_at,
                crate::now(),
            )?;
            access = pair.access_token;
        }

        let ids = self
            .upstream
            .list_models(&access, &account.proxy_url, &account.headers)
            .await?;
        let model = crate::upstream::pick_chat_model(&ids)
            .unwrap_or_else(|| crate::upstream::FALLBACK_MODEL.to_string());

        let payload = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with exactly OK"}],
            "max_tokens": 4,
            "stream": false,
        });
        self.upstream
            .chat_completions(&access, &account.proxy_url, &account.headers, &payload)
            .await?;
        Ok(model)
    }

    pub async fn sweep(&self, limit: usize, concurrency: usize) -> Result<SweepReport> {
        let accounts = self.store.list(Some(Provider::Build))?;
        let candidates: Vec<Account> = accounts
            .into_iter()
            .filter(|account| !matches!(account.health, Health::Disabled | Health::NeedsReauth))
            .take(if limit == 0 { usize::MAX } else { limit })
            .collect();

        let mut report = SweepReport {
            checked: 0,
            alive: 0,
            revoked: 0,
            other: 0,
        };
        let width = concurrency.clamp(1, 16);
        for chunk in candidates.chunks(width) {
            let mut futures = Vec::with_capacity(chunk.len());
            for account in chunk {
                futures.push(async move {
                    let outcome = self
                        .upstream
                        .refresh_token(&account.refresh_token, &account.proxy_url)
                        .await;
                    (account, outcome)
                });
            }
            for (account, outcome) in futures::future::join_all(futures).await {
                report.checked += 1;
                match outcome {
                    Ok(pair) => {
                        report.alive += 1;
                        self.store.save_refreshed(
                            account.id,
                            &pair.access_token,
                            &pair.refresh_token,
                            pair.expires_at,
                            crate::now(),
                        )?;
                    }
                    Err(err) => {
                        let failure = downcast_failure(&err);
                        if failure == Failure::Revoked {
                            report.revoked += 1;
                        } else {
                            report.other += 1;
                        }
                        self.report_failure(account, &failure, &err.to_string())?;
                    }
                }
            }
        }
        Ok(report)
    }

    pub fn healthy_count(&self) -> Result<usize> {
        let now = crate::now();
        Ok(self
            .store
            .list(Some(Provider::Build))?
            .iter()
            .filter(|account| account.is_available(now) && account.health != Health::Disabled)
            .count())
    }
}

pub fn downcast_failure(err: &anyhow::Error) -> Failure {
    err.downcast_ref::<UpstreamError>()
        .map(UpstreamError::failure)
        .unwrap_or(Failure::Transient)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuotaReport {
    pub checked: usize,
    /// Chat actually returned a completion.
    pub usable: usize,
    /// Upstream refuses chat for this account (entitlement).
    pub no_permission: usize,
    /// Quota/credit exhausted; recovers on its own window.
    pub no_credit: usize,
    pub rate_limited: usize,
    /// Never got an answer — says nothing about the account.
    pub unreachable: usize,
    /// Model the upstream is currently serving.
    pub model: String,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct SweepReport {
    pub checked: usize,
    pub alive: usize,
    /// Refresh token rejected — these will never recover on their own.
    pub revoked: usize,
    /// Rate limited, entitlement denied, or unreachable.
    pub other: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AccountImport;

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
        store.record_success(id, "grok-4.6", 10).unwrap();
        assert_eq!(pool.advertised_models().unwrap(), vec!["grok-4.6"]);
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
}
