//! 账号持久化基础设施，统一管理账号级文件布局、非敏感账号索引、系统凭据库与活动数据库。
//!
//! [`AppState`](crate::state::AppState) 持有这些服务，但未登录时不打开业务 SQLite。

use thiserror::Error;

/// 账号文件路径模型。
pub mod paths;

/// 非敏感账号索引。
pub mod index;

/// 系统凭据库封装。
pub mod credentials;

/// 按 UID 隔离的活动账号数据库。
pub mod database;

/// 旧单库一次性迁移。
pub mod migration;

/// 尚未完成二次验证的短期登录秘密缓存。
pub mod pending_login;

/// Token 恢复、会话发布以及启动时的最后账号路由。
pub mod session;

pub use credentials::{CredentialStore, KeyringCredentialStore};
pub use database::AccountDatabaseManager;
pub use index::AccountIndexStore;
pub use migration::LegacyDatabaseMigrator;
pub use paths::AppPaths;
pub use pending_login::PendingLoginCache;

/// 账号持久化与会话切换过程中可能出现的统一错误。
///
/// 该类型保留底层文件系统、JSON 与数据库错误的来源，同时为凭据不可用、
/// 登录流程状态异常和活动账号冲突提供稳定的业务错误边界。
#[derive(Debug, Error)]
pub enum AccountError {
    /// UID 小于或等于零，不能用于构造账号目录。
    #[error("账号 UID 必须为正整数，收到 {0}")]
    InvalidUid(i64),
    /// 账号文件或目录的读写操作失败。
    #[error("账号文件系统操作失败: {0}")]
    Io(#[from] std::io::Error),
    /// 账号索引或迁移状态的 JSON 编解码失败。
    #[error("账号 JSON 数据处理失败: {0}")]
    Json(#[from] serde_json::Error),
    /// 系统凭据存储当前不可用；字符串包含可安全展示的原因摘要。
    #[allow(dead_code)]
    #[error("系统凭据存储不可用: {0}")]
    CredentialUnavailable(String),
    /// 当前没有已激活的账号数据库。
    #[error("当前没有活动账号数据库")]
    NoActiveDatabase,
    /// 请求的账号与当前活动账号不一致。
    #[error("活动账号 UID {active} 与请求 UID {requested} 不一致")]
    ActiveUidMismatch {
        /// 当前已激活数据库所属的 UID。
        active: i64,
        /// 本次操作请求访问的 UID。
        requested: i64,
    },
    /// 账号数据库操作失败。
    #[error("账号数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
    /// 登录流程缺少尚待完成的登录上下文。
    ///
    /// 变体已由 [`pending_login`] 构造；生产命令接入前二进制目标仍视为未使用。
    #[cfg_attr(not(test), allow(dead_code))]
    #[error("不存在待完成的登录")]
    MissingPendingLogin,
    /// 登录流程试图再次消费已经使用过的密码。
    ///
    /// 变体已由 [`pending_login`] 构造；生产命令接入前二进制目标仍视为未使用。
    #[cfg_attr(not(test), allow(dead_code))]
    #[error("登录密码已被使用，禁止重复消费")]
    PasswordAlreadyReused,
    /// 过期代际试图打开另一 UID 并替换已经打开的较新活动库。
    #[error("打开账号库的代际 {incoming} 已落后于活动账号 {active_uid} 的代际 {active_generation}")]
    StaleOpenGeneration {
        /// 本次过期打开使用的认证代际。
        incoming: u64,
        /// 当前活动库所属的 UID。
        active_uid: i64,
        /// 当前活动库记录的认证代际。
        active_generation: u64,
    },
}
