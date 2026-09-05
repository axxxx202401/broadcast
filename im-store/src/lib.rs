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

use prost::Message;
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
        migrate_messages_content_text(&pool).await?;
        migrate_lottery_config_issues(&pool).await?;
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

/// 检查 `messages` 表，并在缺失时补充 `content_text` 列。
///
/// 对于已有数据行（`content_text` 为空），尝试从 `raw_proto` 中重建 `GroupMessage`
/// 并提取明文文本；version == 0 的消息直接拷贝 `content`，加密消息无法解密则保留空字符串。
async fn migrate_messages_content_text(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'content_text'",
    )
    .fetch_one(pool)
    .await?;
    if column_count == 0 {
        sqlx::query("ALTER TABLE messages ADD COLUMN content_text TEXT NOT NULL DEFAULT ''")
            .execute(pool)
            .await?;
        return Ok(());
    }
    // 检查是否有未填充的行（content_text 为空且 raw_proto 非空）。
    let empty_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE content_text = '' AND raw_proto IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    if empty_count == 0 {
        return Ok(());
    }
    // 批量提取：对每行尝试从 raw_proto 提取明文。
    let rows: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT msg_id, raw_proto FROM messages WHERE content_text = ''")
            .fetch_all(pool)
            .await?;
    for (msg_id, raw_proto) in rows {
        if let Ok(msg) = im_proto::GroupMessage::decode(raw_proto.as_slice()) {
            let text = if msg.version == 0 {
                String::from_utf8_lossy(&msg.content).to_string()
            } else {
                // version > 0 的加密消息无法离线解密，保留空字符串。
                String::new()
            };
            if !text.is_empty() {
                sqlx::query("UPDATE messages SET content_text = ? WHERE msg_id = ?")
                    .bind(&text)
                    .bind(msg_id)
                    .execute(pool)
                    .await?;
            }
        }
    }
    Ok(())
}

/// 检查 `lottery_config` 表，并把旧 `current_issue INTEGER` 列迁移为 `current_issues TEXT`。
///
/// 旧列存单个期号；新列存 JSON 数组。迁移时会把旧值包进长度为 1 的数组，并删除旧列。
async fn migrate_lottery_config_issues(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let has_old: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('lottery_config') WHERE name = 'current_issue'",
    )
    .fetch_one(pool)
    .await?;
    let has_new: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('lottery_config') WHERE name = 'current_issues'",
    )
    .fetch_one(pool)
    .await?;

    // 全新库：`current_issues` 已被 SCHEMA_SQL 创建，无需操作。
    if has_new > 0 {
        return Ok(());
    }

    // 旧库：若存在 `current_issue`，先复制其值到新列，再删旧列。
    // SQLite 不支持在同一语句中 ADD + DROP，因此分两步执行。
    if has_old > 0 {
        sqlx::query(
            "ALTER TABLE lottery_config ADD COLUMN current_issues TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE lottery_config
             SET current_issues = json_array(current_issue)
             WHERE current_issue IS NOT NULL AND current_issue != 0",
        )
        .execute(pool)
        .await?;
        sqlx::query("ALTER TABLE lottery_config DROP COLUMN current_issue")
            .execute(pool)
            .await?;
    }
    Ok(())
}
