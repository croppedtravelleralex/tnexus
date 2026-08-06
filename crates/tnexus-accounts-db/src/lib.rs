//! Shared gptimage-compatible `accounts.db` access (WAL + transactional writes).

mod backend;
mod pg;
pub mod sync_file;

pub use backend::AccountsBackend;
pub use pg::AccountsPg;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn accounts_db_path() -> Result<PathBuf> {
    std::env::var("ACCOUNTS_DB")
        .map(PathBuf::from)
        .context("ACCOUNTS_DB is required (shared sqlite pool; JSON snapshot removed)")
}

#[derive(Clone)]
pub struct AccountsDb {
    path: PathBuf,
}

impl AccountsDb {
    pub fn from_env() -> Result<Self> {
        Self::open(accounts_db_path()?)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let db = Self { path };
        db.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM accounts", [], |_| Ok(()))
                .context("accounts table missing — is ACCOUNTS_DB pointing at gptimage sqlite?")
        })?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        let conn = Connection::open(&self.path)
            .with_context(|| format!("open accounts db {:?}", self.path))?;
        conn.busy_timeout(Duration::from_secs(30))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=30000;",
        )?;
        f(&conn)
    }

    pub fn list_account_values(&self) -> Result<Vec<Value>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT access_token, data FROM accounts")?;
            let rows = stmt.query_map([], |row| {
                let token: String = row.get(0)?;
                let data: String = row.get(1)?;
                Ok((token, data))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (token, data) = row?;
                out.push(row_to_value(&token, &data)?);
            }
            Ok(out)
        })
    }

    pub fn accounts_by_email(&self) -> Result<std::collections::HashMap<String, Value>> {
        let mut map = std::collections::HashMap::new();
        for value in self.list_account_values()? {
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

    pub fn upsert_account_value(&self, value: &Value) -> Result<()> {
        let token = access_token_from_value(value)?;
        let data_str = normalize_data_json(value, &token)?;
        let email = value
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());

        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let updated = tx.execute(
                "UPDATE accounts SET access_token = ?1, data = ?2 WHERE access_token = ?3",
                params![token, data_str, token],
            )?;
            if updated > 0 {
                tx.commit()?;
                return Ok(());
            }
            if let Some(email) = email.as_deref() {
                if let Some((id, old_token)) = find_id_by_email(&tx, email)? {
                    tx.execute(
                        "UPDATE accounts SET access_token = ?1, data = ?2 WHERE id = ?3",
                        params![token, data_str, id],
                    )?;
                    if old_token != token {
                        tracing::debug!(email, "accounts.db token rotated for email");
                    }
                    tx.commit()?;
                    return Ok(());
                }
            }
            tx.execute(
                "INSERT INTO accounts (access_token, data) VALUES (?1, ?2)",
                params![token, data_str],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn save_all_values(&self, values: &[Value]) -> Result<()> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            for value in values {
                let token = access_token_from_value(value)?;
                let data_str = normalize_data_json(value, &token)?;
                let email = value
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty());
                let updated = tx.execute(
                    "UPDATE accounts SET access_token = ?1, data = ?2 WHERE access_token = ?3",
                    params![token, data_str, token],
                )?;
                if updated > 0 {
                    continue;
                }
                if let Some(email) = email.as_deref() {
                    if let Some((id, _)) = find_id_by_email(&tx, email)? {
                        tx.execute(
                            "UPDATE accounts SET access_token = ?1, data = ?2 WHERE id = ?3",
                            params![token, data_str, id],
                        )?;
                        continue;
                    }
                }
                tx.execute(
                    "INSERT INTO accounts (access_token, data) VALUES (?1, ?2)",
                    params![token, data_str],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn delete_by_access_token(&self, token: &str) -> Result<bool> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(false);
        }
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM accounts WHERE access_token = ?1",
                params![token],
            )?;
            Ok(n > 0)
        })
    }

    /// Reset `image_inflight` to 0 when above `ceiling` (stale inflight leak repair).
    pub fn reconcile_inflight_above(&self, ceiling: i64) -> Result<usize> {
        if ceiling <= 0 {
            return Ok(0);
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut reset = 0usize;
            {
                let mut stmt = tx.prepare("SELECT id, access_token, data FROM accounts")?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?;
                for row in rows {
                    let (id, token, data) = row?;
                    let mut value = row_to_value(&token, &data)?;
                    let cur = value
                        .get("image_inflight")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if cur <= ceiling {
                        continue;
                    }
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("image_inflight".into(), json!(0));
                    }
                    let data_str = normalize_data_json(&value, &token)?;
                    tx.execute(
                        "UPDATE accounts SET data = ?1 WHERE id = ?2",
                        params![data_str, id],
                    )?;
                    reset += 1;
                }
            }
            tx.commit()?;
            Ok(reset)
        })
    }

    pub fn touch_inflight(&self, email: &str, delta: i64) -> Result<()> {
        let key = email.trim().to_lowercase();
        if key.is_empty() {
            return Ok(());
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare("SELECT id, access_token, data FROM accounts")?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?;
                for row in rows {
                    let (id, token, data) = row?;
                    let mut value = row_to_value(&token, &data)?;
                    let em = value
                        .get("email")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if em != key {
                        continue;
                    }
                    let cur = value
                        .get("image_inflight")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let next = (cur + delta).max(0);
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("image_inflight".into(), json!(next));
                    }
                    let data_str = normalize_data_json(&value, &token)?;
                    tx.execute(
                        "UPDATE accounts SET data = ?1 WHERE id = ?2",
                        params![data_str, id],
                    )?;
                    break;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Decrement local `quota` by `amount` (floored at 0). Returns `(before, after)`.
    pub fn decrement_quota(&self, email: &str, amount: i64) -> Result<Option<(i64, i64)>> {
        let key = email.trim().to_lowercase();
        if key.is_empty() || amount <= 0 {
            return Ok(None);
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut out = None;
            {
                let mut stmt = tx.prepare("SELECT id, access_token, data FROM accounts")?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?;
                for row in rows {
                    let (id, token, data) = row?;
                    let mut value = row_to_value(&token, &data)?;
                    let em = value
                        .get("email")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if em != key {
                        continue;
                    }
                    if value
                        .get("image_quota_unknown")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        break;
                    }
                    let before = value.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
                    let after = (before - amount).max(0);
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("quota".into(), json!(after));
                    }
                    let data_str = normalize_data_json(&value, &token)?;
                    tx.execute(
                        "UPDATE accounts SET data = ?1 WHERE id = ?2",
                        params![data_str, id],
                    )?;
                    out = Some((before, after));
                    break;
                }
            }
            tx.commit()?;
            Ok(out)
        })
    }
}

fn access_token_from_value(value: &Value) -> Result<String> {
    let token = value
        .get("access_token")
        .or_else(|| value.get("accessToken"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .context("access_token is required")?;
    Ok(token)
}

fn normalize_data_json(value: &Value, token: &str) -> Result<String> {
    let mut data = value.clone();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("access_token".into(), json!(token));
    }
    Ok(serde_json::to_string(&data)?)
}

fn row_to_value(token: &str, data: &str) -> Result<Value> {
    let mut value: Value = serde_json::from_str(data).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        if !token.trim().is_empty() {
            obj.insert("access_token".into(), json!(token.trim()));
        }
    }
    Ok(value)
}

fn find_id_by_email(conn: &Connection, email: &str) -> Result<Option<(i64, String)>> {
    let key = email.trim().to_lowercase();
    let mut stmt = conn.prepare("SELECT id, access_token, data FROM accounts")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, token, data) = row?;
        let value = row_to_value(&token, &data)?;
        let em = value
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if em == key {
            return Ok(Some((id, token)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn upsert_and_inflight_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tnexus-accounts-db-{}", uuid()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE accounts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    access_token TEXT NOT NULL UNIQUE,
                    data TEXT NOT NULL
                );",
            )
            .unwrap();
        }
        let db = AccountsDb::open(&path).unwrap();
        db.upsert_account_value(&json!({
            "email": "test@example.com",
            "access_token": "tok-a",
            "status": "正常",
            "quota": 5
        }))
        .unwrap();
        db.touch_inflight("test@example.com", 1).unwrap();
        let items = db.list_account_values().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("image_inflight").and_then(|v| v.as_i64()),
            Some(1)
        );
    }

    #[test]
    fn reconcile_inflight_above_resets_stale_counters() {
        let dir = std::env::temp_dir().join(format!("tnexus-accounts-db-{}", uuid()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE accounts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    access_token TEXT NOT NULL UNIQUE,
                    data TEXT NOT NULL
                );",
            )
            .unwrap();
        }
        let db = AccountsDb::open(&path).unwrap();
        db.upsert_account_value(&json!({
            "email": "stale@example.com",
            "access_token": "tok-stale",
            "status": "正常",
            "image_inflight": 99
        }))
        .unwrap();
        let reset = db.reconcile_inflight_above(8).unwrap();
        assert_eq!(reset, 1);
        let items = db.list_account_values().unwrap();
        assert_eq!(
            items[0].get("image_inflight").and_then(|v| v.as_i64()),
            Some(0)
        );
    }

    #[test]
    fn decrement_quota_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tnexus-accounts-db-{}", uuid()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE accounts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    access_token TEXT NOT NULL UNIQUE,
                    data TEXT NOT NULL
                );",
            )
            .unwrap();
        }
        let db = AccountsDb::open(&path).unwrap();
        db.upsert_account_value(&json!({
            "email": "quota@example.com",
            "access_token": "tok-q",
            "status": "正常",
            "quota": 5
        }))
        .unwrap();
        let changed = db.decrement_quota("quota@example.com", 1).unwrap();
        assert_eq!(changed, Some((5, 4)));
        let items = db.list_account_values().unwrap();
        assert_eq!(items[0].get("quota").and_then(|v| v.as_i64()), Some(4));
    }

    fn uuid() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }
}
