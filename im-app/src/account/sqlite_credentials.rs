//! 加密 SQLite 凭据仓储：Token 与密码写入本地库，应用内直接读写，不调用 Keychain。

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use super::credentials::CredentialStore;
use super::paths::AppPaths;
use super::secret_cipher::{decrypt_secret, encrypt_secret, load_or_create_master_key, MASTER_KEY_LEN};
use super::AccountError;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS account_secrets (
  uid INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('token', 'password')),
  nonce BLOB NOT NULL,
  ciphertext BLOB NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (uid, kind)
);
";

/// Token 与密码在 SQLite 中的 kind 列取值。
#[derive(Clone, Copy)]
enum SecretKind {
    Token,
    Password,
}

impl SecretKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Password => "password",
        }
    }
}

/// 基于 AES-256-GCM 加密 SQLite 的生产凭据仓储。
///
/// 主密钥保存在 [`AppPaths::credential_key_file`]，数据库路径为
/// [`AppPaths::credentials_db`]。读写均在本进程内完成，不会触发系统凭据弹窗。
pub struct SqliteCredentialStore {
    pool: SqlitePool,
    master_key: Arc<[u8; MASTER_KEY_LEN]>,
    write_lock: Mutex<()>,
}

impl SqliteCredentialStore {
    /// 打开或初始化凭据库，并加载本机主密钥。
    pub async fn open(paths: &AppPaths) -> Result<Self, AccountError> {
        let db_path = paths.credentials_db();
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let master_key =
            Arc::new(load_or_create_master_key(&paths.credential_key_file()).await?);
        let dsn = format!("sqlite://{}", db_path.display());
        let options = SqliteConnectOptions::from_str(&dsn)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
        let pool = SqlitePool::connect_with(options).await?;
        sqlx::query(SCHEMA_SQL).execute(&pool).await?;
        Ok(Self {
            pool,
            master_key,
            write_lock: Mutex::new(()),
        })
    }

    async fn read_secret(
        &self,
        uid: i64,
        kind: SecretKind,
    ) -> Result<Option<String>, AccountError> {
        require_positive_uid(uid)?;
        let row: Option<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
            "SELECT nonce, ciphertext FROM account_secrets WHERE uid = ? AND kind = ?",
        )
        .bind(uid)
        .bind(kind.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some((nonce, ciphertext)) = row else {
            return Ok(None);
        };
        decrypt_secret(self.master_key.as_ref(), &nonce, &ciphertext).map(Some)
    }

    async fn write_secret(
        &self,
        uid: i64,
        kind: SecretKind,
        value: &str,
    ) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        let _guard = self.write_lock.lock().await;
        let (nonce, ciphertext) = encrypt_secret(self.master_key.as_ref(), value)?;
        let updated_at = now_millis();
        sqlx::query(
            "INSERT INTO account_secrets (uid, kind, nonce, ciphertext, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(uid, kind) DO UPDATE SET
               nonce = excluded.nonce,
               ciphertext = excluded.ciphertext,
               updated_at = excluded.updated_at",
        )
        .bind(uid)
        .bind(kind.as_str())
        .bind(nonce)
        .bind(ciphertext)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_secret(&self, uid: i64, kind: SecretKind) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        let _guard = self.write_lock.lock().await;
        sqlx::query("DELETE FROM account_secrets WHERE uid = ? AND kind = ?")
            .bind(uid)
            .bind(kind.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn require_positive_uid(uid: i64) -> Result<(), AccountError> {
    if uid <= 0 {
        Err(AccountError::InvalidUid(uid))
    } else {
        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[async_trait::async_trait]
impl CredentialStore for SqliteCredentialStore {
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError> {
        self.read_secret(uid, SecretKind::Token).await
    }

    async fn set_token(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        self.write_secret(uid, SecretKind::Token, value).await
    }

    async fn delete_token(&self, uid: i64) -> Result<(), AccountError> {
        self.remove_secret(uid, SecretKind::Token).await
    }

    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError> {
        self.read_secret(uid, SecretKind::Password).await
    }

    async fn set_password(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        self.write_secret(uid, SecretKind::Password, value).await
    }

    async fn delete_password(&self, uid: i64) -> Result<(), AccountError> {
        self.remove_secret(uid, SecretKind::Password).await
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialStore, SqliteCredentialStore};
    use crate::account::AppPaths;

    /// Token 与密码独立存储，删除其中一类不得影响另一类。
    #[tokio::test]
    async fn sqlite_store_keeps_token_and_password_separate() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path());
        let store = SqliteCredentialStore::open(&paths).await.unwrap();
        store.set_token(42, "token-a").await.unwrap();
        store.set_password(42, "secret-a").await.unwrap();
        assert_eq!(store.token(42).await.unwrap().as_deref(), Some("token-a"));
        assert_eq!(
            store.password(42).await.unwrap().as_deref(),
            Some("secret-a")
        );

        store.delete_token(42).await.unwrap();
        assert_eq!(store.token(42).await.unwrap(), None);
        assert_eq!(
            store.password(42).await.unwrap().as_deref(),
            Some("secret-a")
        );
    }

    /// 重新打开凭据库后应能解密已写入的 Token。
    #[tokio::test]
    async fn sqlite_store_persists_encrypted_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path());
        {
            let store = SqliteCredentialStore::open(&paths).await.unwrap();
            store.set_token(99, "persist-token").await.unwrap();
        }
        let store = SqliteCredentialStore::open(&paths).await.unwrap();
        assert_eq!(
            store.token(99).await.unwrap().as_deref(),
            Some("persist-token")
        );
        let raw = tokio::fs::read_to_string(paths.credentials_db()).await;
        assert!(raw.is_err() || !raw.unwrap().contains("persist-token"));
    }
}
