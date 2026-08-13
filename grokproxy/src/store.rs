//! SQLite-backed account store.
//!
//! Deliberately a single file with no external database: this service must be
//! deployable as one container plus one volume.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::{Account, AccountImport, Health, ImportOutcome, Provider};

/// What an account has spent against what the upstream says it is entitled to.
///
/// The upstream's `x-ratelimit-remaining-*` headers never move — verified with
/// six back-to-back calls — so they advertise the entitlement rather than count
/// down. Remaining quota is therefore ours to track: entitlement from the
/// header, spend from the `usage` block of every response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Budget {
    pub spent_tokens: i64,
    /// -1 when the upstream has never told us the entitlement.
    pub limit_tokens: i64,
}

impl Budget {
    pub fn known(&self) -> bool {
        self.limit_tokens > 0
    }

    pub fn spent(&self) -> bool {
        self.known() && self.spent_tokens >= self.limit_tokens
    }
}

/// What one upstream call consumed, as reported by the upstream itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    /// `cost_in_usd_ticks` from the response; 1e7 ticks = 1 USD.
    pub cost_ticks: i64,
}

impl Usage {
    /// Pull usage out of an OpenAI-shaped response body.
    pub fn from_response(body: &serde_json::Value) -> Self {
        let usage = match body.get("usage") {
            Some(value) => value,
            None => return Usage::default(),
        };
        let get = |key: &str| {
            usage
                .get(key)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0)
        };
        Usage {
            prompt_tokens: get("prompt_tokens"),
            completion_tokens: get("completion_tokens"),
            total_tokens: get("total_tokens"),
            cost_ticks: get("cost_in_usd_ticks"),
        }
    }
}

/// Filters for one page of the admin account list.
#[derive(Debug, Clone)]
pub struct AccountQuery {
    pub provider: Option<Provider>,
    pub health: Option<Health>,
    pub search: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

impl Default for AccountQuery {
    fn default() -> Self {
        AccountQuery {
            provider: None,
            health: None,
            search: None,
            limit: 50,
            offset: 0,
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    provider       TEXT    NOT NULL,
    email          TEXT    NOT NULL,
    health         TEXT    NOT NULL DEFAULT 'active',
    access_token   TEXT    NOT NULL DEFAULT '',
    refresh_token  TEXT    NOT NULL DEFAULT '',
    sso_token      TEXT    NOT NULL DEFAULT '',
    expires_at     INTEGER NOT NULL DEFAULT 0,
    proxy_url      TEXT    NOT NULL DEFAULT '',
    headers        TEXT    NOT NULL DEFAULT '',
    last_model     TEXT    NOT NULL DEFAULT '',
    last_used_at   INTEGER NOT NULL DEFAULT 0,
    cooling_until  INTEGER NOT NULL DEFAULT 0,
    success_count  INTEGER NOT NULL DEFAULT 0,
    failure_count  INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT    NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL DEFAULT 0,
    updated_at     INTEGER NOT NULL DEFAULT 0,
    UNIQUE(provider, email)
);
CREATE INDEX IF NOT EXISTS idx_accounts_pick
    ON accounts(provider, health, cooling_until, last_used_at);
"#;

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create data dir {}", parent.display()))?;
            }
        }
        let conn =
            Connection::open(path).with_context(|| format!("open sqlite at {}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // WAL keeps the scheduler's reads from blocking token writebacks.
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.pragma_update(None, "busy_timeout", 5_000).ok();
        conn.execute_batch(SCHEMA).context("apply schema")?;
        Self::migrate(&conn);
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Additive column migrations, safe to re-run.
    ///
    /// SQLite has no `ADD COLUMN IF NOT EXISTS`, and an existing column is the
    /// normal case on every restart, so a duplicate-column error is expected
    /// rather than exceptional.
    fn migrate(conn: &Connection) {
        // Usage accounting: the upstream exposes no quota endpoint (/usage,
        // /credits, /quota all 404), so "remaining" is unknowable. Chat
        // responses do carry token counts and a cost tick, which is enough to
        // show consumption per account.
        for column in [
            "prompt_tokens INTEGER NOT NULL DEFAULT 0",
            "completion_tokens INTEGER NOT NULL DEFAULT 0",
            "total_tokens INTEGER NOT NULL DEFAULT 0",
            "cost_ticks INTEGER NOT NULL DEFAULT 0",
            // Proven to have served a request. Distinct from `active`, which is
            // merely the state a freshly imported account starts in.
            "verified_at INTEGER NOT NULL DEFAULT 0",
            // Quota as last reported by x-ratelimit-* on a chat response.
            // -1 = never observed, so "unknown" stays distinct from "zero left".
            "limit_tokens INTEGER NOT NULL DEFAULT -1",
            "remaining_tokens INTEGER NOT NULL DEFAULT -1",
            "limit_requests INTEGER NOT NULL DEFAULT -1",
            "remaining_requests INTEGER NOT NULL DEFAULT -1",
            "quota_checked_at INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE accounts ADD COLUMN {column}"), []);
        }
    }

    fn row_to_account(row: &Row<'_>) -> rusqlite::Result<Account> {
        let headers_raw: String = row.get("headers")?;
        Ok(Account {
            id: row.get("id")?,
            provider: Provider::parse(&row.get::<_, String>("provider")?)
                .unwrap_or(Provider::Build),
            email: row.get("email")?,
            health: Health::parse(&row.get::<_, String>("health")?),
            access_token: row.get("access_token")?,
            refresh_token: row.get("refresh_token")?,
            sso_token: row.get("sso_token")?,
            expires_at: row.get("expires_at")?,
            proxy_url: row.get("proxy_url")?,
            headers: serde_json::from_str(&headers_raw).unwrap_or(serde_json::Value::Null),
            last_model: row.get("last_model")?,
            last_used_at: row.get("last_used_at")?,
            cooling_until: row.get("cooling_until")?,
            success_count: row.get("success_count")?,
            failure_count: row.get("failure_count")?,
            last_error: row.get("last_error")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            prompt_tokens: row.get("prompt_tokens").unwrap_or(0),
            completion_tokens: row.get("completion_tokens").unwrap_or(0),
            total_tokens: row.get("total_tokens").unwrap_or(0),
            cost_ticks: row.get("cost_ticks").unwrap_or(0),
            verified_at: row.get("verified_at").unwrap_or(0),
            limit_tokens: row.get("limit_tokens").unwrap_or(-1),
            remaining_tokens: row.get("remaining_tokens").unwrap_or(-1),
            limit_requests: row.get("limit_requests").unwrap_or(-1),
            remaining_requests: row.get("remaining_requests").unwrap_or(-1),
            quota_checked_at: row.get("quota_checked_at").unwrap_or(0),
        })
    }

    /// Page of accounts matching optional provider / health / email filters.
    ///
    /// The pool runs to thousands of rows, so the admin surface must never pull
    /// the whole table to render one screen.
    pub fn query(&self, filter: &AccountQuery) -> Result<(Vec<Account>, i64)> {
        let conn = self.conn.lock().unwrap();
        let provider = filter.provider.map(|p| p.as_str().to_string());
        let health = filter.health.map(|h| h.as_str().to_string());
        let search = filter
            .search
            .as_ref()
            .map(|value| format!("%{}%", value.trim().to_ascii_lowercase()));

        let where_sql = "WHERE (?1 IS NULL OR provider = ?1)
                           AND (?2 IS NULL OR health = ?2)
                           AND (?3 IS NULL OR email LIKE ?3)";

        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM accounts {where_sql}"),
            params![provider, health, search],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM accounts {where_sql}
             ORDER BY
               CASE health WHEN 'active' THEN 0 WHEN 'cooling' THEN 1 ELSE 2 END,
               last_used_at DESC, id
             LIMIT ?4 OFFSET ?5"
        ))?;
        let rows = stmt.query_map(
            params![
                provider,
                health,
                search,
                filter.limit.clamp(1, 500),
                filter.offset.max(0)
            ],
            Self::row_to_account,
        )?;
        Ok((rows.collect::<rusqlite::Result<Vec<_>>>()?, total))
    }

    pub fn list(&self, provider: Option<Provider>) -> Result<Vec<Account>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM accounts
             WHERE (?1 IS NULL OR provider = ?1)
             ORDER BY provider, email",
        )?;
        let filter = provider.map(|p| p.as_str().to_string());
        let rows = stmt.query_map(params![filter], Self::row_to_account)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Account>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT * FROM accounts WHERE id = ?1", params![id], |row| {
                Self::row_to_account(row)
            })
            .optional()?)
    }

    /// Upsert on `(provider, email)`.
    ///
    /// Blank incoming fields never overwrite stored values: the pipeline sends
    /// Web-only or Build-only payloads for the same address, and a partial
    /// submission must not wipe the other half of the credential set.
    pub fn upsert(&self, provider: Provider, item: &AccountImport, now: i64) -> Result<bool> {
        let email = item.resolved_email();
        let access = item.access_token.clone().unwrap_or_default();
        let refresh = item.refresh_token.clone().unwrap_or_default();
        let sso = item.resolved_sso();
        let proxy = item.proxy_url.clone().unwrap_or_default();
        let headers = item
            .headers
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let mut expires = item.resolved_expires_at();
        if expires == 0 && !access.is_empty() {
            expires = crate::jwt::access_token_expiry(&access).unwrap_or(0);
        }

        let conn = self.conn.lock().unwrap();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM accounts WHERE provider = ?1 AND email = ?2",
                params![provider.as_str(), email],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            conn.execute(
                "UPDATE accounts SET
                    access_token  = CASE WHEN ?1 <> '' THEN ?1 ELSE access_token END,
                    refresh_token = CASE WHEN ?2 <> '' THEN ?2 ELSE refresh_token END,
                    sso_token     = CASE WHEN ?3 <> '' THEN ?3 ELSE sso_token END,
                    expires_at    = CASE WHEN ?4 > 0   THEN ?4 ELSE expires_at END,
                    proxy_url     = CASE WHEN ?5 <> '' THEN ?5 ELSE proxy_url END,
                    headers       = CASE WHEN ?6 <> '' THEN ?6 ELSE headers END,
                    -- Fresh credentials revive a dead account; that is the whole
                    -- point of re-importing one.
                    health        = CASE WHEN ?2 <> '' OR ?3 <> '' THEN 'active' ELSE health END,
                    cooling_until = CASE WHEN ?2 <> '' OR ?3 <> '' THEN 0 ELSE cooling_until END,
                    last_error    = CASE WHEN ?2 <> '' OR ?3 <> '' THEN '' ELSE last_error END,
                    updated_at    = ?7
                 WHERE id = ?8",
                params![access, refresh, sso, expires, proxy, headers, now, id],
            )?;
            return Ok(false);
        }

        conn.execute(
            "INSERT INTO accounts
                (provider, email, health, access_token, refresh_token, sso_token,
                 expires_at, proxy_url, headers, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                provider.as_str(),
                email,
                access,
                refresh,
                sso,
                expires,
                proxy,
                headers,
                now
            ],
        )?;
        Ok(true)
    }

    pub fn import(
        &self,
        default_provider: Option<Provider>,
        items: &[AccountImport],
        now: i64,
    ) -> Result<ImportOutcome> {
        let mut outcome = ImportOutcome {
            inserted: 0,
            updated: 0,
            skipped: Vec::new(),
        };
        for item in items {
            let provider = item
                .provider
                .as_deref()
                .and_then(Provider::parse)
                .or(default_provider);
            let Some(provider) = provider else {
                outcome.skipped.push("missing provider".into());
                continue;
            };
            let email = item.resolved_email();
            if email.is_empty() {
                outcome.skipped.push("missing email".into());
                continue;
            }
            let usable = match provider {
                Provider::Build => item
                    .refresh_token
                    .as_deref()
                    .map(|t| !t.trim().is_empty())
                    .unwrap_or(false),
                Provider::Web => !item.resolved_sso().is_empty(),
            };
            if !usable {
                outcome.skipped.push(format!(
                    "{email}: missing credential for {}",
                    provider.as_str()
                ));
                continue;
            }
            if self.upsert(provider, item, now)? {
                outcome.inserted += 1;
            } else {
                outcome.updated += 1;
            }
        }
        Ok(outcome)
    }

    /// Persist a rotated credential pair.
    ///
    /// xAI revokes the old refresh token the moment a new one is issued, so a
    /// refresh whose result is not stored permanently kills the account. This
    /// is the only place tokens are written after a refresh.
    pub fn save_refreshed(
        &self,
        id: i64,
        access_token: &str,
        refresh_token: &str,
        expires_at: i64,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET
                access_token  = ?1,
                refresh_token = CASE WHEN ?2 <> '' THEN ?2 ELSE refresh_token END,
                expires_at    = ?3,
                health        = CASE WHEN health = 'needs_reauth' THEN 'active' ELSE health END,
                last_error    = '',
                updated_at    = ?4
             WHERE id = ?5",
            params![access_token, refresh_token, expires_at, now, id],
        )?;
        Ok(())
    }

    pub fn mark_health(
        &self,
        id: i64,
        health: Health,
        cooling_until: i64,
        error: &str,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET health = ?1, cooling_until = ?2, last_error = ?3, updated_at = ?4
             WHERE id = ?5",
            params![health.as_str(), cooling_until, error, now, id],
        )?;
        Ok(())
    }

    pub fn record_success(&self, id: i64, model: &str, now: i64) -> Result<()> {
        self.record_success_with_usage(id, model, now, &Usage::default(), None)?;
        Ok(())
    }

    /// Success plus whatever the upstream reported it cost and how much the
    /// account is entitled to. Returns the account's running budget so the
    /// caller can retire it once it is spent.
    ///
    /// Quota columns are only written when the headers were actually present,
    /// so a response without them leaves the last known figures intact instead
    /// of blanking them to "unknown".
    pub fn record_success_with_usage(
        &self,
        id: i64,
        model: &str,
        now: i64,
        usage: &Usage,
        quota: Option<&crate::upstream::RateLimit>,
    ) -> Result<Budget> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET
                success_count     = success_count + 1,
                last_used_at      = ?1,
                verified_at       = ?1,
                last_model        = CASE WHEN ?2 <> '' THEN ?2 ELSE last_model END,
                health            = 'active',
                cooling_until     = 0,
                last_error        = '',
                prompt_tokens     = prompt_tokens + ?4,
                completion_tokens = completion_tokens + ?5,
                total_tokens      = total_tokens + ?6,
                cost_ticks        = cost_ticks + ?7,
                updated_at        = ?1
             WHERE id = ?3",
            params![
                now,
                model,
                id,
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens,
                usage.cost_ticks
            ],
        )?;
        if let Some(quota) = quota.filter(|q| q.observed()) {
            conn.execute(
                "UPDATE accounts SET
                    limit_tokens       = ?1,
                    remaining_tokens   = ?2,
                    limit_requests     = ?3,
                    remaining_requests = ?4,
                    quota_checked_at   = ?5
                 WHERE id = ?6",
                params![
                    quota.limit_tokens,
                    quota.remaining_tokens,
                    quota.limit_requests,
                    quota.remaining_requests,
                    now,
                    id
                ],
            )?;
        }
        let budget = conn.query_row(
            "SELECT total_tokens, limit_tokens FROM accounts WHERE id = ?1",
            params![id],
            |row| {
                Ok(Budget {
                    spent_tokens: row.get(0)?,
                    limit_tokens: row.get(1)?,
                })
            },
        )?;
        Ok(budget)
    }

    pub fn record_failure(
        &self,
        id: i64,
        health: Health,
        cooling_until: i64,
        error: &str,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET
                failure_count = failure_count + 1,
                last_used_at  = ?1,
                health        = ?2,
                cooling_until = ?3,
                last_error    = ?4,
                updated_at    = ?1
             WHERE id = ?5",
            params![now, health.as_str(), cooling_until, error, id],
        )?;
        Ok(())
    }

    /// Least-recently-used available account, claimed atomically.
    ///
    /// Proven accounts come first. A bulk import marks everything `active`
    /// without checking, so a pool can be mostly dead credentials; ordering by
    /// last_used_at alone then hands every request a string of corpses and it
    /// fails after exhausting its attempt budget. Accounts that have actually
    /// served a request sort ahead of never-verified ones, and within each
    /// group it is still least-recently-used.
    ///
    /// The claim bumps `last_used_at` inside the same lock so two concurrent
    /// requests cannot pick the same account and double its rate.
    pub fn claim_next(&self, provider: Provider, now: i64) -> Result<Option<Account>> {
        let conn = self.conn.lock().unwrap();
        let picked: Option<i64> = conn
            .query_row(
                "SELECT id FROM accounts
                 WHERE provider = ?1
                   AND (health = 'active' OR (health = 'cooling' AND cooling_until > 0 AND cooling_until <= ?2))
                 ORDER BY (verified_at = 0), last_used_at ASC, id ASC
                 LIMIT 1",
                params![provider.as_str(), now],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = picked else { return Ok(None) };
        conn.execute(
            "UPDATE accounts SET last_used_at = ?1, health = 'active', cooling_until = 0
             WHERE id = ?2",
            params![now, id],
        )?;
        conn.query_row("SELECT * FROM accounts WHERE id = ?1", params![id], |row| {
            Self::row_to_account(row)
        })
        .optional()
        .map_err(Into::into)
    }

    pub fn stats(&self) -> Result<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT provider, health, COUNT(*) FROM accounts GROUP BY provider, health")?;
        let mut out = serde_json::Map::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (provider, health, count) = row?;
            let entry = out
                .entry(provider)
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(map) = entry.as_object_mut() {
                map.insert(health, serde_json::Value::from(count));
            }
        }
        Ok(serde_json::Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_item(email: &str, refresh: &str) -> AccountImport {
        serde_json::from_value(serde_json::json!({
            "email": email,
            "access_token": "access-1",
            "refresh_token": refresh,
            "proxy_url": "http://u:p@host:1",
        }))
        .unwrap()
    }

    #[test]
    fn import_inserts_then_updates() {
        let store = Store::open_in_memory().unwrap();
        let items = vec![build_item("a@b.c", "r1")];
        let first = store.import(Some(Provider::Build), &items, 100).unwrap();
        assert_eq!((first.inserted, first.updated), (1, 0));

        let second = store.import(Some(Provider::Build), &items, 200).unwrap();
        assert_eq!((second.inserted, second.updated), (0, 1));
        assert_eq!(store.list(None).unwrap().len(), 1);
    }

    #[test]
    fn build_without_refresh_token_is_skipped() {
        let store = Store::open_in_memory().unwrap();
        let item: AccountImport =
            serde_json::from_value(serde_json::json!({"email": "a@b.c"})).unwrap();
        let outcome = store.import(Some(Provider::Build), &[item], 1).unwrap();
        assert_eq!(outcome.inserted, 0);
        assert_eq!(outcome.skipped.len(), 1);
    }

    #[test]
    fn web_import_needs_only_sso() {
        let store = Store::open_in_memory().unwrap();
        let item: AccountImport =
            serde_json::from_value(serde_json::json!({"name": "a@b.c", "sso_token": "s"})).unwrap();
        let outcome = store.import(Some(Provider::Web), &[item], 1).unwrap();
        assert_eq!(outcome.inserted, 1);
    }

    #[test]
    fn partial_reimport_does_not_wipe_the_other_half() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();

        // A later submission that only carries a proxy must keep the tokens.
        let thin: AccountImport =
            serde_json::from_value(serde_json::json!({"email":"a@b.c","refresh_token":"r2"}))
                .unwrap();
        store.import(Some(Provider::Build), &[thin], 2).unwrap();

        let account = &store.list(None).unwrap()[0];
        assert_eq!(account.refresh_token, "r2");
        assert_eq!(account.access_token, "access-1");
        assert_eq!(account.proxy_url, "http://u:p@host:1");
    }

    #[test]
    fn reimport_revives_a_dead_account() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;
        store
            .mark_health(id, Health::NeedsReauth, 0, "revoked", 2)
            .unwrap();

        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r2")], 3)
            .unwrap();
        let account = store.get(id).unwrap().unwrap();
        assert_eq!(account.health, Health::Active);
        assert!(account.last_error.is_empty());
    }

    #[test]
    fn rotated_refresh_token_is_persisted() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;

        store.save_refreshed(id, "access-2", "r2", 999, 5).unwrap();
        let account = store.get(id).unwrap().unwrap();
        assert_eq!(account.refresh_token, "r2");
        assert_eq!(account.access_token, "access-2");
        assert_eq!(account.expires_at, 999);
    }

    #[test]
    fn refresh_without_rotation_keeps_the_old_token() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;

        store.save_refreshed(id, "access-2", "", 999, 5).unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().refresh_token, "r1");
    }

    #[test]
    fn claim_is_least_recently_used_and_exclusive() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(
                Some(Provider::Build),
                &[build_item("a@b.c", "r1"), build_item("b@b.c", "r2")],
                1,
            )
            .unwrap();

        let first = store.claim_next(Provider::Build, 10).unwrap().unwrap();
        let second = store.claim_next(Provider::Build, 11).unwrap().unwrap();
        assert_ne!(first.id, second.id);

        // Round robin returns to the oldest.
        let third = store.claim_next(Provider::Build, 12).unwrap().unwrap();
        assert_eq!(third.id, first.id);
    }

    #[test]
    fn claim_skips_dead_and_still_cooling_accounts() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(
                Some(Provider::Build),
                &[build_item("a@b.c", "r1"), build_item("b@b.c", "r2")],
                1,
            )
            .unwrap();
        let accounts = store.list(None).unwrap();
        store
            .mark_health(accounts[0].id, Health::NeedsReauth, 0, "dead", 2)
            .unwrap();
        store
            .mark_health(accounts[1].id, Health::Cooling, 500, "slow down", 2)
            .unwrap();

        assert!(store.claim_next(Provider::Build, 499).unwrap().is_none());
        let revived = store.claim_next(Provider::Build, 500).unwrap().unwrap();
        assert_eq!(revived.id, accounts[1].id);
    }

    #[test]
    fn claim_does_not_cross_providers() {
        let store = Store::open_in_memory().unwrap();
        let web: AccountImport =
            serde_json::from_value(serde_json::json!({"email":"w@b.c","sso_token":"s"})).unwrap();
        store.import(Some(Provider::Web), &[web], 1).unwrap();
        assert!(store.claim_next(Provider::Build, 10).unwrap().is_none());
        assert!(store.claim_next(Provider::Web, 10).unwrap().is_some());
    }

    #[test]
    fn same_email_can_hold_both_providers() {
        let store = Store::open_in_memory().unwrap();
        let build = build_item("a@b.c", "r1");
        let web: AccountImport =
            serde_json::from_value(serde_json::json!({"email":"a@b.c","sso_token":"s"})).unwrap();
        store.import(Some(Provider::Build), &[build], 1).unwrap();
        store.import(Some(Provider::Web), &[web], 1).unwrap();
        assert_eq!(store.list(None).unwrap().len(), 2);
    }

    #[test]
    fn success_clears_cooling_and_records_model() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;
        store
            .record_failure(id, Health::Cooling, 900, "429", 2)
            .unwrap();
        store.record_success(id, "grok-4.6", 3).unwrap();

        let account = store.get(id).unwrap().unwrap();
        assert_eq!(account.health, Health::Active);
        assert_eq!(account.cooling_until, 0);
        assert_eq!(account.last_model, "grok-4.6");
        assert_eq!(account.success_count, 1);
        assert_eq!(account.failure_count, 1);
    }

    #[test]
    fn usage_is_parsed_from_a_real_response_shape() {
        let body = serde_json::json!({
            "usage": {
                "prompt_tokens": 208,
                "completion_tokens": 4,
                "total_tokens": 437,
                "cost_in_usd_ticks": 15020000
            }
        });
        let usage = Usage::from_response(&body);
        assert_eq!(usage.prompt_tokens, 208);
        assert_eq!(usage.total_tokens, 437);
        assert_eq!(usage.cost_ticks, 15020000);
    }

    #[test]
    fn missing_usage_is_zero_not_an_error() {
        let usage = Usage::from_response(&serde_json::json!({"choices": []}));
        assert_eq!(usage.total_tokens, 0);
    }

    #[test]
    fn usage_accumulates_across_calls() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;
        let usage = Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 20,
            cost_ticks: 1_000_000,
        };
        store
            .record_success_with_usage(id, "grok-4.6", 5, &usage, None)
            .unwrap();
        store
            .record_success_with_usage(id, "grok-4.6", 6, &usage, None)
            .unwrap();

        let account = store.get(id).unwrap().unwrap();
        assert_eq!(account.total_tokens, 40);
        assert_eq!(account.cost_ticks, 2_000_000);
        assert_eq!(account.success_count, 2);
    }

    #[test]
    fn entitlement_is_stored_and_the_budget_tracks_spend() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("q@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;
        let quota = crate::upstream::RateLimit {
            limit_tokens: 100,
            remaining_tokens: 100,
            limit_requests: 21,
            remaining_requests: 21,
        };
        let usage = Usage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            cost_ticks: 0,
        };

        let budget = store
            .record_success_with_usage(id, "grok-4.6", 5, &usage, Some(&quota))
            .unwrap();
        assert_eq!(budget.limit_tokens, 100);
        assert_eq!(budget.spent_tokens, 30);
        assert!(!budget.spent());

        // Headers advertise the same entitlement forever; spend is what moves.
        for tick in 6..=9 {
            store
                .record_success_with_usage(id, "grok-4.6", tick, &usage, Some(&quota))
                .unwrap();
        }
        let budget = store
            .record_success_with_usage(id, "grok-4.6", 10, &usage, Some(&quota))
            .unwrap();
        assert!(budget.spent(), "180 tokens spent against a 100 entitlement");
    }

    #[test]
    fn a_response_without_quota_headers_keeps_the_known_entitlement() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("q@b.c", "r1")], 1)
            .unwrap();
        let id = store.list(None).unwrap()[0].id;
        let quota = crate::upstream::RateLimit {
            limit_tokens: 1_000_000,
            remaining_tokens: 1_000_000,
            limit_requests: 21,
            remaining_requests: 21,
        };
        store
            .record_success_with_usage(id, "grok-4.6", 5, &Usage::default(), Some(&quota))
            .unwrap();

        // A later response that omits the headers must not blank the figure.
        let budget = store
            .record_success_with_usage(id, "grok-4.6", 6, &Usage::default(), None)
            .unwrap();
        assert_eq!(budget.limit_tokens, 1_000_000);
    }

    #[test]
    fn an_unknown_entitlement_never_counts_as_spent() {
        // Otherwise a never-used account would be retired before its first call.
        let budget = Budget {
            spent_tokens: 999,
            limit_tokens: -1,
        };
        assert!(!budget.known());
        assert!(!budget.spent());
    }

    #[test]
    fn proven_accounts_are_scheduled_before_never_verified_ones() {
        // A bulk import marks everything active without checking. Ordering by
        // last_used_at alone would hand a request a string of dead credentials.
        let store = Store::open_in_memory().unwrap();
        store
            .import(
                Some(Provider::Build),
                &[
                    build_item("never@b.c", "r1"),
                    build_item("proven@b.c", "r2"),
                ],
                1,
            )
            .unwrap();
        let accounts = store.list(None).unwrap();
        let proven = accounts.iter().find(|a| a.email == "proven@b.c").unwrap();
        // Mark it proven, and make it the *most* recently used so plain LRU
        // ordering would put it last.
        store.record_success(proven.id, "grok-4.6", 9_999).unwrap();

        let picked = store.claim_next(Provider::Build, 10_000).unwrap().unwrap();
        assert_eq!(picked.email, "proven@b.c");
    }

    #[test]
    fn query_pages_without_loading_everything() {
        let store = Store::open_in_memory().unwrap();
        let items: Vec<AccountImport> = (0..30)
            .map(|i| build_item(&format!("u{i:02}@b.c"), "r"))
            .collect();
        store.import(Some(Provider::Build), &items, 1).unwrap();

        let (page, total) = store
            .query(&AccountQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.len(), 10);
        assert_eq!(total, 30);

        let (page2, _) = store
            .query(&AccountQuery {
                limit: 10,
                offset: 25,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page2.len(), 5);
    }

    #[test]
    fn query_filters_by_health_and_email() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(
                Some(Provider::Build),
                &[build_item("alpha@b.c", "r"), build_item("beta@b.c", "r")],
                1,
            )
            .unwrap();
        let id = store.list(None).unwrap()[0].id;
        store
            .mark_health(id, Health::NeedsReauth, 0, "dead", 2)
            .unwrap();

        let (dead, total) = store
            .query(&AccountQuery {
                health: Some(Health::NeedsReauth),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(dead.len(), 1);

        let (found, _) = store
            .query(&AccountQuery {
                search: Some("BET".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].email, "beta@b.c");
    }

    #[test]
    fn query_puts_usable_accounts_first() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(
                Some(Provider::Build),
                &[build_item("a@b.c", "r"), build_item("b@b.c", "r")],
                1,
            )
            .unwrap();
        let first = store.list(None).unwrap()[0].id;
        store
            .mark_health(first, Health::NeedsReauth, 0, "dead", 2)
            .unwrap();

        let (page, _) = store.query(&AccountQuery::default()).unwrap();
        // An operator opening the page cares about what is serving traffic.
        assert_eq!(page[0].health, Health::Active);
    }

    #[test]
    fn stats_group_by_provider_and_health() {
        let store = Store::open_in_memory().unwrap();
        store
            .import(Some(Provider::Build), &[build_item("a@b.c", "r1")], 1)
            .unwrap();
        let stats = store.stats().unwrap();
        assert_eq!(stats["build"]["active"], 1);
    }
}
