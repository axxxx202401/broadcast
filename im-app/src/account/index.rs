//! 非敏感账号索引：持久化账号摘要与最后使用账号，不保存密码或 Token。

use super::AccountError;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// 单条账号的非密钥摘要。
///
/// 该记录只保存展示与状态标志，不得写入密码、Token、密码掩码或 Token 摘要。
/// 序列化字段名固定为 camelCase。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    /// 服务端账号 UID。
    pub uid: i64,
    /// 用户输入的邮箱或手机号，仅用于展示和回填。
    pub display_account: String,
    /// 登录该账号时使用的登录方式标识。
    pub login_type: i32,
    /// 该账号最近一次被选为当前账号的时间戳，数值含义由写入方决定。
    pub last_used_at: i64,
    /// 系统凭据库中是否已保存该账号密码；索引本身不存密码。
    pub has_saved_password: bool,
    /// 系统凭据库中是否仍保存 Token；索引本身不存 Token。
    pub has_token: bool,
}

impl AccountRecord {
    /// 构造一条新登录账号记录。
    ///
    /// 新写入账号默认视为已持有 Token（`has_token = true`），且尚未记录已保存密码。
    /// 调用方随后可通过 [`AccountIndexStore::set_secret_flags`] 或
    /// [`AccountIndexStore::mark_logged_out`] 更新这两个标志。
    pub fn new(
        uid: i64,
        display_account: impl Into<String>,
        login_type: i32,
        last_used_at: i64,
    ) -> Self {
        Self {
            uid,
            display_account: display_account.into(),
            login_type,
            last_used_at,
            has_saved_password: false,
            has_token: true,
        }
    }
}

/// 全局非敏感账号索引快照。
///
/// `last_used_uid` 记录启动恢复或登录页应优先选中的账号；`accounts` 保存全部已知账号摘要。
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountIndex {
    /// 最后使用账号的 UID；没有任何账号时为 `None`。
    pub last_used_uid: Option<i64>,
    /// 已保存的账号摘要列表，按首次写入顺序保留。
    pub accounts: Vec<AccountRecord>,
}

/// 账号索引文件的读写仓储。
///
/// 所有改写操作持有 `write_lock` 完成读改写，避免并发更新互相覆盖。
/// 持久化先写同目录临时文件并 `sync_all`，再替换正式 `accounts.json`。
pub struct AccountIndexStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl AccountIndexStore {
    /// 使用指定索引文件路径创建仓储。
    ///
    /// 构造时不读取或创建文件；首次 [`Self::load`] 在文件缺失时返回默认空索引。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
    }

    /// 读取当前账号索引快照。
    ///
    /// 文件不存在时返回 [`AccountIndex::default`]。JSON 损坏时返回 [`AccountError::Json`]，
    /// 不会改写或清空原文件。
    pub async fn load(&self) -> Result<AccountIndex, AccountError> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(AccountIndex::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// 写入或更新一条账号记录，并把该 UID 标为最后使用账号。
    ///
    /// 同一 UID 再次写入会覆盖原摘要而不是追加重复项。新登录账号视为已持有 Token
    ///（`has_token = true`）。本方法不写入密码、Token 或任何密钥摘要。
    pub async fn upsert(&self, mut record: AccountRecord) -> Result<(), AccountError> {
        self.mutate(|index| {
            record.has_token = true;
            let uid = record.uid;
            if let Some(existing) = index.accounts.iter_mut().find(|item| item.uid == uid) {
                *existing = record;
            } else {
                index.accounts.push(record);
            }
            index.last_used_uid = Some(uid);
        })
        .await
    }

    /// 将指定账号标记为已退出：`has_token = false`，保留记录与 `has_saved_password`。
    ///
    /// 不改变 `last_used_uid`，以便登录页继续选中刚退出的账号。未知 UID 不修改索引。
    pub async fn mark_logged_out(&self, uid: i64) -> Result<(), AccountError> {
        self.mutate(|index| {
            if let Some(record) = index.accounts.iter_mut().find(|item| item.uid == uid) {
                record.has_token = false;
            }
        })
        .await
    }

    /// 更新指定账号的非密钥存在性标志。
    ///
    /// 只改 `has_saved_password` 与 `has_token`，不改展示账号、登录方式、最近使用时间
    /// 或 `last_used_uid`。未知 UID 不修改索引。
    pub async fn set_secret_flags(
        &self,
        uid: i64,
        has_saved_password: bool,
        has_token: bool,
    ) -> Result<(), AccountError> {
        self.mutate(|index| {
            if let Some(record) = index.accounts.iter_mut().find(|item| item.uid == uid) {
                record.has_saved_password = has_saved_password;
                record.has_token = has_token;
            }
        })
        .await
    }

    /// 从索引中删除指定账号。
    ///
    /// 不删除该 UID 的 SQLite 文件。若删除的是 `last_used_uid`，则改选剩余账号中
    /// `last_used_at` 最大者；没有剩余账号时清空 `last_used_uid`。未知 UID 不修改索引。
    pub async fn remove(&self, uid: i64) -> Result<(), AccountError> {
        self.mutate(|index| {
            index.accounts.retain(|item| item.uid != uid);
            if index.last_used_uid == Some(uid) {
                index.last_used_uid = index
                    .accounts
                    .iter()
                    .max_by_key(|item| item.last_used_at)
                    .map(|item| item.uid);
            }
        })
        .await
    }

    /// 在写锁保护下读取、修改并原子写回索引，避免并发读改写丢失更新。
    async fn mutate(&self, update: impl FnOnce(&mut AccountIndex)) -> Result<(), AccountError> {
        let _guard = self.write_lock.lock().await;
        let mut index = self.load().await?;
        update(&mut index);
        self.save(&index).await
    }

    /// 将索引原子写入正式文件。
    ///
    /// 先写入同目录临时文件并 `sync_all`，再替换正式路径，避免半写入损坏已有索引。
    async fn save(&self, index: &AccountIndex) -> Result<(), AccountError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        let tmp_path = self.path.with_extension("json.tmp");
        let payload = serde_json::to_vec(index)?;
        let mut file = tokio::fs::File::create(&tmp_path).await?;
        file.write_all(&payload).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp_path, &self.path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountIndex, AccountIndexStore, AccountRecord};
    use crate::account::AccountError;

    /// 新写入账号成为最后使用账号；退出只清 Token 标志，保留记录。
    #[tokio::test]
    async fn account_index_tracks_last_account_and_logout_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(temp.path().join("accounts.json"));
        store
            .upsert(AccountRecord::new(42, "a@example.com", 4, 100))
            .await
            .unwrap();
        store
            .upsert(AccountRecord::new(84, "13800138000", 3, 200))
            .await
            .unwrap();
        store.mark_logged_out(84).await.unwrap();

        let snapshot = store.load().await.unwrap();
        assert_eq!(snapshot.last_used_uid, Some(84));
        assert!(
            !snapshot
                .accounts
                .iter()
                .find(|item| item.uid == 84)
                .unwrap()
                .has_token
        );
        assert_eq!(snapshot.accounts.len(), 2);
    }

    /// 删除当前最后账号后，按 `last_used_at` 选中剩余账号中最近使用的一条。
    #[tokio::test]
    async fn remove_preserves_other_accounts_and_selects_most_recent() {
        let temp = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(temp.path().join("accounts.json"));
        store
            .upsert(AccountRecord::new(84, "13800138000", 3, 300))
            .await
            .unwrap();
        store
            .upsert(AccountRecord::new(42, "a@example.com", 4, 100))
            .await
            .unwrap();
        store
            .upsert(AccountRecord::new(21, "b@example.com", 4, 200))
            .await
            .unwrap();

        store.remove(21).await.unwrap();

        let snapshot = store.load().await.unwrap();
        assert_eq!(snapshot.last_used_uid, Some(84));
        assert_eq!(
            snapshot
                .accounts
                .iter()
                .map(|item| item.uid)
                .collect::<Vec<_>>(),
            vec![84, 42]
        );
    }

    /// 索引文件尚未创建时，读取返回空快照而不是错误。
    #[tokio::test]
    async fn load_returns_default_when_index_file_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(temp.path().join("accounts.json"));
        let snapshot = store.load().await.unwrap();
        assert_eq!(snapshot, AccountIndex::default());
    }

    /// 损坏的 JSON 以结构化错误返回，并保留原文件内容。
    #[tokio::test]
    async fn load_rejects_corrupt_account_index_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("accounts.json");
        std::fs::write(&path, "{not-json").unwrap();
        let store = AccountIndexStore::new(path.clone());

        let error = store.load().await.unwrap_err();
        assert!(matches!(error, AccountError::Json(_)));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{not-json");
    }

    /// 同一 UID 再次写入会覆盖展示信息，不会追加重复记录。
    #[tokio::test]
    async fn upsert_replaces_existing_account_without_duplicating() {
        let temp = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(temp.path().join("accounts.json"));
        store
            .upsert(AccountRecord::new(42, "old@example.com", 4, 100))
            .await
            .unwrap();
        store
            .upsert(AccountRecord::new(42, "new@example.com", 3, 200))
            .await
            .unwrap();

        let snapshot = store.load().await.unwrap();
        assert_eq!(snapshot.accounts.len(), 1);
        assert_eq!(snapshot.accounts[0].display_account, "new@example.com");
        assert_eq!(snapshot.accounts[0].login_type, 3);
        assert_eq!(snapshot.accounts[0].last_used_at, 200);
        assert!(snapshot.accounts[0].has_token);
        assert_eq!(snapshot.last_used_uid, Some(42));
    }

    /// 密钥存在性标志可单独更新，且不改动展示字段或最后账号。
    #[tokio::test]
    async fn set_secret_flags_updates_password_and_token_presence() {
        let temp = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(temp.path().join("accounts.json"));
        store
            .upsert(AccountRecord::new(42, "a@example.com", 4, 100))
            .await
            .unwrap();
        store.set_secret_flags(42, true, false).await.unwrap();

        let snapshot = store.load().await.unwrap();
        let record = snapshot
            .accounts
            .iter()
            .find(|item| item.uid == 42)
            .unwrap();
        assert!(record.has_saved_password);
        assert!(!record.has_token);
        assert_eq!(record.display_account, "a@example.com");
        assert_eq!(snapshot.last_used_uid, Some(42));
    }
}
