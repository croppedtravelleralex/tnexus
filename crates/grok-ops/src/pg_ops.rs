//! Build 四池探针的真实存储接线（G3-P5）。
//!
//! [`PgBuildProbeOps`] 把 [`crate::four_pool::BuildProbeOps`] 接到 grok-storage 的
//! `PgAccountRepository` 上，让 `BuildFourPool` 的 tick 循环用真实 PG 数据工作。
//!
//! 结构：
//! - [`ProbeRepo`]：适配层对存储的最小读+写接口（grok-ops 本地 trait，允许测试注入
//!   内存 fake；`PgAccountRepository` 实现它，写路径委托 `AccountOps`）。
//! - [`BuildProbeTransport`]：对上游 Build Responses 的一次探测（HTTP 状态 + body），
//!   由接线方注入真实 HTTP 客户端；默认 [`NotWiredTransport`] 返回 `transport not wired`。
//! - 纯函数 [`classify_credential_probe`] / [`capability_probe_error`] /
//!   [`observed_model_from_body`]：对齐 Go `probeBuildChatCredential` /
//!   `probeBuildChatCapabilityOnly` 的判定，单测直接覆盖。
//!
//! 边界：`prepare_credential`（令牌刷新）与 `refresh_billing`（上游 Billing 拉取）依赖
//! Go 侧 sidecar（浏览器签名 / billing API），adapter 内不做——见各自文档。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use grok_domain::{Account, Billing, ModelState, ModelStatus, QuotaRecovery};
use grok_storage::repo::account::{AccountRepository, PgAccountRepository};
use grok_storage::repo::accounts_ops::AccountOps;
use grok_storage::StorageError;

use crate::error::OpsResult;
use crate::four_pool::{BuildProbeOps, PROBE_COOLDOWN};

// ── 存储接口（trait 分界，测试注入 fake）────────────────────────────

/// 适配层对存储的最小接口：Build 账号的读 + 探针副作用写。
#[async_trait]
pub trait ProbeRepo: Send + Sync {
    async fn get_account(&self, id: i64) -> OpsResult<Option<Account>>;
    async fn list_build_accounts(&self, now: DateTime<Utc>) -> OpsResult<Vec<Account>>;
    async fn recoveries_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>>;
    async fn billings_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, Billing>>;
    async fn get_recovery(&self, id: i64) -> OpsResult<Option<QuotaRecovery>>;
    async fn get_billing(&self, id: i64) -> OpsResult<Option<Billing>>;

    /// 记录账号观察到的最新上游模型（对齐 Go `ObserveResponseModel`）。
    async fn observe_model(&self, id: i64, model: &str) -> OpsResult<()>;
    async fn save_model_state(&self, state: ModelState) -> OpsResult<()>;
    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        reason: &str,
        reset_last_success: bool,
    ) -> OpsResult<()>;
    /// 标记账号可删（对齐 Go `markBuildDeletable`）。
    async fn mark_deletable(&self, id: i64, reason: &str) -> OpsResult<()>;
    async fn clear_recovery(&self, id: i64) -> OpsResult<()>;
    async fn delete_account(&self, id: i64) -> OpsResult<()>;
}

/// `PgAccountRepository` 实现 `ProbeRepo`（写路径委托 `AccountOps`，读路径用 repo 方法）。
#[async_trait]
impl ProbeRepo for PgAccountRepository {
    async fn get_account(&self, id: i64) -> OpsResult<Option<Account>> {
        match AccountRepository::get(self, id).await {
            Ok(account) => Ok(Some(account)),
            Err(StorageError::NotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn list_build_accounts(&self, _now: DateTime<Utc>) -> OpsResult<Vec<Account>> {
        PgAccountRepository::list_build_accounts(self)
            .await
            .map_err(Into::into)
    }

    async fn recoveries_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>> {
        PgAccountRepository::recoveries(self, ids)
            .await
            .map_err(Into::into)
    }

    async fn billings_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, Billing>> {
        PgAccountRepository::billings(self, ids)
            .await
            .map_err(Into::into)
    }

    async fn get_recovery(&self, id: i64) -> OpsResult<Option<QuotaRecovery>> {
        PgAccountRepository::recovery(self, id)
            .await
            .map_err(Into::into)
    }

    async fn get_billing(&self, id: i64) -> OpsResult<Option<Billing>> {
        PgAccountRepository::billing(self, id)
            .await
            .map_err(Into::into)
    }

    async fn observe_model(&self, id: i64, model: &str) -> OpsResult<()> {
        AccountOps::observe_model(self, id, model)
            .await
            .map_err(Into::into)
    }

    async fn save_model_state(&self, state: ModelState) -> OpsResult<()> {
        AccountOps::save_model_state(self, state)
            .await
            .map_err(Into::into)
    }

    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        reason: &str,
        reset_last_success: bool,
    ) -> OpsResult<()> {
        AccountOps::update_health(
            self,
            id,
            failure_count,
            cooldown_until,
            reason,
            reset_last_success,
        )
        .await
        .map_err(Into::into)
    }

    async fn mark_deletable(&self, id: i64, reason: &str) -> OpsResult<()> {
        AccountOps::mark_deletable(self, id, reason)
            .await
            .map_err(Into::into)
    }

    async fn clear_recovery(&self, id: i64) -> OpsResult<()> {
        AccountOps::clear_quota_recovery(self, id)
            .await
            .map_err(Into::into)
    }

    async fn delete_account(&self, id: i64) -> OpsResult<()> {
        AccountOps::delete_account(self, id)
            .await
            .map_err(Into::into)
    }
}

// ── 上游探测接口 ───────────────────────────────────────────────────

/// 对上游 Build Responses 的一次探测。
///
/// 返回 `(http_status, body)`，供 adapter 复刻 Go 的状态分支
/// （2xx / 401 / 403-permission-denied / 其它）；`Err` 为传输层失败
/// （连接错误等，视为普通失败进入冷却路径）。
///
/// ponytail: 任务书给的签名是 `Result<String,String>`，但 Go 分支需要 HTTP 状态码区分
/// 401 vs 403，故返回 (status, body)。接线方（真实 HTTP client）按此签名实现。
#[async_trait]
pub trait BuildProbeTransport: Send + Sync {
    async fn probe_chat(&self, account: &Account) -> Result<(i32, String), String>;
}

/// 默认未接线 transport：探针调用直接失败，防止未接线时静默通过。
#[derive(Debug, Default, Clone)]
pub struct NotWiredTransport;

#[async_trait]
impl BuildProbeTransport for NotWiredTransport {
    async fn probe_chat(&self, _account: &Account) -> Result<(i32, String), String> {
        Err("transport not wired".to_string())
    }
}

// ── 纯判定函数（对齐 Go probeBuildChatCredential / probeBuildChatCapabilityOnly）────

/// 验证轨探测动作（对齐 Go `probeBuildChatCredential` 的状态分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialProbeAction {
    /// 2xx：携带观察到的模型名。
    Verified(String),
    /// 终端失败：标记可删（reason 传给 `mark_deletable`，message 对外返回）。
    Deletable {
        reason: &'static str,
        message: &'static str,
    },
    /// 其它失败：进入冷却（携带 HTTP 状态码）。
    Cooldown(i32),
}

/// 判定验证轨动作（纯函数，单测覆盖）。
pub fn classify_credential_probe(status: i32, body: &str) -> CredentialProbeAction {
    if (200..300).contains(&status) {
        return CredentialProbeAction::Verified(observed_model_from_body(body));
    }
    let meta = body.to_ascii_lowercase();
    if status == 401 {
        return CredentialProbeAction::Deletable {
            reason: "grok_build credential rejected",
            message: "Build Chat 能力探测认证失败",
        };
    }
    if status == 403
        && (meta.contains("permission-denied")
            || meta.contains("permission_denied")
            || meta.contains("access to the chat endpoint is denied"))
    {
        return CredentialProbeAction::Deletable {
            reason: "grok_build chat endpoint access denied",
            message: "Build Chat 权限不足",
        };
    }
    CredentialProbeAction::Cooldown(status)
}

/// 从响应 body 提取观察到的模型名；缺失时回退 `grok-4.5`（对齐 Go）。
pub fn observed_model_from_body(body: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Envelope {
        model: Option<String>,
    }
    serde_json::from_str::<Envelope>(body)
        .ok()
        .and_then(|e| e.model)
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "grok-4.5".to_string())
}

/// capability-only 探测的错误文本（对齐 Go `probeBuildChatCapabilityOnly`）：
/// 2xx → `None`；否则 `Some("status {code}: {snippet}")`，snippet ≤200 字符。
pub fn capability_probe_error(status: i32, body: &str) -> Option<String> {
    if (200..300).contains(&status) {
        return None;
    }
    let mut snippet = body.trim().to_string();
    snippet.truncate(200);
    Some(format!("status {status}: {snippet}"))
}

// ── adapter ───────────────────────────────────────────────────────

/// 把 `BuildProbeOps` 接到真实 PG 存储 + 上游 transport 上的实现。
pub struct PgBuildProbeOps {
    repo: Arc<dyn ProbeRepo>,
    transport: Arc<dyn BuildProbeTransport>,
}

impl PgBuildProbeOps {
    /// 未接 transport 的构造（探针会失败并返回 `transport not wired`）。
    pub fn new(repo: Arc<dyn ProbeRepo>) -> Self {
        Self::with_transport(repo, Arc::new(NotWiredTransport))
    }

    /// 注入上游探测 transport。
    pub fn with_transport(
        repo: Arc<dyn ProbeRepo>,
        transport: Arc<dyn BuildProbeTransport>,
    ) -> Self {
        Self { repo, transport }
    }
}

#[async_trait]
impl BuildProbeOps for PgBuildProbeOps {
    async fn get_account(&self, id: i64) -> OpsResult<Option<Account>> {
        self.repo.get_account(id).await
    }

    async fn list_build_accounts(&self, now: DateTime<Utc>) -> OpsResult<Vec<Account>> {
        self.repo.list_build_accounts(now).await
    }

    async fn recoveries_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>> {
        self.repo.recoveries_for(ids).await
    }

    async fn billings_for(&self, ids: &[i64]) -> OpsResult<HashMap<i64, Billing>> {
        self.repo.billings_for(ids).await
    }

    async fn get_recovery(&self, id: i64) -> OpsResult<Option<QuotaRecovery>> {
        self.repo.get_recovery(id).await
    }

    async fn get_billing(&self, id: i64) -> OpsResult<Option<Billing>> {
        self.repo.get_billing(id).await
    }

    async fn prepare_credential(
        &self,
        account: &Account,
        _refresh_tokens: bool,
    ) -> OpsResult<Account> {
        // Go `prepareBuildProbeCredential` 在令牌过期时经 sidecar 浏览器刷新；
        // Rust 侧该链路未接线，直接放行（过期判定与刷新由接线方负责）。
        Ok(account.clone())
    }

    async fn refresh_billing(&self, _id: i64) -> OpsResult<()> {
        // Go `refreshBuildProbeBilling` 从上游 Billing API 拉取快照；
        // Rust 侧未接线，no-op（额度新鲜度由 selector 的 BILLING_FRESH_TTL 兜底）。
        Ok(())
    }

    async fn probe_chat_credential(&self, account: &Account) -> Result<String, String> {
        let (status, body) = self.transport.probe_chat(account).await?;
        match classify_credential_probe(status, &body) {
            CredentialProbeAction::Verified(model) => {
                // 对齐 Go 2xx 分支：ObserveResponseModel + UpdateHealth(0)。
                let _ = self.repo.observe_model(account.id, &model).await;
                let _ = self.repo.update_health(account.id, 0, None, "", true).await;
                Ok(model)
            }
            CredentialProbeAction::Deletable { reason, message } => {
                let _ = self.repo.mark_deletable(account.id, reason).await;
                Err(message.to_string())
            }
            CredentialProbeAction::Cooldown(status) => {
                // 对齐 Go `cooldownBuildProbe`：fail+1、now+15min 冷却。
                let until = Utc::now() + PROBE_COOLDOWN;
                let msg = format!("build chat capability probe status {status}");
                let _ = self
                    .repo
                    .update_health(
                        account.id,
                        account.failure_count + 1,
                        Some(until),
                        &msg,
                        false,
                    )
                    .await;
                Err(format!("Build Chat 能力探测返回 {status}"))
            }
        }
    }

    async fn probe_chat_capability(&self, account: &Account) -> Result<String, String> {
        let (status, body) = self.transport.probe_chat(account).await?;
        if let Some(err) = capability_probe_error(status, &body) {
            return Err(err);
        }
        Ok(observed_model_from_body(&body))
    }

    async fn observe_model(&self, id: i64, model: &str) -> OpsResult<()> {
        // 对齐 Go `ObserveResponseModel`：更新账号 observed_model + 落库模型状态。
        let now = Utc::now();
        self.repo.observe_model(id, model).await?;
        self.repo
            .save_model_state(ModelState {
                account_id: id,
                upstream_model: model.to_string(),
                status: ModelStatus::Available,
                reason: Some("image_generated".into()),
                consecutive_failures: 0,
                last_attempt_at: Some(now),
                last_success_at: Some(now),
                cooldown_until: None,
                updated_at: now,
            })
            .await
    }

    async fn update_health(
        &self,
        id: i64,
        failure_count: i32,
        cooldown_until: Option<DateTime<Utc>>,
        reason: &str,
        reset_last_success: bool,
    ) -> OpsResult<()> {
        self.repo
            .update_health(
                id,
                failure_count,
                cooldown_until,
                reason,
                reset_last_success,
            )
            .await
    }

    async fn mark_deletable(&self, id: i64, reason: &str) -> OpsResult<()> {
        self.repo.mark_deletable(id, reason).await
    }

    async fn clear_recovery(&self, id: i64) -> OpsResult<()> {
        self.repo.clear_recovery(id).await
    }

    async fn delete_account(&self, id: i64) -> OpsResult<()> {
        self.repo.delete_account(id).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use chrono::Utc;

    use super::*;

    fn account(id: i64) -> Account {
        Account {
            id,
            identity_key: format!("acc-{id}"),
            provider: grok_domain::Provider::GrokBuild,
            enabled: true,
            failure_count: 2,
            ..Default::default()
        }
    }

    // ── 纯判定 ─────────────────────────────────────────────────────

    #[test]
    fn classify_credential_probe_matches_go_branches() {
        // 2xx → verified + 提取模型
        let action = classify_credential_probe(200, r#"{"model":"grok-4.5-build-free"}"#);
        assert_eq!(
            action,
            CredentialProbeAction::Verified("grok-4.5-build-free".into())
        );
        // 2xx 空 body → 回退 grok-4.5
        let action = classify_credential_probe(200, "");
        assert_eq!(action, CredentialProbeAction::Verified("grok-4.5".into()));
        // 401 → deletable（认证失败）
        let action = classify_credential_probe(401, r#"{"error":{"code":"unauthorized"}}"#);
        assert_eq!(
            action,
            CredentialProbeAction::Deletable {
                reason: "grok_build credential rejected",
                message: "Build Chat 能力探测认证失败",
            }
        );
        // 403 + permission-denied → deletable（权限不足）
        let action = classify_credential_probe(
            403,
            r#"{"error":{"code":"permission-denied","message":"Access to the chat endpoint is denied"}}"#,
        );
        assert_eq!(
            action,
            CredentialProbeAction::Deletable {
                reason: "grok_build chat endpoint access denied",
                message: "Build Chat 权限不足",
            }
        );
        // 403 + permission_denied（下划线变体）
        let action = classify_credential_probe(403, r#"{"code":"permission_denied"}"#);
        assert!(matches!(action, CredentialProbeAction::Deletable { .. }));
        // 403 其它 body → 冷却
        let action = classify_credential_probe(403, "rate limited");
        assert_eq!(action, CredentialProbeAction::Cooldown(403));
        // 429 → 冷却
        let action = classify_credential_probe(429, "slow down");
        assert_eq!(action, CredentialProbeAction::Cooldown(429));
    }

    #[test]
    fn observed_model_parses_envelope_with_default() {
        assert_eq!(
            observed_model_from_body(r#"{"model":"grok-4.5-build-free"}"#),
            "grok-4.5-build-free"
        );
        assert_eq!(
            observed_model_from_body(r#"{"model": "  grok-3  "}"#),
            "grok-3"
        );
        assert_eq!(observed_model_from_body(""), "grok-4.5");
        assert_eq!(observed_model_from_body("not json"), "grok-4.5");
    }

    #[test]
    fn capability_error_formats_status_snippet() {
        assert_eq!(capability_probe_error(200, "ok"), None);
        assert_eq!(
            capability_probe_error(403, "denied"),
            Some("status 403: denied".into())
        );
        let long = "x".repeat(300);
        let got = capability_probe_error(500, &long).unwrap();
        assert_eq!(
            got.len(),
            "status 500: ".len() + 200,
            "snippet truncated to 200"
        );
    }

    // ── adapter 端到端（fake repo + fake transport，副作用断言）────────

    /// 内存 fake：记录副作用调用，便于断言 Go 分支对应的写路径。
    #[derive(Default)]
    #[allow(clippy::type_complexity)]
    struct FakeRepo {
        accounts: Mutex<HashMap<i64, Account>>,
        mark_deletable_calls: Mutex<Vec<(i64, String)>>,
        observe_model_calls: Mutex<Vec<(i64, String)>>,
        health_updates: Mutex<Vec<(i64, i32, Option<DateTime<Utc>>)>>,
    }

    #[async_trait]
    impl ProbeRepo for FakeRepo {
        async fn get_account(&self, id: i64) -> OpsResult<Option<Account>> {
            Ok(self.accounts.lock().unwrap().get(&id).cloned())
        }
        async fn list_build_accounts(&self, _now: DateTime<Utc>) -> OpsResult<Vec<Account>> {
            Ok(self.accounts.lock().unwrap().values().cloned().collect())
        }
        async fn recoveries_for(&self, _ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>> {
            Ok(HashMap::new())
        }
        async fn billings_for(&self, _ids: &[i64]) -> OpsResult<HashMap<i64, Billing>> {
            Ok(HashMap::new())
        }
        async fn get_recovery(&self, _id: i64) -> OpsResult<Option<QuotaRecovery>> {
            Ok(None)
        }
        async fn get_billing(&self, _id: i64) -> OpsResult<Option<Billing>> {
            Ok(None)
        }
        async fn observe_model(&self, id: i64, model: &str) -> OpsResult<()> {
            self.observe_model_calls
                .lock()
                .unwrap()
                .push((id, model.to_string()));
            if let Some(a) = self.accounts.lock().unwrap().get_mut(&id) {
                a.observed_model = Some(model.to_string());
            }
            Ok(())
        }
        async fn save_model_state(&self, _state: ModelState) -> OpsResult<()> {
            Ok(())
        }
        async fn update_health(
            &self,
            id: i64,
            failure_count: i32,
            cooldown_until: Option<DateTime<Utc>>,
            _reason: &str,
            _reset_last_success: bool,
        ) -> OpsResult<()> {
            self.health_updates
                .lock()
                .unwrap()
                .push((id, failure_count, cooldown_until));
            Ok(())
        }
        async fn mark_deletable(&self, id: i64, reason: &str) -> OpsResult<()> {
            self.mark_deletable_calls
                .lock()
                .unwrap()
                .push((id, reason.to_string()));
            Ok(())
        }
        async fn clear_recovery(&self, _id: i64) -> OpsResult<()> {
            Ok(())
        }
        async fn delete_account(&self, _id: i64) -> OpsResult<()> {
            Ok(())
        }
    }

    struct FakeTransport {
        status: i32,
        body: String,
    }

    #[async_trait]
    impl BuildProbeTransport for FakeTransport {
        async fn probe_chat(&self, _account: &Account) -> Result<(i32, String), String> {
            Ok((self.status, self.body.clone()))
        }
    }

    fn repo_with(account: Account) -> Arc<FakeRepo> {
        let repo = Arc::new(FakeRepo::default());
        repo.accounts.lock().unwrap().insert(account.id, account);
        repo
    }

    fn adapter_with(
        repo: Arc<FakeRepo>,
        status: i32,
        body: &str,
    ) -> (PgBuildProbeOps, Arc<FakeRepo>) {
        let ops = PgBuildProbeOps::with_transport(
            repo.clone() as Arc<dyn ProbeRepo>,
            Arc::new(FakeTransport {
                status,
                body: body.to_string(),
            }),
        );
        (ops, repo)
    }

    #[tokio::test]
    async fn probe_chat_credential_2xx_observes_model_and_clears_health() {
        let (ops, repo) = adapter_with(
            repo_with(account(1)),
            200,
            r#"{"model":"grok-4.5-build-free"}"#,
        );
        let result = ops.probe_chat_credential(&account(1)).await;
        assert_eq!(result.unwrap(), "grok-4.5-build-free");
        assert_eq!(
            *repo.observe_model_calls.lock().unwrap(),
            vec![(1, "grok-4.5-build-free".to_string())]
        );
        assert_eq!(
            *repo.health_updates.lock().unwrap(),
            vec![(1, 0, None)],
            "health reset on success"
        );
        assert!(repo.mark_deletable_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn probe_chat_credential_401_marks_deletable() {
        let (ops, repo) = adapter_with(
            repo_with(account(1)),
            401,
            r#"{"error":{"code":"unauthorized"}}"#,
        );
        let err = ops.probe_chat_credential(&account(1)).await.unwrap_err();
        assert_eq!(err, "Build Chat 能力探测认证失败");
        assert_eq!(
            *repo.mark_deletable_calls.lock().unwrap(),
            vec![(1, "grok_build credential rejected".to_string())]
        );
    }

    #[tokio::test]
    async fn probe_chat_credential_403_permission_marks_deletable() {
        let (ops, repo) = adapter_with(
            repo_with(account(1)),
            403,
            r#"{"error":{"code":"permission-denied","message":"Access to the chat endpoint is denied"}}"#,
        );
        let err = ops.probe_chat_credential(&account(1)).await.unwrap_err();
        assert_eq!(err, "Build Chat 权限不足");
        assert_eq!(
            *repo.mark_deletable_calls.lock().unwrap(),
            vec![(1, "grok_build chat endpoint access denied".to_string())]
        );
    }

    #[tokio::test]
    async fn probe_chat_credential_other_status_cooldowns() {
        let (ops, repo) = adapter_with(repo_with(account(1)), 429, "slow down");
        let err = ops.probe_chat_credential(&account(1)).await.unwrap_err();
        assert_eq!(err, "Build Chat 能力探测返回 429");
        let updates = repo.health_updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, 1);
        assert_eq!(updates[0].1, 3, "failure_count 2 + 1");
        assert!(updates[0].2.is_some(), "cooldown set to now+15min");
        assert!(repo.mark_deletable_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn probe_chat_capability_ok_and_err() {
        let (ops, repo) = adapter_with(repo_with(account(1)), 200, r#"{"model":"grok-3"}"#);
        assert_eq!(
            ops.probe_chat_capability(&account(1)).await.unwrap(),
            "grok-3"
        );
        assert!(repo.mark_deletable_calls.lock().unwrap().is_empty());

        let (ops, _) = adapter_with(repo_with(account(1)), 403, "denied");
        assert_eq!(
            ops.probe_chat_capability(&account(1)).await.unwrap_err(),
            "status 403: denied"
        );
    }

    #[tokio::test]
    async fn not_wired_transport_fails_probe() {
        let repo = Arc::new(FakeRepo::default());
        let ops = PgBuildProbeOps::new(repo as Arc<dyn ProbeRepo>);
        let err = ops.probe_chat_credential(&account(1)).await.unwrap_err();
        assert_eq!(err, "transport not wired");
    }

    #[tokio::test]
    async fn get_account_maps_reader() {
        let a = account(7);
        let repo = repo_with(a.clone());
        let ops = PgBuildProbeOps::new(repo.clone() as Arc<dyn ProbeRepo>);
        assert_eq!(ops.get_account(7).await.unwrap().unwrap().id, 7);
        assert!(ops.get_account(99).await.unwrap().is_none());
        assert_eq!(ops.list_build_accounts(Utc::now()).await.unwrap().len(), 1);
    }
}
