//! 旧单库一次性迁移：把升级前的共享 `im_monitor.db` 复制到首个成功登录账号的隔离目录。
//!
//! 迁移只执行一次。成功、无旧库或目标已存在都会写入带明确枚举的 `migration.json`；
//! 复制或校验失败不写标记，以便下次登录重试。原旧库文件始终保留。
//! 生产接线完成前，二进制目标不会引用本模块公开类型，因此允许 dead_code。

#![allow(dead_code)]

use super::paths::AppPaths;
use super::AccountError;
use im_store::SqliteStore;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// 一次性迁移的结果。
///
/// 序列化字段名固定为 camelCase。`AlreadyHandled` 只作为再次调用的返回值，
/// 不会写入 `migration.json`；落盘的 outcome 只能是其余三个明确枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationOutcome {
    /// 旧库已复制到目标账号目录，且原文件仍保留。
    Migrated,
    /// 根目录已存在 `migration.json`，本次不再扫描或复制。
    AlreadyHandled,
    /// 未发现旧单库，已记录“无需迁移”。
    NoLegacyDatabase,
    /// 目标账号库已存在，未覆盖其内容。
    TargetAlreadyExists,
}

/// 根目录 `migration.json` 的落盘内容。
///
/// `outcome` 必须是 [`MigrationOutcome`] 枚举，禁止用自由文本推断迁移状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationMarker {
    /// 触发这次迁移判定的账号 UID。
    pub uid: i64,
    /// 写入标记时的 Unix 毫秒时间戳。
    pub completed_at: i64,
    /// 这次判定的明确结果。
    pub outcome: MigrationOutcome,
}

/// 旧单库迁移器。
///
/// 所有判定与复制持有 `migrate_lock`，避免并发首次登录同时写入半成品目标库。
/// 本类型不打开活动账号数据库，也不接入 AppState。
pub struct LegacyDatabaseMigrator {
    paths: AppPaths,
    migrate_lock: Mutex<()>,
}

impl LegacyDatabaseMigrator {
    /// 使用应用路径模型创建迁移器，构造时不读写任何文件。
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            migrate_lock: Mutex::new(()),
        }
    }

    /// 在需要时把旧单库一次性迁移到指定 UID 的隔离目录。
    ///
    /// UID 必须为正整数，否则返回 [`AccountError::InvalidUid`]。已有标记时返回
    /// [`MigrationOutcome::AlreadyHandled`]，即使标记里记录的是其他 outcome 或其他 UID。
    /// 复制使用 `VACUUM INTO` 写入账号目录临时文件，再用 [`SqliteStore::new`] 校验 schema；
    /// 任一步失败都不会写入 `migration.json`，并尽量删除临时文件。
    pub async fn migrate_if_needed(&self, uid: i64) -> Result<MigrationOutcome, AccountError> {
        let target = self.paths.account_db(uid)?;
        let _guard = self.migrate_lock.lock().await;

        if tokio::fs::try_exists(self.paths.migration_marker()).await? {
            return Ok(MigrationOutcome::AlreadyHandled);
        }

        let legacy = self.paths.legacy_db();
        if !tokio::fs::try_exists(&legacy).await? {
            self.write_marker(uid, MigrationOutcome::NoLegacyDatabase)
                .await?;
            return Ok(MigrationOutcome::NoLegacyDatabase);
        }

        if tokio::fs::try_exists(&target).await? {
            self.write_marker(uid, MigrationOutcome::TargetAlreadyExists)
                .await?;
            return Ok(MigrationOutcome::TargetAlreadyExists);
        }

        let parent = target.parent().ok_or_else(|| {
            std::io::Error::other(format!("账号数据库路径 {} 缺少父目录", target.display()))
        })?;
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("创建账号数据库目录 {} 失败: {error}", parent.display()),
            )
        })?;

        let temp = parent.join("im_monitor.migrating.db");
        if let Err(error) = self.copy_and_verify(&legacy, &temp).await {
            cleanup_sqlite_files(&temp).await;
            return Err(error);
        }

        if let Err(error) = replace_file(&temp, &target).await {
            cleanup_sqlite_files(&temp).await;
            return Err(error);
        }

        self.write_marker(uid, MigrationOutcome::Migrated).await?;
        Ok(MigrationOutcome::Migrated)
    }

    /// 用 `VACUUM INTO` 把旧库写入临时目标，再用 SqliteStore 打开并读取 schema 确认可用。
    async fn copy_and_verify(&self, legacy: &Path, temp: &Path) -> Result<(), AccountError> {
        cleanup_sqlite_files(temp).await;

        // 只读打开旧库，避免迁移过程改写原文件；VACUUM INTO 把一致快照写入新文件。
        let options = SqliteConnectOptions::new()
            .filename(legacy)
            .create_if_missing(false)
            .read_only(true);
        let pool = SqlitePool::connect_with(options).await.map_err(|error| {
            std::io::Error::other(format!("打开旧数据库 {} 失败: {error}", legacy.display()))
        })?;
        let vacuum = sqlx::query("VACUUM INTO ?")
            .bind(temp.to_string_lossy().as_ref())
            .execute(&pool)
            .await;
        pool.close().await;
        vacuum.map_err(|error| {
            std::io::Error::other(format!(
                "VACUUM INTO {} 失败: {error}",
                temp.display()
            ))
        })?;

        let store = SqliteStore::new(&temp.to_string_lossy())
            .await
            .map_err(|error| {
                std::io::Error::other(format!(
                    "打开迁移临时库 {} 失败: {error}",
                    temp.display()
                ))
            })?;
        let verified = verify_usable_schema(&store, temp).await;
        // 校验期间 SqliteStore 会以 WAL 打开临时库；截断 checkpoint 后再关闭，
        // 避免改名后留下无法跟随的 -wal / -shm 旁路文件。
        let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&store.pool)
            .await;
        store.pool.close().await;
        verified?;
        checkpoint.map_err(|error| {
            std::io::Error::other(format!(
                "截断迁移临时库 {} 的 WAL 失败: {error}",
                temp.display()
            ))
        })?;
        cleanup_sqlite_sidecars(temp).await;
        Ok(())
    }

    /// 原子写入包含 UID、时间和明确 outcome 的 `migration.json`。
    async fn write_marker(
        &self,
        uid: i64,
        outcome: MigrationOutcome,
    ) -> Result<(), AccountError> {
        let marker = MigrationMarker {
            uid,
            completed_at: unix_ms(),
            outcome,
        };
        let path = self.paths.migration_marker();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        let tmp_path = path.with_extension("json.tmp");
        let payload = serde_json::to_vec(&marker)?;
        let mut file = tokio::fs::File::create(&tmp_path).await?;
        file.write_all(&payload).await?;
        file.sync_all().await?;
        drop(file);
        replace_file(&tmp_path, &path).await
    }
}

/// 确认临时库含有业务表且群组查询可用。
async fn verify_usable_schema(store: &SqliteStore, temp: &Path) -> Result<(), AccountError> {
    let groups: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'groups'",
    )
    .fetch_one(&store.pool)
    .await?;
    if groups == 0 {
        return Err(std::io::Error::other(format!(
            "迁移临时库 {} 缺少 groups 表",
            temp.display()
        ))
        .into());
    }
    store.groups.list_all().await?;
    Ok(())
}

/// 用已同步的临时文件替换正式路径。
///
/// Windows 上 `rename` 不能覆盖已存在目标，所以先删除目标再改名；目标不存在视为成功。
async fn replace_file(from: &Path, to: &Path) -> Result<(), AccountError> {
    #[cfg(windows)]
    {
        match tokio::fs::remove_file(to).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    tokio::fs::rename(from, to).await?;
    Ok(())
}

/// 删除 SQLite 主文件及其 WAL / SHM 旁路，忽略文件不存在。
async fn cleanup_sqlite_files(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
    cleanup_sqlite_sidecars(path).await;
}

/// 删除 `*.db-wal` 与 `*.db-shm`，忽略文件不存在。
async fn cleanup_sqlite_sidecars(path: &Path) {
    for sidecar in sqlite_sidecars(path) {
        let _ = tokio::fs::remove_file(sidecar).await;
    }
}

/// SQLite WAL 模式在主文件名后追加 `-wal` / `-shm`。
fn sqlite_sidecars(path: &Path) -> [PathBuf; 2] {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let mut shm = path.as_os_str().to_os_string();
    shm.push("-shm");
    [PathBuf::from(wal), PathBuf::from(shm)]
}

/// 当前 Unix 毫秒；时钟异常时退回 0，避免迁移因时间失败。
fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{LegacyDatabaseMigrator, MigrationMarker, MigrationOutcome};
    use crate::account::paths::AppPaths;
    use crate::account::AccountError;
    use im_store::group::GroupRow;
    use im_store::SqliteStore;
    use std::path::Path;

    /// 构造测试用群组行，只填充迁移前后数据对比所需的字段。
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

    /// 在指定路径创建真实 SQLite，并写入一条群组，用于证明后续复制带走了业务数据。
    async fn seed_database(path: &Path, group_id: i64, name: &str) {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.unwrap();
            }
        }
        let store = SqliteStore::new(&path.to_string_lossy()).await.unwrap();
        store
            .groups
            .insert_or_update(&group_row(group_id, name))
            .await
            .unwrap();
        store.pool.close().await;
    }

    /// 在旧单库路径写入一条可识别的群组。
    async fn seed_legacy_database(path: &Path) {
        seed_database(path, 7, "旧库群").await;
    }

    /// 读取并反序列化 `migration.json`，要求字段是明确枚举而不是自由文本。
    fn read_marker(paths: &AppPaths) -> MigrationMarker {
        let bytes = std::fs::read(paths.migration_marker()).unwrap();
        serde_json::from_slice(&bytes).expect("migration.json 必须是可反序列化的 MigrationMarker")
    }

    /// 打开账号库并返回全部可见群组名称，用于确认复制结果或目标未被覆盖。
    async fn list_group_names(path: &Path) -> Vec<String> {
        let store = SqliteStore::new(&path.to_string_lossy()).await.unwrap();
        let names = store
            .groups
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .map(|group| group.name)
            .collect();
        store.pool.close().await;
        names
    }

    /// 旧库只迁移一次：目标库出现、原文件保留、标记落盘；再次启动即使换成其他 UID 也视为已处理。
    #[tokio::test]
    async fn legacy_database_is_migrated_once_and_original_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path().to_path_buf());
        seed_legacy_database(&paths.legacy_db()).await;
        let migrator = LegacyDatabaseMigrator::new(paths.clone());

        assert_eq!(
            migrator.migrate_if_needed(42).await.unwrap(),
            MigrationOutcome::Migrated
        );
        assert!(paths.legacy_db().exists());
        assert!(paths.account_db(42).unwrap().exists());
        assert!(paths.migration_marker().exists());
        assert_eq!(
            migrator.migrate_if_needed(84).await.unwrap(),
            MigrationOutcome::AlreadyHandled
        );

        let marker = read_marker(&paths);
        assert_eq!(marker.uid, 42);
        assert_eq!(marker.outcome, MigrationOutcome::Migrated);
        assert!(marker.completed_at > 0);
        assert_eq!(
            list_group_names(&paths.account_db(42).unwrap()).await,
            vec!["旧库群".to_string()]
        );
        assert_eq!(
            list_group_names(&paths.legacy_db()).await,
            vec!["旧库群".to_string()]
        );
    }

    /// 没有旧库时写入 `NoLegacyDatabase` 标记，后续启动不再扫描文件系统语义之外的状态。
    #[tokio::test]
    async fn migrate_if_needed_records_no_legacy_database() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path().to_path_buf());
        let migrator = LegacyDatabaseMigrator::new(paths.clone());

        assert_eq!(
            migrator.migrate_if_needed(42).await.unwrap(),
            MigrationOutcome::NoLegacyDatabase
        );
        assert!(!paths.account_db(42).unwrap().exists());
        let marker = read_marker(&paths);
        assert_eq!(marker.uid, 42);
        assert_eq!(marker.outcome, MigrationOutcome::NoLegacyDatabase);
        assert_eq!(
            migrator.migrate_if_needed(84).await.unwrap(),
            MigrationOutcome::AlreadyHandled
        );
    }

    /// 目标账号库已存在时不得覆盖其数据，只写入 `TargetAlreadyExists` 标记。
    #[tokio::test]
    async fn migrate_if_needed_does_not_overwrite_existing_account_database() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path().to_path_buf());
        seed_legacy_database(&paths.legacy_db()).await;
        seed_database(&paths.account_db(42).unwrap(), 9, "目标群").await;
        let migrator = LegacyDatabaseMigrator::new(paths.clone());

        assert_eq!(
            migrator.migrate_if_needed(42).await.unwrap(),
            MigrationOutcome::TargetAlreadyExists
        );
        assert_eq!(
            list_group_names(&paths.account_db(42).unwrap()).await,
            vec!["目标群".to_string()]
        );
        assert_eq!(
            list_group_names(&paths.legacy_db()).await,
            vec!["旧库群".to_string()]
        );
        let marker = read_marker(&paths);
        assert_eq!(marker.uid, 42);
        assert_eq!(marker.outcome, MigrationOutcome::TargetAlreadyExists);
    }

    /// 账号目录无法创建时不得落下 `migration.json`；该失败发生在 `copy_and_verify` 之前。
    #[tokio::test]
    async fn migrate_if_needed_does_not_write_marker_when_account_directory_cannot_be_created() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path().to_path_buf());
        seed_legacy_database(&paths.legacy_db()).await;

        let account_dir = paths.account_db(42).unwrap();
        let account_dir = account_dir.parent().expect("账号库路径必须包含父目录");
        std::fs::create_dir_all(account_dir.parent().expect("accounts 目录必须存在")).unwrap();
        std::fs::write(account_dir, b"not-a-directory").unwrap();

        let migrator = LegacyDatabaseMigrator::new(paths.clone());
        assert!(
            migrator.migrate_if_needed(42).await.is_err(),
            "账号目录被文件占用时迁移必须失败"
        );
        assert!(
            !paths.migration_marker().exists(),
            "创建账号目录失败时不得写入 migration.json"
        );
    }

    /// 旧库不是 SQLite 时必须进入 `copy_and_verify` 并失败：不写标记，也不留下临时库或 WAL 旁路。
    #[tokio::test]
    async fn migrate_if_needed_does_not_write_marker_when_copy_or_verify_fails() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path().to_path_buf());
        std::fs::write(paths.legacy_db(), b"this is not a sqlite database").unwrap();
        let migrator = LegacyDatabaseMigrator::new(paths.clone());

        assert!(
            migrator.migrate_if_needed(42).await.is_err(),
            "非 SQLite 旧库必须使打开或 VACUUM 失败"
        );
        assert!(
            !paths.migration_marker().exists(),
            "复制或校验失败时不得写入 migration.json"
        );

        let account_dir = paths
            .account_db(42)
            .unwrap()
            .parent()
            .expect("账号库路径必须包含父目录")
            .to_path_buf();
        for leftover in [
            account_dir.join("im_monitor.migrating.db"),
            account_dir.join("im_monitor.migrating.db-wal"),
            account_dir.join("im_monitor.migrating.db-shm"),
        ] {
            assert!(
                !leftover.exists(),
                "复制失败后不得留下临时库或旁路文件: {}",
                leftover.display()
            );
        }
    }

    /// 非正 UID 必须走路径模型的 `InvalidUid`，不得开始迁移或写标记。
    #[tokio::test]
    async fn migrate_if_needed_rejects_non_positive_uid() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(temp.path().to_path_buf());
        seed_legacy_database(&paths.legacy_db()).await;
        let migrator = LegacyDatabaseMigrator::new(paths.clone());

        let error = match migrator.migrate_if_needed(0).await {
            Err(error) => error,
            Ok(_) => panic!("UID 0 不得开始迁移"),
        };
        assert!(matches!(error, AccountError::InvalidUid(0)));

        let error = match migrator.migrate_if_needed(-1).await {
            Err(error) => error,
            Ok(_) => panic!("负 UID 不得开始迁移"),
        };
        assert!(matches!(error, AccountError::InvalidUid(-1)));
        assert!(!paths.migration_marker().exists());
    }
}
