//! 活动账号数据库管理器：按 UID 打开、切换并关闭隔离的 SQLite 库。
//!
//! 未认证时不持有业务数据库。打开成功后才替换活动句柄；打开失败时保留原活动库。
use super::paths::AppPaths;
use super::AccountError;
use im_store::SqliteStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// 当前已打开的账号数据库句柄。
///
/// `store` 指向该 UID 的隔离 SQLite；切换时先打开新库并替换本结构，再关闭旧连接池。
/// `generation` 是最近一次成功 `open` 的认证代际，失败收尾据此避免关掉更新的同 UID 打开。
#[derive(Clone)]
pub struct ActiveDatabase {
    /// 当前活动数据库所属的账号 UID。
    pub uid: i64,
    /// 最近一次打开该库的认证代际。
    pub generation: u64,
    /// 该账号的 SQLite 存储入口。
    pub store: Arc<SqliteStore>,
}

/// 按 UID 管理活动账号数据库的打开、校验与关闭。
///
/// `switch_lock` 串行化打开与关闭，避免两个切换同时替换活动句柄。
/// `active` 只在新库打开成功后更新；错误文本可包含路径，但不得包含凭据。
pub struct AccountDatabaseManager {
    paths: AppPaths,
    active: RwLock<Option<ActiveDatabase>>,
    switch_lock: Mutex<()>,
}

impl AccountDatabaseManager {
    /// 使用应用路径模型创建管理器，此时尚未打开任何数据库。
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            active: RwLock::new(None),
            switch_lock: Mutex::new(()),
        }
    }

    /// 返回指定 UID 的隔离数据库路径，不打开文件。
    pub fn database_path(&self, uid: i64) -> Result<PathBuf, AccountError> {
        self.paths.account_db(uid)
    }

    /// 打开指定 UID 的账号数据库并设为活动库。
    ///
    /// 先校验 UID 并创建账号目录，再打开 [`SqliteStore`]。新库成功后才替换
    /// `active`；若已有其他账号的活动库，先取走旧句柄并关闭其连接池。
    /// 同一 UID 重复打开时复用现有句柄，避免关闭仍被调用方持有的连接池。
    /// `generation` 记录本次打开所属的认证代际，供失败收尾判断是否仍可关闭。
    /// 同 UID 复用时只允许把代际推进到 `max(当前, 本次)`，不得回退，否则过期
    /// `open` 会让随后的 `close_if_opened_by` 关掉已经发布的较新活动库。
    pub async fn open(&self, uid: i64, generation: u64) -> Result<Arc<SqliteStore>, AccountError> {
        let _switch = self.switch_lock.lock().await;
        let db_path = self.database_path(uid)?;

        {
            let mut active = self.active.write().await;
            if let Some(current) = active.as_mut() {
                if current.uid == uid {
                    current.generation = current.generation.max(generation);
                    return Ok(current.store.clone());
                }
            }
        }

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("创建账号数据库目录 {} 失败: {error}", parent.display()),
                )
            })?;
        }

        let store = Arc::new(SqliteStore::new(&db_path.to_string_lossy()).await.map_err(
            |error| {
                std::io::Error::other(format!(
                    "打开账号数据库 {} 失败: {error}",
                    db_path.display()
                ))
            },
        )?);

        let previous = {
            let mut active = self.active.write().await;
            active.replace(ActiveDatabase {
                uid,
                generation,
                store: store.clone(),
            })
        };
        if let Some(previous) = previous {
            previous.store.pool.close().await;
        }

        Ok(store)
    }

    /// 返回当前活动数据库；尚未打开时返回 [`AccountError::NoActiveDatabase`]。
    ///
    /// 生产命令通过 [`Self::require`] 按会话 UID 取库；本方法供未登录断言与测试使用。
    #[allow(dead_code)]
    pub async fn active(&self) -> Result<Arc<SqliteStore>, AccountError> {
        self.active
            .read()
            .await
            .as_ref()
            .map(|database| database.store.clone())
            .ok_or(AccountError::NoActiveDatabase)
    }

    /// 校验请求 UID 与当前活动账号一致后返回其数据库。
    ///
    /// 尚未打开时返回 [`AccountError::NoActiveDatabase`]；UID 不一致时返回
    /// [`AccountError::ActiveUidMismatch`]，避免旧会话拿到新账号数据库。
    pub async fn require(&self, uid: i64) -> Result<Arc<SqliteStore>, AccountError> {
        match self.active.read().await.as_ref() {
            None => Err(AccountError::NoActiveDatabase),
            Some(database) if database.uid != uid => Err(AccountError::ActiveUidMismatch {
                active: database.uid,
                requested: uid,
            }),
            Some(database) => Ok(database.store.clone()),
        }
    }

    /// 取走活动句柄并关闭其连接池；当前没有活动库时为空操作。
    pub async fn close(&self) {
        let _switch = self.switch_lock.lock().await;
        let previous = self.active.write().await.take();
        if let Some(previous) = previous {
            previous.store.pool.close().await;
        }
    }

    /// 仅当活动库属于 `uid` 且仍由指定 `generation` 打开时关闭连接池。
    ///
    /// 其他账号已占用活动库，或同一 UID 已被更新的代际重新打开时，都不得关闭。
    /// 登录失败收尾必须使用本方法，避免并发登录、切换或同账号重入后被误关。
    pub async fn close_if_opened_by(&self, uid: i64, generation: u64) {
        let _switch = self.switch_lock.lock().await;
        let mut active = self.active.write().await;
        if active
            .as_ref()
            .is_some_and(|database| database.uid == uid && database.generation == generation)
        {
            if let Some(previous) = active.take() {
                drop(active);
                previous.store.pool.close().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AccountDatabaseManager;
    use crate::account::paths::AppPaths;
    use crate::account::AccountError;
    use im_store::group::GroupRow;

    /// 构造测试用群组行，只填充分库隔离断言所需的字段。
    fn group_row(group_id: i64, name: &str) -> GroupRow {
        GroupRow {
            group_id,
            name: name.to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        }
    }

    /// 不同 UID 必须落到独立数据库；关闭后再打开另一账号时不得看见上一账号写入的群组。
    #[tokio::test]
    async fn database_manager_switches_between_uid_scoped_databases() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
        let first = manager.open(42, 1).await.unwrap();
        first
            .groups
            .insert_or_update(&group_row(7, "账号一"))
            .await
            .unwrap();
        manager.close().await;

        let second = manager.open(84, 1).await.unwrap();
        assert!(second.groups.list_all().await.unwrap().is_empty());
        assert_ne!(
            manager.database_path(42).unwrap(),
            manager.database_path(84).unwrap()
        );
        let error = match manager.require(42).await {
            Err(error) => error,
            Ok(_) => panic!("旧会话不得取得新账号数据库"),
        };
        assert!(matches!(
            error,
            AccountError::ActiveUidMismatch {
                active: 84,
                requested: 42
            }
        ));
    }

    /// 未调用 close 直接打开另一 UID 时，必须切到空库并禁止旧会话继续 require 原账号。
    #[tokio::test]
    async fn open_switches_active_database_without_explicit_close() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
        let first = manager.open(42, 1).await.unwrap();
        first
            .groups
            .insert_or_update(&group_row(7, "账号一"))
            .await
            .unwrap();

        let second = manager.open(84, 2).await.unwrap();
        assert!(second.groups.list_all().await.unwrap().is_empty());

        let error = match manager.require(42).await {
            Err(error) => error,
            Ok(_) => panic!("未关闭即切换后，旧会话不得取得新账号数据库"),
        };
        assert!(matches!(
            error,
            AccountError::ActiveUidMismatch {
                active: 84,
                requested: 42
            }
        ));
        assert!(
            first.groups.list_all().await.is_err(),
            "切换后旧连接池应已关闭，不能继续查询上一账号"
        );
    }

    /// 同一 UID 再次打开必须复用原 Arc，且第一次拿到的句柄仍能读到已写入群组。
    #[tokio::test]
    async fn open_reuses_store_for_the_same_uid() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
        let first = manager.open(42, 1).await.unwrap();
        first
            .groups
            .insert_or_update(&group_row(7, "账号一"))
            .await
            .unwrap();

        let second = manager.open(42, 1).await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(first.groups.list_all().await.unwrap().len(), 1);
        assert_eq!(first.groups.list_all().await.unwrap()[0].name, "账号一");
    }

    /// 同一 UID 被更新一代打开后，旧 generation 的关闭不得拆掉新活动库。
    #[tokio::test]
    async fn older_generation_does_not_close_newer_same_uid_open() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
        let first = manager.open(42, 1).await.unwrap();
        first
            .groups
            .insert_or_update(&group_row(7, "账号一"))
            .await
            .unwrap();
        let second = manager.open(42, 2).await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));

        manager.close_if_opened_by(42, 1).await;
        manager
            .require(42)
            .await
            .expect("较新同 UID 打开不得被旧 generation 关闭");
        assert_eq!(first.groups.list_all().await.unwrap()[0].name, "账号一");

        manager.close_if_opened_by(42, 2).await;
        let error = match manager.active().await {
            Err(error) => error,
            Ok(_) => panic!("匹配 uid 与 generation 时必须关闭活动库"),
        };
        assert!(matches!(error, AccountError::NoActiveDatabase));
    }

    /// 较旧的同 UID `open` 不得把活动代际回退，也不得因此让旧失败路径关掉新库。
    #[tokio::test]
    async fn older_open_does_not_downgrade_generation_or_close_newer_db() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
        let newer = manager.open(42, 2).await.unwrap();
        newer
            .groups
            .insert_or_update(&group_row(7, "账号一"))
            .await
            .unwrap();

        let older = manager.open(42, 1).await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&newer, &older));

        manager.close_if_opened_by(42, 1).await;
        manager
            .require(42)
            .await
            .expect("较旧 open 回退 generation 后不得关掉较新活动库");
        assert_eq!(newer.groups.list_all().await.unwrap()[0].name, "账号一");

        manager.close_if_opened_by(42, 2).await;
        let error = match manager.active().await {
            Err(error) => error,
            Ok(_) => panic!("较新 generation 仍应能关闭活动库"),
        };
        assert!(matches!(error, AccountError::NoActiveDatabase));
    }

    /// 活动库已切到其他 UID 时，按旧 UID 关闭不得影响当前库。
    #[tokio::test]
    async fn close_if_uid_leaves_other_active_database_open() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
        manager.open(42, 1).await.unwrap();
        manager.open(84, 2).await.unwrap();

        manager.close_if_opened_by(42, 1).await;
        manager
            .require(84)
            .await
            .expect("关闭旧 UID 不得影响当前活动库");
        manager.close_if_opened_by(84, 2).await;
        let error = match manager.active().await {
            Err(error) => error,
            Ok(_) => panic!("匹配 UID 时必须关闭活动库"),
        };
        assert!(matches!(error, AccountError::NoActiveDatabase));
    }

    /// 未打开任何账号数据库时，活动句柄与按 UID 索取都必须拒绝访问。
    #[tokio::test]
    async fn active_database_requires_authenticated_account() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));

        let error = match manager.active().await {
            Err(error) => error,
            Ok(_) => panic!("未打开数据库时不应返回活动句柄"),
        };
        assert!(matches!(error, AccountError::NoActiveDatabase));

        let error = match manager.require(42).await {
            Err(error) => error,
            Ok(_) => panic!("未打开数据库时不应按 UID 返回句柄"),
        };
        assert!(matches!(error, AccountError::NoActiveDatabase));
    }
}
