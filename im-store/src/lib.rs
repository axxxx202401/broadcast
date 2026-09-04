//! 基于 SQLite 的群组与消息持久化。
//!
//! 本 crate 通过 [`SqliteStore`] 共享一个连接池，并分别由 [`message::MessageStore`]
//! 和 [`group::GroupStore`] 提供消息、群组数据访问能力。

/// 群组数据访问类型。
pub mod group;
/// 开奖配置数据访问类型。
pub mod lottery_config;
/// 当前账号 App 密钥对数据访问类型。
pub mod key_pair;
/// 消息数据访问类型。
pub mod message;
/// SQLite 表结构定义。
pub mod schema;

#[cfg(test)]
mod tests;

use schema::SCHEMA_SQL;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

use crate::group::GroupStore;
use crate::key_pair::UserKeyPairStore;
use crate::lottery_config::LotteryConfigStore;
use crate::message::MessageStore;

/// SQLite 存储的总入口。
///
/// 该类型持有共享连接池，并暴露消息与群组两个数据访问入口。
pub struct SqliteStore {
    /// 底层 SQLite 连接池，可供调用方执行本 crate 未封装的查询。
    pub pool: SqlitePool,
    /// 使用同一连接池的消息数据访问入口。
    pub messages: MessageStore,
    /// 使用同一连接池的群组数据访问入口。
    pub groups: GroupStore,
    /// 使用同一连接池的当前账号 App 密钥对访问入口。
    pub key_pairs: UserKeyPairStore,
    /// 使用同一连接池的开奖配置数据访问入口。
    pub lottery_config: LotteryConfigStore,
}

impl SqliteStore {
    /// 按 `dsn` 打开 SQLite 存储并初始化表结构。
    ///
    /// `dsn` 由 [`SqliteConnectOptions`] 解析；连接选项启用
    /// `create_if_missing(true)`，因此目标数据库不存在时会尝试创建。连接成功后，本方法依次
    /// 执行 [`SCHEMA_SQL`]、为旧版 `groups` 表补充默认值为 `1` 的 `available` 列，
    /// 再创建仅覆盖 `available = 1` 行的索引。
    ///
    /// 返回后，`pool`、`messages` 与 `groups` 共享同一个连接池。解析 DSN、连接或任一
    /// SQL 执行失败时返回对应的 [`sqlx::Error`]。初始化过程未包裹在事务中，因此失败前已
    /// 创建的数据库文件、表、列或索引可能保留。
    ///
    /// 旧表补列采用“先检查列、再执行 `ALTER TABLE`”的两步流程，不具备并发安全性。同一
    /// 旧数据库若被多个调用方并发首次初始化，它们可能同时判断缺列，随后重复执行
    /// `ALTER TABLE`，其中一个初始化因列已存在而报错。调用方应串行完成旧库的首次初始化。
    pub async fn new(dsn: &str) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(dsn)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePool::connect_with(options).await?;
        sqlx::query(SCHEMA_SQL).execute(&pool).await?;
        migrate_groups_available(&pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_groups_available
             ON groups(available) WHERE available = 1",
        )
        .execute(&pool)
        .await?;
        migrate_messages_matched(&pool).await?;
        Ok(Self {
            pool: pool.clone(),
            messages: MessageStore::new(pool.clone()).await,
            groups: GroupStore::new(pool.clone()).await,
            key_pairs: UserKeyPairStore::new(pool.clone()),
            lottery_config: LotteryConfigStore::new(pool.clone()),
        })
    }
}

/// 检查旧版 `groups` 表，并在缺失时补充 `available` 列。
///
/// 检查与 `ALTER TABLE` 不是原子操作；同一旧库的首次迁移必须由调用方串行执行。
async fn migrate_groups_available(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('groups') WHERE name = 'available'",
    )
    .fetch_one(pool)
    .await?;
    if column_count == 0 {
        sqlx::query("ALTER TABLE groups ADD COLUMN available INTEGER NOT NULL DEFAULT 1")
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// 检查 `messages` 表，并在缺失时补充 `matched` 列。
async fn migrate_messages_matched(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'matched'",
    )
    .fetch_one(pool)
    .await?;
    if column_count == 0 {
        sqlx::query("ALTER TABLE messages ADD COLUMN matched INTEGER NOT NULL DEFAULT 0")
            .execute(pool)
            .await?;
    }
    Ok(())
}
