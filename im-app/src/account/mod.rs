//! 账号持久化基础设施，统一管理账号级文件布局、非敏感账号索引与后续账号操作错误。

use thiserror::Error;

/// 账号文件路径模型。
pub mod paths;

/// 非敏感账号索引。
pub mod index;

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
    #[error("不存在待完成的登录")]
    MissingPendingLogin,
    /// 登录流程试图再次消费已经使用过的密码。
    #[error("登录密码已被使用，禁止重复消费")]
    PasswordAlreadyReused,
}
