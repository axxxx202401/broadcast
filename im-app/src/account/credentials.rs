//! 凭据仓储 trait 与测试替身；生产实现见 [`super::sqlite_credentials::SqliteCredentialStore`]。

use super::AccountError;
use std::collections::HashMap;
use tokio::sync::Mutex;

/// 按 UID 读写 Token 与密码的凭据仓储。
///
/// Token 与密码必须分别存储；SQLite 中只保存加密后的 nonce 与 ciphertext，
/// 实现不得将密钥明文写入账号索引 JSON、日志或错误文本。
/// UID 必须为正整数，否则返回 [`AccountError::InvalidUid`]。
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    /// 读取指定 UID 的 Token；条目不存在时返回 `None`。
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError>;
    /// 写入或覆盖指定 UID 的 Token。
    async fn set_token(&self, uid: i64, value: &str) -> Result<(), AccountError>;
    /// 删除指定 UID 的 Token；条目不存在时视为成功。
    async fn delete_token(&self, uid: i64) -> Result<(), AccountError>;
    /// 读取指定 UID 的密码；条目不存在时返回 `None`。
    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError>;
    /// 写入或覆盖指定 UID 的密码。
    async fn set_password(&self, uid: i64, value: &str) -> Result<(), AccountError>;
    /// 删除指定 UID 的密码；条目不存在时视为成功。
    async fn delete_password(&self, uid: i64) -> Result<(), AccountError>;
}

/// 仅存在于进程内存中的凭据替身，用于测试与本地注入。
///
/// 该实现不读写任何文件；Token 与密码使用独立映射保存。
/// 不实现 `Debug`，避免测试日志或断言意外打印密钥值。
#[allow(dead_code)]
#[derive(Default)]
pub struct MemoryCredentialStore {
    tokens: Mutex<HashMap<i64, String>>,
    passwords: Mutex<HashMap<i64, String>>,
}

/// 始终报告凭据存储不可用的替身。
///
/// 所有读写都返回 [`AccountError::CredentialUnavailable`]，且不会创建或修改任何文件。
/// 错误摘要只描述不可用状态，不回显调用方传入的 Token 或密码。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableCredentialStore;

/// 读写走内存、删除 Token/密码始终失败的测试替身。
///
/// 用于验证退出或移除在凭据删除失败后仍更新索引，且错误文本不含密钥。
#[cfg(test)]
#[derive(Default)]
pub(crate) struct FailingDeleteCredentialStore {
    inner: MemoryCredentialStore,
}

#[allow(dead_code)]
fn require_positive_uid(uid: i64) -> Result<(), AccountError> {
    if uid <= 0 {
        Err(AccountError::InvalidUid(uid))
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
fn unavailable() -> AccountError {
    AccountError::CredentialUnavailable("凭据存储不可用".to_string())
}

#[async_trait::async_trait]
impl CredentialStore for MemoryCredentialStore {
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError> {
        require_positive_uid(uid)?;
        Ok(self.tokens.lock().await.get(&uid).cloned())
    }

    async fn set_token(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        self.tokens.lock().await.insert(uid, value.to_string());
        Ok(())
    }

    async fn delete_token(&self, uid: i64) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        self.tokens.lock().await.remove(&uid);
        Ok(())
    }

    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError> {
        require_positive_uid(uid)?;
        Ok(self.passwords.lock().await.get(&uid).cloned())
    }

    async fn set_password(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        self.passwords.lock().await.insert(uid, value.to_string());
        Ok(())
    }

    async fn delete_password(&self, uid: i64) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        self.passwords.lock().await.remove(&uid);
        Ok(())
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl CredentialStore for FailingDeleteCredentialStore {
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError> {
        self.inner.token(uid).await
    }

    async fn set_token(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        self.inner.set_token(uid, value).await
    }

    async fn delete_token(&self, uid: i64) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }

    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError> {
        self.inner.password(uid).await
    }

    async fn set_password(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        self.inner.set_password(uid, value).await
    }

    async fn delete_password(&self, uid: i64) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }
}

#[async_trait::async_trait]
impl CredentialStore for UnavailableCredentialStore {
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }

    async fn set_token(&self, uid: i64, _value: &str) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }

    async fn delete_token(&self, uid: i64) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }

    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }

    async fn set_password(&self, uid: i64, _value: &str) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }

    async fn delete_password(&self, uid: i64) -> Result<(), AccountError> {
        require_positive_uid(uid)?;
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialStore, MemoryCredentialStore, UnavailableCredentialStore};
    use crate::account::AccountError;

    /// Token 与密码按 UID 独立存放，删除其中一类不得影响另一类。
    #[tokio::test]
    async fn credential_store_keeps_token_and_password_separate() {
        let store = MemoryCredentialStore::default();
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

    /// 凭据存储不可用时返回安全摘要错误，且不得降级写出任何文件。
    #[tokio::test]
    async fn unavailable_credential_store_returns_error_without_writing_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = UnavailableCredentialStore;

        let set_token = store.set_token(42, "token-a").await.unwrap_err();
        assert!(matches!(set_token, AccountError::CredentialUnavailable(_)));
        assert!(
            !set_token.to_string().contains("token-a"),
            "错误文本不得包含 Token 值"
        );

        let set_password = store.set_password(42, "secret-a").await.unwrap_err();
        assert!(matches!(
            set_password,
            AccountError::CredentialUnavailable(_)
        ));
        assert!(
            !set_password.to_string().contains("secret-a"),
            "错误文本不得包含密码值"
        );

        assert!(matches!(
            store.token(42).await.unwrap_err(),
            AccountError::CredentialUnavailable(_)
        ));
        assert!(matches!(
            store.password(42).await.unwrap_err(),
            AccountError::CredentialUnavailable(_)
        ));
        assert!(matches!(
            store.delete_token(42).await.unwrap_err(),
            AccountError::CredentialUnavailable(_)
        ));
        assert!(matches!(
            store.delete_password(42).await.unwrap_err(),
            AccountError::CredentialUnavailable(_)
        ));

        assert!(
            std::fs::read_dir(temp.path()).unwrap().next().is_none(),
            "不可用凭据替身不得在探测目录中写入文件"
        );
    }

    /// 零和负 UID 没有合法账号语义，内存替身应拒绝读写。
    #[tokio::test]
    async fn memory_credential_store_rejects_non_positive_uid() {
        let store = MemoryCredentialStore::default();

        assert!(matches!(
            store.set_token(0, "token-a").await.unwrap_err(),
            AccountError::InvalidUid(0)
        ));
        assert!(matches!(
            store.set_password(-1, "secret-a").await.unwrap_err(),
            AccountError::InvalidUid(-1)
        ));
        assert!(matches!(
            store.token(0).await.unwrap_err(),
            AccountError::InvalidUid(0)
        ));
        assert!(matches!(
            store.password(-7).await.unwrap_err(),
            AccountError::InvalidUid(-7)
        ));
        assert!(matches!(
            store.delete_token(0).await.unwrap_err(),
            AccountError::InvalidUid(0)
        ));
        assert!(matches!(
            store.delete_password(-3).await.unwrap_err(),
            AccountError::InvalidUid(-3)
        ));
    }
}
