//! 凭据只读仓储（grok_credentials）。
//!
//! G0 只提供 SELECT 密文原样通道；解密逻辑不入本 crate（凭据加密密钥在
//! ETL / 启动期单独治理，docs/39b §1）。
//!
//! 列名对齐 Go `account_credentials` 的
//! `encrypted_primary` / `encrypted_refresh`（迁移 010 骨架扩展为 Go parity 后
//! 即为最终列名，见 39e / 39b §3 表 4）。

use async_trait::async_trait;
use grok_domain::{ProviderError, SsoTokenProvider};
use sqlx::{PgPool, Row};

use crate::StorageError;

/// 凭据只读 repository（G0）。
#[async_trait]
pub trait CredentialRepository {
    /// 返回账号访问令牌密文（原样字节，AES-GCM 密文，不解密）。
    async fn get(&self, account_id: i64) -> Result<Vec<u8>, StorageError>;

    /// 存在性 + refresh 到期判断（只读，供刷新调度判断）。
    /// 返回 (access_ciphertext, Option<refresh_ciphertext>, Option<refresh_due_at>)。
    async fn refresh_due(
        &self,
        account_id: i64,
    ) -> Result<
        Option<(
            Vec<u8>,
            Option<Vec<u8>>,
            Option<chrono::DateTime<chrono::Utc>>,
        )>,
        StorageError,
    >;
}

/// PG 凭据只读实现。
pub struct PgCredentialRepository {
    pool: PgPool,
}

impl PgCredentialRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CredentialRepository for PgCredentialRepository {
    async fn get(&self, account_id: i64) -> Result<Vec<u8>, StorageError> {
        let row =
            sqlx::query("SELECT encrypted_primary FROM grok_credentials WHERE account_id = $1")
                .bind(account_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Err(StorageError::NotFound(format!("credential {account_id}")));
        };
        row.try_get::<Vec<u8>, _>("encrypted_primary")
            .map_err(StorageError::from)
    }

    async fn refresh_due(
        &self,
        account_id: i64,
    ) -> Result<
        Option<(
            Vec<u8>,
            Option<Vec<u8>>,
            Option<chrono::DateTime<chrono::Utc>>,
        )>,
        StorageError,
    > {
        let row = sqlx::query(
            "SELECT encrypted_primary, encrypted_refresh, refresh_due_at \
             FROM grok_credentials WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let primary: Vec<u8> = row.try_get("encrypted_primary")?;
        let refresh: Option<Vec<u8>> = row.try_get("encrypted_refresh")?;
        let due: Option<chrono::DateTime<chrono::Utc>> = row.try_get("refresh_due_at")?;
        Ok(Some((primary, refresh, due)))
    }
}

// ── AES-256-GCM 凭据解密（无 chrome 直连路径用）────────────────────

/// 用 32 字节主密钥解密 `encrypted_primary`（Go parity，对齐 ETL decrypt smoke 语义）：
/// nonce12 为密文头 12 字节，其后紧跟 GCM ciphertext；明文字符串。
/// `encrypted_primary` 列存的是 base64( nonce||ciphertext )（RAW_STD 编码）。
pub fn decrypt_primary(b64_or_raw: &[u8], key: &[u8; 32]) -> Result<String, StorageError> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    use base64::Engine;
    // 兼容两种存储形态：密文本身是 base64 文本，或已经是原始字节。
    let raw = if is_std_base64(b64_or_raw) {
        base64::engine::general_purpose::STANDARD
            .decode(b64_or_raw)
            .map_err(|e| StorageError::Decrypt(format!("base64 decode: {e}")))?
    } else {
        b64_or_raw.to_vec()
    };
    if raw.len() < 12 {
        return Err(StorageError::Decrypt("ciphertext too short".into()));
    }
    let (nonce, ct) = raw.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| StorageError::Decrypt("key".into()))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| StorageError::Decrypt("AES-GCM 解密失败".into()))?;
    String::from_utf8(plain).map_err(|_| StorageError::Decrypt("非 UTF-8 明文".into()))
}

/// 判别字节是否已是 base64 文本（无控制字节且长度能被 4 整除）。
fn is_std_base64(data: &[u8]) -> bool {
    if data.is_empty() || !data.len().is_multiple_of(4) {
        return false;
    }
    data.iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/' || *b == b'=')
}

/// 从 base64 32B 解析 AES-GCM 主密钥（`GROK_CREDENTIAL_KEY`）。
pub fn parse_credential_key(base64_encoded: &str) -> Result<[u8; 32], StorageError> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(base64_encoded.trim())
        .map_err(|e| StorageError::Decrypt(format!("key base64 decode: {e}")))?;
    raw.try_into()
        .map_err(|_| StorageError::Decrypt("key must be 32 bytes".into()))
}

/// PG 账号 → 解密 sso token 提供者（无 chrome 直连路径）。
pub struct PgSsoTokenProvider {
    repo: PgCredentialRepository,
    key: [u8; 32],
}

impl PgSsoTokenProvider {
    pub fn new(pool: PgPool, key: [u8; 32]) -> Self {
        Self {
            repo: PgCredentialRepository::new(pool),
            key,
        }
    }
}

#[async_trait]
impl SsoTokenProvider for PgSsoTokenProvider {
    async fn sso_token(&self, account_id: i64) -> Result<String, ProviderError> {
        let ciphertext = self
            .repo
            .get(account_id)
            .await
            .map_err(|e| ProviderError::Upstream(format!("credential: {e}")))?;
        decrypt_primary(&ciphertext, &self.key).map_err(|e| ProviderError::Upstream(e.to_string()))
    }
}
