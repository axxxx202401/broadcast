pub mod group;
pub mod message;
pub mod schema;

#[cfg(test)]
mod tests;

use schema::SCHEMA_SQL;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::group::GroupStore;
use crate::message::MessageStore;

/// High-level handle that owns the connection pool and exposes
/// [MessageStore] and [GroupStore] sub-accessors.
pub struct SqliteStore {
    pub pool: SqlitePool,
    pub messages: MessageStore,
    pub groups: GroupStore,
}

impl SqliteStore {
    /// Create a new store, creating all tables if they do not exist.
    pub async fn new(dsn: &str) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(dsn)?.create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await?;
        sqlx::query(SCHEMA_SQL).execute(&pool).await?;
        migrate_groups_available(&pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_groups_available
             ON groups(available) WHERE available = 1",
        )
        .execute(&pool)
        .await?;
        Ok(Self {
            pool: pool.clone(),
            messages: MessageStore::new(pool.clone()).await,
            groups: GroupStore::new(pool.clone()).await,
        })
    }
}

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
