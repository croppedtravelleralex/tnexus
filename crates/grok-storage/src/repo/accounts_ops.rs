//! G3-P5 路由候选查询 + 写路径（对齐 Go `account_repository.go` 的
//! `ListRoutingCandidates` / `hydrateRoutingCandidates` / `UpdateHealth` / `ClaimQuotaProbe` 等）。

use std::collections::HashMap;

use async_trait::async_trait;
use grok_domain::{
    Account, AuthStatus, Billing, ModelQuotaBlock, ModelState, Provider, QuotaRecovery,
    RoutingCandidate,
};
use sqlx::{postgres::PgRow, PgPool, Row};

use super::account::{provider_from_str, PgAccountRepository};
use crate::StorageError;
use super::routing::{
    fetch_billings, fetch_quota_recoveries, fetch_quota_windows_first, to_model_quota_block,
};

/// 路由候选查询（对齐 Go `ListRoutingCandidates` / `ListRoutingCandidatesByIDs`）。
#[async_trait]
pub trait RoutingCandidateRepository {
    async fn list_routing_candidates(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
    ) -> Result<Vec<RoutingCandidate>, StorageError>;

    async fn list_routing_candidates_by_ids(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        ids: &[i64],
    ) -> Result<Vec<RoutingCandidate>, StorageError>;
}

/// 账号写路径（对齐 Go UpdateHealth / ClaimQuotaProbe / Save* / Delete）。
#[async_trait]
pub trait AccountOps {
    async fn update_health(
        &self,
        account_id: i64,
        failure_count: i32,
        cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
        reason: &str,
        reset_last_success: bool,
    ) -> Result<(), StorageError>;

    /// 标记账号可删（对齐 Go `markBuildDeletable`）：禁用 + reauthRequired + 去冷却 +
    /// `deletable: {reason}` 前缀（≤512 字符），供四池 delete 池巡检与 admin 清理。
    async fn mark_deletable(&self, account_id: i64, reason: &str) -> Result<(), StorageError>;

    /// 记录账号观察到的最新上游模型（对齐 Go `ObserveResponseModel`）：更新
    /// `grok_accounts.observed_model`（验证池 → 调度池毕业的关键写路径）。
    async fn observe_model(&self, account_id: i64, model: &str) -> Result<(), StorageError>;

    /// 抢占配额探针；返回是否抢占成功（Go `ClaimQuotaProbe`）。
    async fn claim_quota_probe(
        &self,
        account_id: i64,
        now: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, StorageError>;

    async fn clear_quota_recovery(&self, account_id: i64) -> Result<(), StorageError>;
    async fn save_quota_recovery(&self, recovery: QuotaRecovery) -> Result<(), StorageError>;
    async fn save_model_state(&self, state: ModelState) -> Result<(), StorageError>;
    async fn upsert_model_quota_block(&self, block: ModelQuotaBlock) -> Result<(), StorageError>;
    async fn delete_account(&self, account_id: i64) -> Result<(), StorageError>;
}

const ROUTING_COLS: &str = "id, identity_key, provider, enabled, auth_status, priority, \
     observed_model, max_concurrent, minimum_remaining, failure_count, cooldown_until, \
     last_error, last_used_at, observed_model_at, name, email, user_id, team_id, source_key, \
     created_at, updated_at";

fn auth_status_from_str_raw(s: &str) -> Result<AuthStatus, StorageError> {
    match s {
        "unknown" => Ok(AuthStatus::Unknown),
        "active" => Ok(AuthStatus::Active),
        "restricted" => Ok(AuthStatus::Restricted),
        "banned" => Ok(AuthStatus::Banned),
        "reauth_required" | "reauthRequired" => Ok(AuthStatus::ReauthRequired),
        other => Err(StorageError::Decode(format!("unknown auth_status: {other}"))),
    }
}

fn map_routing_row(row: &PgRow) -> Result<Account, StorageError> {
    let provider_str: String = row.try_get("provider")?;
    let status_str: String = row.try_get("auth_status")?;
    Ok(Account {
        id: row.try_get("id")?,
        identity_key: row.try_get("identity_key")?,
        provider: provider_from_str(&provider_str)?,
        enabled: row.try_get("enabled")?,
        auth_status: auth_status_from_str_raw(&status_str)?,
        priority: row.try_get("priority")?,
        observed_model: row.try_get("observed_model")?,
        observed_model_at: row.try_get("observed_model_at")?,
        max_concurrent: row.try_get("max_concurrent")?,
        minimum_remaining: row.try_get("minimum_remaining")?,
        failure_count: row.try_get("failure_count")?,
        cooldown_until: row.try_get("cooldown_until")?,
        last_error: row.try_get("last_error")?,
        last_used_at: row.try_get("last_used_at")?,
        name: row.try_get("name")?,
        email: row.try_get("email")?,
        user_id: row.try_get("user_id")?,
        team_id: row.try_get("team_id")?,
        source_key: row.try_get("source_key")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        ..Default::default()
    })
}

/// `deletable:` 前缀 + 理由，≤512 字符（对齐 Go `markBuildDeletable` 的 last_error 组装）。
pub fn deletable_reason(reason: &str) -> String {
    let mut text = format!("deletable: {}", reason.trim());
    text.truncate(512);
    text
}

/// ListEnabled（对齐 Go：provider + enabled + auth_status='active'，priority DESC, id ASC）。
async fn list_enabled(
    pool: &PgPool,
    provider: Provider,
    ids: Option<&[i64]>,
) -> Result<Vec<Account>, StorageError> {
    let where_ids = match ids {
        Some(_) => " AND id = ANY($2::bigint[])",
        None => "",
    };
    let sql = format!(
        "SELECT {ROUTING_COLS} FROM grok_accounts \
         WHERE provider = $1 AND enabled = true AND auth_status = 'active'{where_ids} \
         ORDER BY priority DESC, id ASC"
    );
    let rows = match ids {
        None => sqlx::query(&sql).bind(provider.as_str()).fetch_all(pool).await?,
        Some(ids) => sqlx::query(&sql)
            .bind(provider.as_str())
            .bind(ids)
            .fetch_all(pool)
            .await?,
    };
    rows.iter().map(map_routing_row).collect()
}

impl PgAccountRepository {
    /// 列出全部 Build 账号（含 disabled/deletable，供四池 delete 池巡检与池汇总）。
    /// 全列（ROUTING_COLS），优先级降序（对齐 Go `List(provider)`）。
    pub async fn list_build_accounts(
        &self,
    ) -> Result<Vec<Account>, StorageError> {
        let sql = format!(
            "SELECT {ROUTING_COLS} FROM grok_accounts WHERE provider = $1 \
             ORDER BY priority DESC, id ASC"
        );
        let rows = sqlx::query(&sql)
            .bind(Provider::GrokBuild.as_str())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(map_routing_row).collect()
    }

    /// 批量读额度恢复（Go `GetQuotaRecoveries`）。
    pub async fn recoveries(&self, ids: &[i64]) -> Result<HashMap<i64, QuotaRecovery>, StorageError> {
        fetch_quota_recoveries(&self.pool, ids).await
    }

    /// 批量读账单快照（Go `GetBillings`）。
    pub async fn billings(&self, ids: &[i64]) -> Result<HashMap<i64, Billing>, StorageError> {
        fetch_billings(&self.pool, ids).await
    }

    /// 读单个额度恢复；不存在返回 `Ok(None)`。
    pub async fn recovery(&self, id: i64) -> Result<Option<QuotaRecovery>, StorageError> {
        Ok(self.recoveries(&[id]).await?.remove(&id))
    }

    /// 读单个账单快照；不存在返回 `Ok(None)`。
    pub async fn billing(&self, id: i64) -> Result<Option<Billing>, StorageError> {
        Ok(self.billings(&[id]).await?.remove(&id))
    }

    /// hydrate 路由候选（对齐 Go `hydrateRoutingCandidates`）。
    pub(crate) async fn hydrate_routing_candidates(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        values: Vec<Account>,
    ) -> Result<Vec<RoutingCandidate>, StorageError> {
        let ids: Vec<i64> = values.iter().map(|a| a.id).collect();
        let billings = fetch_billings(&self.pool, &ids).await?;
        let recoveries = fetch_quota_recoveries(&self.pool, &ids).await?;
        let windows = if provider == Provider::GrokWeb || !quota_mode.trim().is_empty() {
            fetch_quota_windows_first(&self.pool, &ids, quota_mode, provider == Provider::GrokWeb)
                .await?
        } else {
            Default::default()
        };

        let mut known: std::collections::HashSet<i64> = Default::default();
        let mut supported: std::collections::HashSet<i64> = Default::default();
        let mut blocks: HashMap<i64, ModelQuotaBlock> = Default::default();
        let mut states: HashMap<i64, ModelState> = Default::default();
        let model = upstream_model.trim();
        if !model.is_empty() && !ids.is_empty() {
            // 能力同步过（Go model_sync_states.last_success_at IS NOT NULL → known）
            let rows = sqlx::query(
                "SELECT account_id FROM grok_model_sync_states \
                 WHERE account_id = ANY($1::bigint[]) AND last_success_at IS NOT NULL",
            )
            .bind(&ids)
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                known.insert(row.try_get("account_id")?);
            }
            // 能力表（支持指定模型）
            let rows = sqlx::query(
                "SELECT account_id FROM grok_model_capabilities \
                 WHERE account_id = ANY($1::bigint[]) AND upstream_model = $2",
            )
            .bind(&ids)
            .bind(model)
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                supported.insert(row.try_get("account_id")?);
            }
            // 未过期的模型额度 block
            let rows = sqlx::query(
                "SELECT account_id, upstream_model, reason, cooldown_until, updated_at \
                 FROM grok_model_quota_blocks \
                 WHERE account_id = ANY($1::bigint[]) AND upstream_model = $2 \
                   AND cooldown_until > now()",
            )
            .bind(&ids)
            .bind(model)
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                let block = to_model_quota_block(&row)?;
                blocks.insert(block.account_id, block);
            }
            // 模型状态（按目标模型过滤）
            let by_account = super::routing::fetch_model_states(&self.pool, &ids).await?;
            for (account_id, list) in by_account {
                if let Some(state) = list.into_iter().find(|s| s.upstream_model == model) {
                    states.insert(account_id, state);
                }
            }
        }

        let mut out = Vec::with_capacity(values.len());
        for account in values {
            let (capability_known, supports_model) = if known.contains(&account.id) {
                (true, supported.contains(&account.id))
            } else {
                (false, false)
            };
            let account_id = account.id;
            out.push(RoutingCandidate {
                account,
                billing: billings.get(&account_id).cloned(),
                quota: windows.get(&account_id).cloned(),
                recovery: recoveries.get(&account_id).cloned(),
                model_quota_block: blocks.get(&account_id).cloned(),
                model_state: states.get(&account_id).cloned(),
                model_capability_known: capability_known,
                supports_model,
            });
        }
        Ok(out)
    }
}

#[async_trait]
impl RoutingCandidateRepository for PgAccountRepository {
    async fn list_routing_candidates(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
    ) -> Result<Vec<RoutingCandidate>, StorageError> {
        let values = list_enabled(&self.pool, provider, None).await?;
        self.hydrate_routing_candidates(provider, upstream_model, quota_mode, values)
            .await
    }

    async fn list_routing_candidates_by_ids(
        &self,
        provider: Provider,
        upstream_model: &str,
        quota_mode: &str,
        ids: &[i64],
    ) -> Result<Vec<RoutingCandidate>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let values = list_enabled(&self.pool, provider, Some(ids)).await?;
        let by_id: HashMap<i64, Account> = values.into_iter().map(|a| (a.id, a)).collect();
        let ordered: Vec<Account> = ids.iter().filter_map(|id| by_id.get(id).cloned()).collect();
        self.hydrate_routing_candidates(provider, upstream_model, quota_mode, ordered)
            .await
    }
}

#[async_trait]
impl AccountOps for PgAccountRepository {
    async fn update_health(
        &self,
        account_id: i64,
        failure_count: i32,
        cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
        reason: &str,
        _reset_last_success: bool,
    ) -> Result<(), StorageError> {
        // Go：非空 reason 才写入 last_error。
        let last_error = if reason.is_empty() {
            None
        } else {
            Some(reason.to_string())
        };
        sqlx::query(
            "UPDATE grok_accounts SET failure_count = $2, cooldown_until = $3, \
             last_error = $4, updated_at = now() WHERE id = $1",
        )
        .bind(account_id)
        .bind(failure_count)
        .bind(cooldown_until)
        .bind(last_error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_deletable(&self, account_id: i64, reason: &str) -> Result<(), StorageError> {
        // 对齐 Go `markBuildDeletable`：enabled=false、reauthRequired、去冷却、
        // last_error = "deletable: " + reason（≤512）。
        let text = deletable_reason(reason);
        sqlx::query(
            "UPDATE grok_accounts SET enabled = false, auth_status = 'reauthRequired', \
             cooldown_until = NULL, last_error = $2, updated_at = now() WHERE id = $1",
        )
        .bind(account_id)
        .bind(text)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn observe_model(&self, account_id: i64, model: &str) -> Result<(), StorageError> {
        // 对齐 Go `ObserveResponseModel`：observed_model + observed_model_at + updated_at。
        sqlx::query(
            "UPDATE grok_accounts SET observed_model = $2, observed_model_at = now(), \
             updated_at = now() WHERE id = $1",
        )
        .bind(account_id)
        .bind(model)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn claim_quota_probe(
        &self,
        account_id: i64,
        now: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE grok_quota_recovery \
             SET status = 'probing', next_probe_at = $3, updated_at = $2 \
             WHERE account_id = $1 AND status = 'exhausted'",
        )
        .bind(account_id)
        .bind(now)
        .bind(until)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn clear_quota_recovery(&self, account_id: i64) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM grok_quota_recovery WHERE account_id = $1")
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn save_quota_recovery(&self, recovery: QuotaRecovery) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO grok_quota_recovery \
             (account_id, kind, status, confirmed_used, confirmed_limit, exhausted_at, \
              next_probe_at, last_confirmed_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now()) \
             ON CONFLICT (account_id) DO UPDATE SET \
               kind = EXCLUDED.kind, status = EXCLUDED.status, \
               confirmed_used = EXCLUDED.confirmed_used, \
               confirmed_limit = EXCLUDED.confirmed_limit, \
               exhausted_at = EXCLUDED.exhausted_at, \
               next_probe_at = EXCLUDED.next_probe_at, \
               last_confirmed_at = EXCLUDED.last_confirmed_at, \
               updated_at = now()",
        )
        .bind(recovery.account_id)
        .bind(recovery.kind.as_str())
        .bind(recovery.status.as_str())
        .bind(recovery.confirmed_used)
        .bind(recovery.confirmed_limit)
        .bind(recovery.exhausted_at)
        .bind(recovery.next_probe_at)
        .bind(recovery.last_confirmed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_model_state(&self, state: ModelState) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO grok_model_states \
             (account_id, upstream_model, status, reason, consecutive_failures, \
              last_attempt_at, last_success_at, cooldown_until, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now()) \
             ON CONFLICT (account_id, upstream_model) DO UPDATE SET \
               status = EXCLUDED.status, reason = EXCLUDED.reason, \
               consecutive_failures = EXCLUDED.consecutive_failures, \
               last_attempt_at = EXCLUDED.last_attempt_at, \
               last_success_at = EXCLUDED.last_success_at, \
               cooldown_until = EXCLUDED.cooldown_until, updated_at = now()",
        )
        .bind(state.account_id)
        .bind(state.upstream_model)
        .bind(state.status.as_str())
        .bind(state.reason.as_deref().unwrap_or(""))
        .bind(state.consecutive_failures)
        .bind(state.last_attempt_at)
        .bind(state.last_success_at)
        .bind(state.cooldown_until)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_model_quota_block(&self, block: ModelQuotaBlock) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO grok_model_quota_blocks \
             (account_id, upstream_model, reason, cooldown_until, updated_at) \
             VALUES ($1,$2,$3,$4,now()) \
             ON CONFLICT (account_id, upstream_model) DO UPDATE SET \
               reason = EXCLUDED.reason, cooldown_until = EXCLUDED.cooldown_until, \
               updated_at = now()",
        )
        .bind(block.account_id)
        .bind(block.upstream_model)
        .bind(block.reason)
        .bind(block.cooldown_until)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_account(&self, account_id: i64) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM grok_accounts WHERE id = $1")
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::deletable_reason;

    #[test]
    fn deletable_reason_prefixes_and_truncates() {
        assert_eq!(
            deletable_reason("grok_build chat endpoint access denied"),
            "deletable: grok_build chat endpoint access denied"
        );
        assert_eq!(deletable_reason("  trimmed  "), "deletable: trimmed");
        let long = "x".repeat(600);
        let got = deletable_reason(&long);
        assert!(got.starts_with("deletable: "), "must keep prefix");
        assert_eq!(got.len(), 512, "truncated to 512");
    }
}