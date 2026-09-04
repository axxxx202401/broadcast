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

    /// 解析默认数据根目录：Unix 为 `~/.im-monitor`，Windows 为 `%USERPROFILE%\.im-monitor`。
    ///
    /// 不创建目录；调用方负责 `create_dir_all`。Tauri setup 之前可用此方法打开凭据库，
    /// 避免在已有 Tokio runtime 内嵌套 `block_on`。
    pub fn default_data_root() -> Result<PathBuf, std::io::Error> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "无法解析用户主目录")
            })?;
        Ok(PathBuf::from(home).join(".im-monitor"))
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

    /// 返回全局加密凭据库 `credentials.db` 的路径。
    pub fn credentials_db(&self) -> PathBuf {
        self.root.join("credentials.db")
    }

    /// 返回 AES-256-GCM 主密钥文件 `.credential_key` 的路径。
    pub fn credential_key_file(&self) -> PathBuf {
        self.root.join(".credential_key")
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
            paths.credentials_db(),
            Path::new("/tmp/im-monitor/credentials.db")
        );
        assert_eq!(
            paths.credential_key_file(),
            Path::new("/tmp/im-monitor/.credential_key")
        );
        assert_eq!(
            paths.account_db(42).expect("正 UID 应生成账号数据库路径"),
            Path::new("/tmp/im-monitor/accounts/42/im_monitor.db")
        );
        assert!(paths.account_db(0).is_err());
        assert!(paths.account_db(-1).is_err());
    }
}
