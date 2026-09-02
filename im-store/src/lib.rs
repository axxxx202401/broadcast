pub mod schema;
pub mod message;
pub mod group;

#[cfg(test)]
mod tests;

use sqlx::SqlitePool;
use schema::SCHEMA_SQL;

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
        let pool = SqlitePool::connect(dsn).await?;
        sqlx::query(SCHEMA_SQL).execute(&pool).await?;
        Ok(Self {
            pool: pool.clone(),
            messages: MessageStore::new(pool.clone()).await,
            groups: GroupStore::new(pool.clone()).await,
        })
    }
}
