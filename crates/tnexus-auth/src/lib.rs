use anyhow::{anyhow, Context, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "member" => Some(Role::Member),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub display_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub role: Role,
    pub exp: usize,
}

#[derive(Clone)]
pub struct AuthService {
    pool: PgPool,
    jwt_secret: String,
    jwt_ttl_secs: u64,
}

impl AuthService {
    pub fn new(pool: PgPool, jwt_secret: String, jwt_ttl_secs: u64) -> Result<Self> {
        if jwt_secret.len() < 32 {
            return Err(anyhow!("JWT_SECRET must be at least 32 characters"));
        }
        Ok(Self {
            pool,
            jwt_secret,
            jwt_ttl_secs,
        })
    }

    pub async fn bootstrap_admin(
        &self,
        account: &str,
        password: &str,
        display_name: &str,
    ) -> Result<()> {
        self.ensure_admin_account(account, password, display_name).await
    }

    /// Create or update the bootstrap admin account from env on every API start.
    pub async fn ensure_admin_account(
        &self,
        account: &str,
        password: &str,
        display_name: &str,
    ) -> Result<()> {
        let account = normalize_account(account);
        validate_account(&account)?;
        if password.len() < 6 {
            return Err(anyhow!("admin password must be at least 6 characters"));
        }
        let hash = hash_password(password)?;
        let existing = sqlx::query(
            "SELECT id FROM users WHERE email = $1",
        )
        .bind(&account)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = existing {
            let id: Uuid = row.get("id");
            sqlx::query(
                "UPDATE users SET password_hash = $2, role = 'admin', display_name = $3, disabled_at = NULL WHERE id = $1",
            )
            .bind(id)
            .bind(&hash)
            .bind(display_name.trim())
            .execute(&self.pool)
            .await?;
            return Ok(());
        }

        sqlx::query(
            r#"INSERT INTO users (email, password_hash, role, display_name)
               VALUES ($1, $2, 'admin', $3)"#,
        )
        .bind(&account)
        .bind(&hash)
        .bind(display_name.trim())
        .execute(&self.pool)
        .await
        .context("create admin")?;
        Ok(())
    }

    /// Create or update a bootstrap demo member account from env on every API start.
    pub async fn ensure_member_account(
        &self,
        account: &str,
        password: &str,
        display_name: &str,
    ) -> Result<()> {
        let account = normalize_account(account);
        validate_account(&account)?;
        if password.len() < 6 {
            return Err(anyhow!("demo password must be at least 6 characters"));
        }
        let hash = hash_password(password)?;
        let existing = sqlx::query("SELECT id, role FROM users WHERE email = $1")
            .bind(&account)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = existing {
            let id: Uuid = row.get("id");
            let role: String = row.get("role");
            if role == "admin" {
                return Ok(());
            }
            sqlx::query(
                "UPDATE users SET password_hash = $2, display_name = $3, disabled_at = NULL WHERE id = $1",
            )
            .bind(id)
            .bind(&hash)
            .bind(display_name.trim())
            .execute(&self.pool)
            .await?;
            return Ok(());
        }

        sqlx::query(
            r#"INSERT INTO users (email, password_hash, role, display_name)
               VALUES ($1, $2, 'member', $3)"#,
        )
        .bind(&account)
        .bind(&hash)
        .bind(display_name.trim())
        .execute(&self.pool)
        .await
        .context("create demo member")?;
        Ok(())
    }

    pub async fn register(
        &self,
        email: &str,
        password: &str,
        display_name: &str,
    ) -> Result<User> {
        if password.len() < 6 {
            return Err(anyhow!("password must be at least 6 characters"));
        }
        self.create_user(email, password, display_name, Role::Member)
            .await
    }

    async fn create_user(
        &self,
        email: &str,
        password: &str,
        display_name: &str,
        role: Role,
    ) -> Result<User> {
        let email = normalize_account(email);
        validate_account(&email)?;
        let hash = hash_password(password)?;
        let row = sqlx::query(
            r#"INSERT INTO users (email, password_hash, role, display_name)
               VALUES ($1, $2, $3, $4)
               RETURNING id, email, role, display_name, created_at, disabled_at"#,
        )
        .bind(&email)
        .bind(&hash)
        .bind(role.as_str())
        .bind(display_name.trim())
        .fetch_one(&self.pool)
        .await
        .context("create user")?;

        Ok(row_to_user(row)?)
    }

    pub async fn login(&self, account: &str, password: &str) -> Result<(User, String)> {
        let account = normalize_account(account);
        let row = sqlx::query(
            "SELECT id, email, password_hash, role, display_name, created_at, disabled_at FROM users WHERE email = $1",
        )
        .bind(&account)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow!("invalid credentials"))?;

        let stored_hash: String = row.get("password_hash");
        verify_password(password, &stored_hash)?;
        let user = row_to_user(row)?;
        if user.disabled {
            return Err(anyhow!("account disabled"));
        }
        let token = self.issue_token(&user)?;
        Ok((user, token))
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, role, display_name, created_at, disabled_at FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_user).transpose()
    }

    pub async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, email, role, display_name, created_at, disabled_at FROM users ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_user).collect()
    }

    pub async fn set_disabled(&self, user_id: Uuid, disabled: bool) -> Result<()> {
        let disabled_at = if disabled {
            Some(chrono::Utc::now())
        } else {
            None
        };
        sqlx::query("UPDATE users SET disabled_at = $2 WHERE id = $1")
            .bind(user_id)
            .bind(disabled_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;
        Ok(data.claims)
    }

    fn issue_token(&self, user: &User) -> Result<String> {
        let exp = (chrono::Utc::now().timestamp() as usize) + self.jwt_ttl_secs as usize;
        let claims = Claims {
            sub: user.id.to_string(),
            email: user.email.clone(),
            role: user.role,
            exp,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .context("issue jwt")
    }
}

fn normalize_account(account: &str) -> String {
    account.trim().to_lowercase()
}

fn validate_account(account: &str) -> Result<()> {
    if account.is_empty() {
        return Err(anyhow!("invalid account"));
    }
    if account.contains('@') {
        if !account.contains('.') {
            return Err(anyhow!("invalid email"));
        }
        return Ok(());
    }
    if account.len() < 3 {
        return Err(anyhow!("username must be at least 3 characters"));
    }
    if !account
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(anyhow!("invalid username"));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("hash password: {e}"))?
        .to_string();
    Ok(hash)
}

fn verify_password(password: &str, stored: &str) -> Result<()> {
    let parsed = PasswordHash::new(stored).map_err(|e| anyhow!("parse hash: {e}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| anyhow!("invalid credentials"))?;
    Ok(())
}

fn row_to_user(row: sqlx::postgres::PgRow) -> Result<User> {
    let role_str: String = row.get("role");
    let role = Role::from_db(&role_str).ok_or_else(|| anyhow!("bad role"))?;
    let disabled_at: Option<chrono::DateTime<chrono::Utc>> = row.get("disabled_at");
    Ok(User {
        id: row.get("id"),
        email: row.get("email"),
        role,
        display_name: row.get("display_name"),
        created_at: row.get("created_at"),
        disabled: disabled_at.is_some(),
    })
}
