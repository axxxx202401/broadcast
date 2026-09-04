//! 系统凭据库封装：按 UID 分别存取 Token 与密码，禁止将密钥写入普通文件。
//!
// Token/密码读写由后续登录保存计划接入；当前生产路径只构造 Keyring 实现。
#![allow(dead_code)]

use super::AccountError;
use std::collections::HashMap;
use tokio::sync::Mutex;

/// Token 在系统凭据库中的固定 service。
const TOKEN_SERVICE: &str = "im-monitor.token";
/// 密码在系统凭据库中的固定 service。
const PASSWORD_SERVICE: &str = "im-monitor.password";

/// 按 UID 读写 Token 与密码的凭据仓储。
///
/// Token 与密码必须分别存储，实现不得将密钥写入 SQLite、JSON 或其他普通文件。
/// 系统凭据库不可用时返回 [`AccountError::CredentialUnavailable`]，且不得降级为明文文件。
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
#[derive(Default)]
pub struct MemoryCredentialStore {
    tokens: Mutex<HashMap<i64, String>>,
    passwords: Mutex<HashMap<i64, String>>,
}

/// 始终报告系统凭据库不可用的替身。
///
/// 所有读写都返回 [`AccountError::CredentialUnavailable`]，且不会创建或修改任何文件。
/// 错误摘要只描述不可用状态，不回显调用方传入的 Token 或密码。
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

/// 基于平台系统凭据库的生产实现。
///
/// Token 使用 service `im-monitor.token`，密码使用 `im-monitor.password`；
/// username 为 UID 的十进制字符串。同步 [`keyring::Entry`] 调用放在
/// [`tokio::task::spawn_blocking`] 中执行，避免阻塞异步运行时。
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringCredentialStore;

/// Token 与密码在系统凭据库中使用不同的固定 service，避免互相覆盖。
#[derive(Clone, Copy)]
enum SecretKind {
    Token,
    Password,
}

impl SecretKind {
    fn service(self) -> &'static str {
        match self {
            Self::Token => TOKEN_SERVICE,
            Self::Password => PASSWORD_SERVICE,
        }
    }
}

fn require_positive_uid(uid: i64) -> Result<(), AccountError> {
    if uid <= 0 {
        Err(AccountError::InvalidUid(uid))
    } else {
        Ok(())
    }
}

/// 将 keyring 错误映射为不含密钥值的原因摘要。
///
/// 只保留错误类别或属性名；不格式化 `BadEncoding` 的原始字节，也不展开平台错误详情。
fn map_keyring_error(error: keyring::Error) -> AccountError {
    let reason = match error {
        keyring::Error::NoEntry => "凭据条目不存在",
        keyring::Error::PlatformFailure(_) => "平台凭据服务失败",
        keyring::Error::NoStorageAccess(_) => "无法访问系统凭据库",
        keyring::Error::BadEncoding(_) => "凭据编码不是有效 UTF-8",
        keyring::Error::TooLong(name, _) => {
            return AccountError::CredentialUnavailable(format!(
                "凭据属性超过平台长度限制: {name}"
            ));
        }
        keyring::Error::Invalid(attr, _) => {
            return AccountError::CredentialUnavailable(format!("凭据属性无效: {attr}"));
        }
        keyring::Error::Ambiguous(_) => "系统凭据库存在歧义条目",
        _ => "系统凭据库未知错误",
    };
    AccountError::CredentialUnavailable(reason.to_string())
}

fn map_join_error(_error: tokio::task::JoinError) -> AccountError {
    AccountError::CredentialUnavailable("凭据任务被取消".to_string())
}

/// 在阻塞线程中执行同步 keyring 操作；调用方负责解释 `NoEntry`。
async fn run_keyring<T, F>(
    uid: i64,
    service: &'static str,
    op: F,
) -> Result<Result<T, keyring::Error>, AccountError>
where
    T: Send + 'static,
    F: FnOnce(&keyring::Entry) -> Result<T, keyring::Error> + Send + 'static,
{
    require_positive_uid(uid)?;
    let username = uid.to_string();
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(service, &username)?;
        op(&entry)
    })
    .await
    .map_err(map_join_error)
}

async fn get_secret(uid: i64, kind: SecretKind) -> Result<Option<String>, AccountError> {
    match run_keyring(uid, kind.service(), |entry| entry.get_password()).await? {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(map_keyring_error(error)),
    }
}

async fn set_secret(uid: i64, kind: SecretKind, value: &str) -> Result<(), AccountError> {
    let value = value.to_string();
    run_keyring(uid, kind.service(), move |entry| entry.set_password(&value))
        .await?
        .map_err(map_keyring_error)
}

async fn delete_secret(uid: i64, kind: SecretKind) -> Result<(), AccountError> {
    match run_keyring(uid, kind.service(), |entry| entry.delete_credential()).await? {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(map_keyring_error(error)),
    }
}

fn unavailable() -> AccountError {
    AccountError::CredentialUnavailable("系统凭据库不可用".to_string())
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

impl KeyringCredentialStore {
    /// 创建使用系统默认凭据库的仓储。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl CredentialStore for KeyringCredentialStore {
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError> {
        get_secret(uid, SecretKind::Token).await
    }

    async fn set_token(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        set_secret(uid, SecretKind::Token, value).await
    }

    async fn delete_token(&self, uid: i64) -> Result<(), AccountError> {
        delete_secret(uid, SecretKind::Token).await
    }

    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError> {
        get_secret(uid, SecretKind::Password).await
    }

    async fn set_password(&self, uid: i64, value: &str) -> Result<(), AccountError> {
        set_secret(uid, SecretKind::Password, value).await
    }

    async fn delete_password(&self, uid: i64) -> Result<(), AccountError> {
        delete_secret(uid, SecretKind::Password).await
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

    /// 系统凭据库不可用时返回安全摘要错误，且不得降级写出任何文件。
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

    /// keyring 错误只保留类别摘要，不得把 `BadEncoding` 中的原始密钥字节写进错误文本。
    #[test]
    fn map_keyring_error_omits_secret_bytes() {
        let error = super::map_keyring_error(keyring::Error::BadEncoding(b"token-a".to_vec()));
        assert!(matches!(error, AccountError::CredentialUnavailable(_)));
        let text = error.to_string();
        assert!(!text.contains("token-a"));
        assert!(!text.contains("116, 111"));
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
