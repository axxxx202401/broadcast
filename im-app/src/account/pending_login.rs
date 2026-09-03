//! 短期登录秘密缓存：仅驻留进程内存，保存尚未完成二次验证的登录密码与账号上下文。
//!
//! 该缓存不得将密码或 token 写入磁盘、JSON 或日志。每条记录自创建起最多保留 10 分钟。
//! 读写方法由后续验证命令接入；当前生产路径只把缓存实例放入 [`AppState`](crate::state::AppState)。
#![allow(dead_code)]

use super::AccountError;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// 待完成登录的内存上下文。
///
/// `password` 使用 [`zeroize::Zeroizing`] 在丢弃时清零；验证码登录可将 `password` 设为 `None`。
/// 该结构不得序列化或写入任何持久化介质。
#[derive(Clone)]
pub struct PendingLogin {
    /// 用户输入的完整显示账号，后续 challenge 不得用脱敏账号覆盖。
    pub display_account: String,
    /// 首次主登录使用的登录方式标识。
    pub primary_login_type: i32,
    /// 可选的原始登录密码；仅密码登录会填充。
    pub password: Option<zeroize::Zeroizing<String>>,
    /// 密码是否已被 [`PendingLoginCache::reuse_password_once`] 消费过。
    pub password_reused: bool,
}

/// 带创建时间的缓存条目，供 10 分钟过期清理使用。
struct TimedPendingLogin {
    login: PendingLogin,
    created_at: Instant,
}

/// 待登录记录自写入起允许存活的最长时间。
const PENDING_LOGIN_TTL: Duration = Duration::from_secs(10 * 60);

/// 仅驻留进程内存的待登录秘密缓存。
///
/// 所有读写先淘汰超过 [`PENDING_LOGIN_TTL`] 的条目。日志只记录条目数量，不得记录 token 或密码。
#[derive(Default)]
pub struct PendingLoginCache {
    entries: Mutex<HashMap<String, TimedPendingLogin>>,
}

impl PendingLoginCache {
    /// 删除创建时间距 `now` 已满 10 分钟的条目。
    fn evict_expired(entries: &mut HashMap<String, TimedPendingLogin>, now: Instant) {
        entries.retain(|_, timed| {
            now.checked_duration_since(timed.created_at)
                .is_some_and(|age| age < PENDING_LOGIN_TTL)
        });
    }

    /// 取得互斥锁、淘汰过期条目，并返回仍有效的映射。
    async fn lock_live(&self) -> tokio::sync::MutexGuard<'_, HashMap<String, TimedPendingLogin>> {
        let mut entries = self.entries.lock().await;
        Self::evict_expired(&mut entries, Instant::now());
        entries
    }

    /// 写入或覆盖指定 token 对应的待登录上下文。
    pub async fn insert(&self, token: &str, login: PendingLogin) {
        let mut entries = self.lock_live().await;
        entries.insert(
            token.to_string(),
            TimedPendingLogin {
                login,
                created_at: Instant::now(),
            },
        );
        tracing::debug!(entry_count = entries.len(), "pending login cache inserted");
    }

    /// 在测试中按指定创建时间写入条目，用于验证过期清理而不削弱生产 TTL。
    #[cfg(test)]
    pub async fn insert_at(&self, token: &str, login: PendingLogin, created_at: Instant) {
        let mut entries = self.lock_live().await;
        entries.insert(
            token.to_string(),
            TimedPendingLogin { login, created_at },
        );
    }

    /// 查看指定 token 是否仍有未过期条目；不移除记录，也不克隆密码以外的秘密用途。
    pub async fn get(&self, token: &str) -> Option<PendingLogin> {
        let entries = self.lock_live().await;
        entries.get(token).map(|timed| timed.login.clone())
    }

    /// 将已有条目从旧 token 迁移到新 token，保留原创建时间与复用标志。
    ///
    /// 旧 token 不存在或已过期时返回 [`AccountError::MissingPendingLogin`]。
    pub async fn move_token(&self, old: &str, new: &str) -> Result<(), AccountError> {
        let mut entries = self.lock_live().await;
        let timed = entries
            .remove(old)
            .ok_or(AccountError::MissingPendingLogin)?;
        entries.insert(new.to_string(), timed);
        tracing::debug!(entry_count = entries.len(), "pending login cache moved");
        Ok(())
    }

    /// 读取密码副本，但不将 `password_reused` 置位。
    ///
    /// 用于在发起远程验证前取出明文，真正的一次性消费仍由 [`Self::reuse_password_once`] 完成。
    /// 条目不存在、已过期或没有密码时返回 [`AccountError::MissingPendingLogin`]；
    /// 密码已被消费过则返回 [`AccountError::PasswordAlreadyReused`]。
    pub async fn peek_password(
        &self,
        token: &str,
    ) -> Result<zeroize::Zeroizing<String>, AccountError> {
        let entries = self.lock_live().await;
        let timed = entries
            .get(token)
            .ok_or(AccountError::MissingPendingLogin)?;
        let password = timed
            .login
            .password
            .as_ref()
            .ok_or(AccountError::MissingPendingLogin)?;
        if timed.login.password_reused {
            return Err(AccountError::PasswordAlreadyReused);
        }
        Ok(password.clone())
    }

    /// 一次性取出指定条目中的密码副本，并将 `password_reused` 置为已消费。
    ///
    /// 条目不存在、已过期或没有密码时返回 [`AccountError::MissingPendingLogin`]；
    /// 密码已被消费过则返回 [`AccountError::PasswordAlreadyReused`]。
    pub async fn reuse_password_once(
        &self,
        token: &str,
    ) -> Result<zeroize::Zeroizing<String>, AccountError> {
        let mut entries = self.lock_live().await;
        let timed = entries
            .get_mut(token)
            .ok_or(AccountError::MissingPendingLogin)?;
        let password = timed
            .login
            .password
            .as_ref()
            .ok_or(AccountError::MissingPendingLogin)?;
        if timed.login.password_reused {
            return Err(AccountError::PasswordAlreadyReused);
        }
        timed.login.password_reused = true;
        Ok(password.clone())
    }

    /// 取出并删除指定 token 的待登录上下文；条目不存在或已过期时返回 `None`。
    pub async fn take(&self, token: &str) -> Option<PendingLogin> {
        let mut entries = self.lock_live().await;
        let login = entries.remove(token).map(|timed| timed.login);
        tracing::debug!(entry_count = entries.len(), "pending login cache taken");
        login
    }

    /// 清空全部待登录条目，无论是否过期。
    pub async fn clear(&self) {
        let mut entries = self.lock_live().await;
        entries.clear();
        tracing::debug!(entry_count = 0, "pending login cache cleared");
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingLogin, PendingLoginCache};
    use crate::account::AccountError;
    use std::time::{Duration, Instant};

    /// 构造一条带密码的待登录记录，供生命周期与复用测试使用。
    fn password_login() -> PendingLogin {
        PendingLogin {
            display_account: "a@example.com".into(),
            primary_login_type: 4,
            password: Some(zeroize::Zeroizing::new("secret".into())),
            password_reused: false,
        }
    }

    #[tokio::test]
    async fn pending_login_secret_moves_to_challenge_token_and_is_taken_once() {
        let cache = PendingLoginCache::default();
        cache
            .insert(
                "issued",
                PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 4,
                    password: Some(zeroize::Zeroizing::new("secret".into())),
                    password_reused: false,
                },
            )
            .await;
        cache.move_token("issued", "challenge").await.unwrap();
        assert!(cache.get("issued").await.is_none());
        let taken = cache.take("challenge").await.unwrap();
        assert_eq!(taken.display_account, "a@example.com");
        assert_eq!(taken.primary_login_type, 4);
        assert!(cache.take("challenge").await.is_none());
    }

    /// 旧 token 不存在时，`move_token` 必须返回 `MissingPendingLogin`。
    #[tokio::test]
    async fn move_token_missing_old_key_returns_missing_pending_login() {
        let cache = PendingLoginCache::default();
        let error = cache.move_token("missing", "challenge").await.unwrap_err();
        assert!(matches!(error, AccountError::MissingPendingLogin));
    }

    /// `clear` 必须删除全部条目，无论是否过期。
    #[tokio::test]
    async fn pending_login_clear_removes_all_entries() {
        let cache = PendingLoginCache::default();
        cache.insert("issued", password_login()).await;
        cache.clear().await;
        assert!(cache.get("issued").await.is_none());
    }

    /// 同一 token 的密码只能被 `reuse_password_once` 消费一次。
    #[tokio::test]
    async fn reuse_password_once_rejects_second_consumption() {
        let cache = PendingLoginCache::default();
        cache.insert("tok", password_login()).await;

        let first = cache.reuse_password_once("tok").await.unwrap();
        assert_eq!(&*first, "secret");

        let error = cache.reuse_password_once("tok").await.unwrap_err();
        assert!(matches!(error, AccountError::PasswordAlreadyReused));
    }

    /// `peek_password` 只读取副本，不得把 `password_reused` 置位；已消费后 peek 也拒绝。
    #[tokio::test]
    async fn peek_password_does_not_consume_reuse() {
        let cache = PendingLoginCache::default();
        cache.insert("tok", password_login()).await;

        let first = cache.peek_password("tok").await.unwrap();
        assert_eq!(&*first, "secret");
        let second = cache.peek_password("tok").await.unwrap();
        assert_eq!(&*second, "secret");
        assert!(!cache.get("tok").await.unwrap().password_reused);

        cache.reuse_password_once("tok").await.unwrap();
        let error = cache.peek_password("tok").await.unwrap_err();
        assert!(matches!(error, AccountError::PasswordAlreadyReused));
    }

    /// 超过 10 分钟的记录必须在后续操作中被清理，未过期记录应保留。
    #[tokio::test]
    async fn pending_login_entries_older_than_ten_minutes_are_evicted() {
        let cache = PendingLoginCache::default();
        let expired_at = Instant::now() - Duration::from_secs(10 * 60 + 1);
        cache
            .insert_at("expired", password_login(), expired_at)
            .await;
        cache.insert("fresh", password_login()).await;

        assert!(cache.get("expired").await.is_none());
        assert!(cache.get("fresh").await.is_some());
    }
}
