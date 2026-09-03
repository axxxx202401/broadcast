//! 当前账号 App 密钥对的本地持久化。

use sqlx::{Row, SqlitePool};

/// 客户端自行生成并由 im-biz 登记公钥的 App 密钥对。
///
/// `private_key` 仅供 Rust 密钥解包流程读取，不应写入日志或跨 IPC 返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserKeyPairRecord {
    /// 账号 UID。
    pub uid: i64,
    /// 服务端登记后返回的密钥版本。
    pub key_version: i32,
    /// X25519 公钥 HEX。
    pub public_key: String,
    /// X25519 私钥 HEX。
    pub private_key: String,
}

/// 基于 SQLite 的当前账号密钥对访问入口。
pub struct UserKeyPairStore {
    pool: SqlitePool,
}

impl UserKeyPairStore {
    /// 使用共享连接池创建访问入口，不执行 SQL。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 保存一个密钥版本；相同账号和版本会被完整替换。
    pub async fn set(&self, record: &UserKeyPairRecord) -> sqlx::Result<()> {
        sqlx::query(
            r#"INSERT OR REPLACE INTO user_key_pairs
               (uid, key_version, public_key, private_key, updated_at)
               VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(record.uid)
        .bind(record.key_version)
        .bind(&record.public_key)
        .bind(&record.private_key)
        .bind(chrono::Utc::now().timestamp_millis())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 读取指定账号版本号最大的本地密钥对。
    pub async fn get_latest(&self, uid: i64) -> sqlx::Result<Option<UserKeyPairRecord>> {
        let row = sqlx::query(
            r#"SELECT uid, key_version, public_key, private_key
               FROM user_key_pairs
               WHERE uid = ?
               ORDER BY key_version DESC
               LIMIT 1"#,
        )
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| UserKeyPairRecord {
            uid: row.get("uid"),
            key_version: row.get("key_version"),
            public_key: row.get("public_key"),
            private_key: row.get("private_key"),
        }))
    }
}
