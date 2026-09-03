#![allow(dead_code)]

use super::AccountError;
use std::path::PathBuf;

/// 应用级账号文件的路径模型。
///
/// `root` 仅表示应用数据根目录；各方法在需要时派生索引、迁移标记、旧版数据库
/// 以及 UID 隔离后的账号数据库路径，不执行任何文件系统读写。
#[derive(Clone, Debug)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// 使用应用数据根目录创建路径模型。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 返回全局账号索引文件 `accounts.json` 的路径。
    pub fn index_file(&self) -> PathBuf {
        self.root.join("accounts.json")
    }

    /// 返回旧数据迁移状态文件 `migration.json` 的路径。
    pub fn migration_marker(&self) -> PathBuf {
        self.root.join("migration.json")
    }

    /// 返回升级前共享数据库 `im_monitor.db` 的路径。
    pub fn legacy_db(&self) -> PathBuf {
        self.root.join("im_monitor.db")
    }

    /// 返回指定 UID 的隔离数据库路径。
    ///
    /// UID 必须为正整数；零和负数没有合法账号语义，因此返回
    /// [`AccountError::InvalidUid`]，且不会创建目录或数据库文件。
    pub fn account_db(&self, uid: i64) -> Result<PathBuf, AccountError> {
        if uid <= 0 {
            return Err(AccountError::InvalidUid(uid));
        }

        Ok(self
            .root
            .join("accounts")
            .join(uid.to_string())
            .join("im_monitor.db"))
    }
}

#[cfg(test)]
mod tests {
    use super::AppPaths;
    use std::path::Path;

    /// 验证每个合法账号都映射到独立目录，并拒绝无法代表服务端账号的 UID。
    #[test]
    fn account_paths_keep_each_uid_in_its_own_directory() {
        let paths = AppPaths::new("/tmp/im-monitor");

        assert_eq!(
            paths.index_file(),
            Path::new("/tmp/im-monitor/accounts.json")
        );
        assert_eq!(
            paths.migration_marker(),
            Path::new("/tmp/im-monitor/migration.json")
        );
        assert_eq!(
            paths.legacy_db(),
            Path::new("/tmp/im-monitor/im_monitor.db")
        );
        assert_eq!(
            paths.account_db(42).expect("正 UID 应生成账号数据库路径"),
            Path::new("/tmp/im-monitor/accounts/42/im_monitor.db")
        );
        assert!(paths.account_db(0).is_err());
        assert!(paths.account_db(-1).is_err());
    }
}
