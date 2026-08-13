//! Graded probes.
//!
//! Checking an account is not one operation but three, with very different
//! prices and very different answers:
//!
//! | probe    | cost                        | what it proves                    |
//! |----------|-----------------------------|-----------------------------------|
//! | `Token`  | one refresh call, free      | the credential still mints        |
//! | `Models` | one GET, free               | the token is accepted, and routing|
//! | `Chat`   | a few hundred of ITS tokens | it can generate, and its budget   |
//!
//! Only `Chat` reveals the entitlement, because the upstream reports it in a
//! chat response header and nowhere else. That makes it the only probe worth
//! spending on an unmeasured account, and the one to avoid on a measured one.

use std::time::Instant;

use anyhow::Result;

use crate::model::Account;
use crate::upstream::{Failure, RateLimit, Upstream};

/// Refresh this many seconds before the token actually expires.
pub const REFRESH_SKEW_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    Token,
    Models,
    Chat,
}

impl Probe {
    pub fn as_str(self) -> &'static str {
        match self {
            Probe::Token => "token",
            Probe::Models => "models",
            Probe::Chat => "chat",
        }
    }

    /// Roughly what running this costs from the account's own token budget.
    /// Used to keep a sweep from quietly draining the pool it is inspecting.
    pub fn budget_cost(self) -> i64 {
        match self {
            Probe::Token | Probe::Models => 0,
            // Measured: a 4-token completion still bills the system prompt.
            Probe::Chat => 600,
        }
    }

    /// The cheapest probe that would still teach us something new.
    ///
    /// An account with a known entitlement that has already served traffic only
    /// needs to be kept alive; paying for a chat there buys nothing. Nor is a
    /// chat probe worth running on an account too close to empty to answer it —
    /// that would spend the last of its budget proving it has none.
    pub fn cheapest_useful_for(account: &Account) -> Probe {
        let unmeasured = account.limit_tokens <= 0 || account.verified_at == 0;
        let affordable = account.limit_tokens <= 0
            || account.limit_tokens - account.total_tokens > Probe::Chat.budget_cost();
        if unmeasured && affordable {
            Probe::Chat
        } else {
            Probe::Token
        }
    }
}

/// What a probe concluded. `Alive` carries whatever the probe happened to
/// learn, which is more for the expensive probes than the cheap ones.
#[derive(Debug)]
pub enum Verdict {
    Alive {
        model: Option<String>,
        entitlement: Option<RateLimit>,
        usage: Option<crate::store::Usage>,
    },
    Dead {
        failure: Failure,
        error: String,
    },
}

#[derive(Debug)]
pub struct ProbeOutcome {
    pub probe: Probe,
    pub verdict: Verdict,
    pub latency_ms: u64,
}

impl ProbeOutcome {
    pub fn alive(&self) -> bool {
        matches!(self.verdict, Verdict::Alive { .. })
    }
}

/// Runs probes. Holds no state of its own so it can be shared freely.
pub struct Prober<'a> {
    pub upstream: &'a Upstream,
    pub store: &'a crate::store::Store,
}

impl Prober<'_> {
    /// Run one probe end to end and persist everything it revealed.
    ///
    /// Persisting here rather than at the call site is deliberate: a rotated
    /// refresh token that is not written back is a permanently dead account, so
    /// no caller should be able to forget.
    pub async fn run(&self, account: &Account, probe: Probe) -> Result<ProbeOutcome> {
        let started = Instant::now();
        let verdict = self.execute(account, probe).await;
        let outcome = ProbeOutcome {
            probe,
            verdict,
            latency_ms: started.elapsed().as_millis() as u64,
        };
        self.persist(account, &outcome)?;
        Ok(outcome)
    }

    async fn execute(&self, account: &Account, probe: Probe) -> Verdict {
        let access = match self.fresh_access(account).await {
            Ok(access) => access,
            Err(err) => return dead(&err),
        };
        if probe == Probe::Token {
            return Verdict::Alive {
                model: None,
                entitlement: None,
                usage: None,
            };
        }

        let ids = match self
            .upstream
            .list_models(&access, &account.proxy_url, &account.headers)
            .await
        {
            Ok(ids) => ids,
            Err(err) => return dead(&err),
        };
        // Never hard-code the model: the upstream renamed 4.5 to 4.6 overnight
        // once already, and an equality check turned a healthy pool into zero.
        let model = crate::upstream::pick_chat_model(&ids)
            .unwrap_or_else(|| crate::upstream::FALLBACK_MODEL.to_string());
        if probe == Probe::Models {
            return Verdict::Alive {
                model: Some(model),
                entitlement: None,
                usage: None,
            };
        }

        let payload = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with exactly OK"}],
            "max_tokens": 4,
            "stream": false,
        });
        match self
            .upstream
            .chat_completions(&access, &account.proxy_url, &account.headers, &payload)
            .await
        {
            Ok(outcome) => Verdict::Alive {
                model: Some(model),
                entitlement: Some(outcome.rate_limit),
                usage: Some(crate::store::Usage::from_response(&outcome.body)),
            },
            Err(err) => dead(&err),
        }
    }

    /// Mint a token if the current one is close to expiry, writing the rotated
    /// refresh token back before it is used anywhere.
    async fn fresh_access(&self, account: &Account) -> Result<String> {
        if !account.needs_refresh(crate::now(), REFRESH_SKEW_SECS) {
            return Ok(account.access_token.clone());
        }
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
        Ok(pair.access_token)
    }

    fn persist(&self, account: &Account, outcome: &ProbeOutcome) -> Result<()> {
        let now = crate::now();
        match &outcome.verdict {
            Verdict::Alive {
                model,
                entitlement,
                usage,
            } => {
                let model = model.as_deref().unwrap_or("");
                // Only a chat probe proves the account can serve, so only that
                // one is allowed to mark it verified.
                if outcome.probe == Probe::Chat {
                    self.store.record_success_with_usage(
                        account.id,
                        model,
                        now,
                        &usage.unwrap_or_default(),
                        entitlement.as_ref(),
                    )?;
                } else {
                    self.store.clear_failure(account.id, model, now)?;
                }
            }
            Verdict::Dead { failure, error } => {
                let cooling_until = match failure.cooling_secs() {
                    0 => 0,
                    secs => now + secs,
                };
                self.store.record_failure(
                    account.id,
                    failure.health(),
                    cooling_until,
                    &crate::upstream::truncate(error, 300),
                    now,
                )?;
            }
        }
        Ok(())
    }
}

fn dead(err: &anyhow::Error) -> Verdict {
    Verdict::Dead {
        failure: crate::pool::downcast_failure(err),
        error: err.to_string(),
    }
}

/// Aggregate of a probing run, reported per verdict so a dead pool never hides
/// behind what looks like a network blip.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProbeReport {
    pub checked: usize,
    pub alive: usize,
    /// Credential itself is gone; will not self-heal.
    pub revoked: usize,
    /// Upstream refuses to serve this account at all.
    pub no_permission: usize,
    /// Out of quota for now; recovers on its own window.
    pub no_credit: usize,
    pub rate_limited: usize,
    /// Never got an answer — says nothing about the account.
    pub unreachable: usize,
    /// Model the upstream is currently serving.
    pub model: String,
    /// Tokens this run spent out of the pool's own budget.
    pub budget_spent: i64,
    /// How many of each probe grade ran, so a cheap sweep is distinguishable
    /// from one that quietly chat-probed the whole pool.
    pub by_probe: std::collections::BTreeMap<&'static str, usize>,
    /// Mean upstream latency in milliseconds — a pool that is alive but slow
    /// looks identical to a healthy one without this.
    pub avg_latency_ms: u64,
    #[serde(skip)]
    latency_total_ms: u64,
}

impl ProbeReport {
    pub fn absorb(&mut self, outcome: &ProbeOutcome) {
        self.checked += 1;
        *self.by_probe.entry(outcome.probe.as_str()).or_default() += 1;
        self.latency_total_ms += outcome.latency_ms;
        self.avg_latency_ms = self.latency_total_ms / self.checked as u64;
        match &outcome.verdict {
            Verdict::Alive { model, usage, .. } => {
                self.alive += 1;
                if self.model.is_empty() {
                    if let Some(model) = model {
                        self.model = model.clone();
                    }
                }
                if let Some(usage) = usage {
                    self.budget_spent += usage.total_tokens;
                }
            }
            Verdict::Dead { failure, .. } => match failure {
                Failure::Revoked => self.revoked += 1,
                Failure::Forbidden => self.no_permission += 1,
                Failure::Cooling(secs) if *secs >= 1800 => self.no_credit += 1,
                Failure::Cooling(_) => self.rate_limited += 1,
                Failure::Transient => self.unreachable += 1,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Account;

    fn account(limit_tokens: i64, verified_at: i64) -> Account {
        Account {
            limit_tokens,
            verified_at,
            ..Default::default()
        }
    }

    #[test]
    fn an_unmeasured_account_is_worth_a_chat_probe() {
        // Entitlement lives only in a chat response header, so nothing cheaper
        // can discover it.
        assert_eq!(
            Probe::cheapest_useful_for(&account(-1, 0)),
            Probe::Chat,
            "never probed"
        );
        assert_eq!(
            Probe::cheapest_useful_for(&account(-1, 999)),
            Probe::Chat,
            "served traffic but entitlement still unknown"
        );
    }

    #[test]
    fn a_measured_account_only_needs_keeping_alive() {
        assert_eq!(
            Probe::cheapest_useful_for(&account(1_000_000, 999)),
            Probe::Token
        );
    }

    #[test]
    fn a_measured_but_unproven_account_still_gets_a_chat_probe() {
        // limit_tokens without verified_at cannot happen today, but treating it
        // as proven would let an unusable account into rotation.
        assert_eq!(
            Probe::cheapest_useful_for(&account(1_000_000, 0)),
            Probe::Chat
        );
    }

    #[test]
    fn only_the_chat_probe_costs_the_account_anything() {
        assert_eq!(Probe::Token.budget_cost(), 0);
        assert_eq!(Probe::Models.budget_cost(), 0);
        assert!(Probe::Chat.budget_cost() > 0);
    }

    #[test]
    fn the_report_separates_a_dead_pool_from_a_bad_network() {
        let mut report = ProbeReport::default();
        for failure in [
            Failure::Revoked,
            Failure::Forbidden,
            Failure::Cooling(3600),
            Failure::Cooling(30),
            Failure::Transient,
        ] {
            report.absorb(&ProbeOutcome {
                probe: Probe::Token,
                verdict: Verdict::Dead {
                    failure,
                    error: "x".into(),
                },
                latency_ms: 1,
            });
        }
        assert_eq!(report.checked, 5);
        assert_eq!(report.revoked, 1);
        assert_eq!(report.no_permission, 1);
        assert_eq!(report.no_credit, 1);
        assert_eq!(report.rate_limited, 1);
        assert_eq!(report.unreachable, 1);
        assert_eq!(report.alive, 0);
    }

    #[test]
    fn the_report_totals_what_probing_cost_the_pool() {
        let mut report = ProbeReport::default();
        for _ in 0..3 {
            report.absorb(&ProbeOutcome {
                probe: Probe::Chat,
                verdict: Verdict::Alive {
                    model: Some("grok-4.6".into()),
                    entitlement: None,
                    usage: Some(crate::store::Usage {
                        prompt_tokens: 200,
                        completion_tokens: 10,
                        total_tokens: 500,
                        cost_ticks: 0,
                    }),
                },
                latency_ms: 1,
            });
        }
        assert_eq!(report.alive, 3);
        assert_eq!(report.budget_spent, 1_500);
        assert_eq!(report.model, "grok-4.6");
    }
}
