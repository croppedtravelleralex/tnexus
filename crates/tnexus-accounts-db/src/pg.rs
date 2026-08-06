//! TNexus-owned account pool in Postgres (`migrations/009_tnexus_accounts.sql`).

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct AccountsPg {
    pool: PgPool,
}

impl AccountsPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_account_values(&self) -> Result<Vec<Value>> {
        let rows = sqlx::query(
            "SELECT email, access_token, data FROM tnexus_accounts ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("list tnexus_accounts")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let email: String = row.try_get("email").context("email col")?;
            let token: String = row.try_get("access_token").context("token col")?;
            let data: Value = row.try_get("data").context("data col")?;
            out.push(row_to_value(email, token, data));
        }
        Ok(out)
    }

    pub async fn upsert_account_value(&self, value: &Value) -> Result<()> {
        let token = access_token_from_value(value)?;
        let email = value
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("import-{}", token.chars().take(8).collect::<String>()));

        let mut data = value.clone();
        if let Some(obj) = data.as_object_mut() {
            obj.remove("email");
            obj.remove("access_token");
            obj.remove("accessToken");
        }

        sqlx::query(
            r#"
            INSERT INTO tnexus_accounts (email, access_token, data, updated_at)
            VALUES ($1, $2, $3, now())
            ON CONFLICT (email) DO UPDATE
            SET access_token = EXCLUDED.access_token,
                data = EXCLUDED.data,
                updated_at = now()
            "#,
        )
        .bind(&email)
        .bind(&token)
        .bind(&data)
        .execute(&self.pool)
        .await
        .context("upsert tnexus_accounts")?;
        Ok(())
    }

    pub async fn delete_by_access_token(&self, token: &str) -> Result<bool> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(false);
        }
        let res = sqlx::query("DELETE FROM tnexus_accounts WHERE access_token = $1")
            .bind(token)
            .execute(&self.pool)
            .await
            .context("delete tnexus_accounts")?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn accounts_by_email(&self) -> Result<std::collections::HashMap<String, Value>> {
        let mut map = std::collections::HashMap::new();
        for value in self.list_account_values().await? {
            if let Some(email) = value
                .get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
            {
                map.insert(email, value);
            }
        }
        Ok(map)
    }

    pub async fn touch_inflight(&self, email: &str, delta: i64) -> Result<()> {
        let key = email.trim().to_lowercase();
        if key.is_empty() {
            return Ok(());
        }
        sqlx::query(
            r#"
            UPDATE tnexus_accounts
            SET data = jsonb_set(
                data,
                '{image_inflight}',
                to_jsonb(GREATEST(0, COALESCE((data->>'image_inflight')::bigint, 0) + $2)),
                true
            ),
            updated_at = now()
            WHERE lower(email) = $1
            "#,
        )
        .bind(&key)
        .bind(delta)
        .execute(&self.pool)
        .await
        .context("touch_inflight postgres")?;
        Ok(())
    }

    pub async fn decrement_quota(&self, email: &str, amount: i64) -> Result<Option<(i64, i64)>> {
        let key = email.trim().to_lowercase();
        if key.is_empty() || amount <= 0 {
            return Ok(None);
        }
        let row = sqlx::query(
            r#"
            SELECT email, data FROM tnexus_accounts WHERE lower(email) = $1
            "#,
        )
        .bind(&key)
        .fetch_optional(&self.pool)
        .await
        .context("decrement_quota select")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let data: Value = row.try_get("data").context("data")?;
        if data
            .get("image_quota_unknown")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(None);
        }
        let before = data.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
        let after = (before - amount).max(0);
        sqlx::query(
            r#"
            UPDATE tnexus_accounts
            SET data = jsonb_set(data, '{quota}', to_jsonb($2::bigint), true),
                updated_at = now()
            WHERE lower(email) = $1
            "#,
        )
        .bind(&key)
        .bind(after)
        .execute(&self.pool)
        .await
        .context("decrement_quota update")?;
        Ok(Some((before, after)))
    }

    pub async fn reconcile_inflight_above(&self, ceiling: i64) -> Result<usize> {
        if ceiling <= 0 {
            return Ok(0);
        }
        let res = sqlx::query(
            r#"
            UPDATE tnexus_accounts
            SET data = jsonb_set(data, '{image_inflight}', '0'::jsonb, true),
                updated_at = now()
            WHERE COALESCE((data->>'image_inflight')::bigint, 0) > $1
            "#,
        )
        .bind(ceiling)
        .execute(&self.pool)
        .await
        .context("reconcile_inflight postgres")?;
        Ok(res.rows_affected() as usize)
    }
}

fn access_token_from_value(value: &Value) -> Result<String> {
    let token = value
        .get("access_token")
        .or_else(|| value.get("accessToken"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("account value missing access_token")?;
    Ok(token.to_string())
}

fn row_to_value(email: String, token: String, data: Value) -> Value {
    let mut obj = match data {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    obj.insert("email".into(), json!(email));
    obj.insert("access_token".into(), json!(token));
    Value::Object(obj)
}
