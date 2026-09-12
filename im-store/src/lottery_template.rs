use sqlx::{Row, SqlitePool};

/// 单条广播模板记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LotteryTemplateRow {
    /// 模板内容；含占位符如 `${preDrawIssue}`。
    pub template: String,
    /// 广播功能是否启用。
    pub enabled: bool,
    /// 最后更新时间（UTC Unix 毫秒）。
    pub updated_at: i64,
}

/// 广播模板数据访问入口。
pub struct LotteryTemplateStore {
    pool: SqlitePool,
}

impl LotteryTemplateStore {
    /// 构造广播模板数据访问入口。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 读取当前账号的模板；未配置时返回默认行（enabled=false）。
    pub async fn get(&self, _uid: i64) -> sqlx::Result<LotteryTemplateRow> {
        let row = sqlx::query(
            "SELECT template, enabled, updated_at FROM lottery_message_templates WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(match row {
            Some(row) => LotteryTemplateRow {
                template: row.get("template"),
                enabled: row.get::<i32, _>("enabled") != 0,
                updated_at: row.get("updated_at"),
            },
            None => LotteryTemplateRow {
                template: String::new(),
                enabled: false,
                updated_at: 0,
            },
        })
    }

    /// 插入或更新当前账号的模板。
    pub async fn upsert(&self, row: &LotteryTemplateRow, _uid: i64) -> sqlx::Result<()> {
        sqlx::query(
            r#"INSERT INTO lottery_message_templates (id, template, enabled, updated_at)
               VALUES (1, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   template = excluded.template,
                   enabled = excluded.enabled,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&row.template)
        .bind(if row.enabled { 1 } else { 0 })
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePool;

    async fn test_pool() -> SqlitePool {
        SqlitePool::connect(":memory:").await.unwrap()
    }

    async fn setup_pool() -> SqlitePool {
        let pool = test_pool().await;
        sqlx::query(
            "CREATE TABLE lottery_message_templates (
                id          INTEGER PRIMARY KEY CHECK(id = 1),
                template    TEXT    NOT NULL DEFAULT '',
                enabled     INTEGER NOT NULL DEFAULT 0,
                updated_at  INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_and_fetch_template() {
        let pool = setup_pool().await;
        let store = LotteryTemplateStore::new(pool);
        let row = LotteryTemplateRow {
            template: "测试模板".to_string(),
            enabled: true,
            updated_at: 1234567890,
        };
        store.upsert(&row, 1).await.unwrap();

        let fetched = store.get(1).await.unwrap();
        assert_eq!(fetched.template, "测试模板");
        assert!(fetched.enabled);
        assert_eq!(fetched.updated_at, 1234567890);
    }

    #[tokio::test]
    async fn fetch_default_when_not_inserted() {
        let pool = setup_pool().await;
        let store = LotteryTemplateStore::new(pool);

        let fetched = store.get(42).await.unwrap();
        assert!(fetched.template.is_empty());
        assert!(!fetched.enabled);
        assert_eq!(fetched.updated_at, 0);
    }

    #[tokio::test]
    async fn upsert_overwrites_existing_template() {
        let pool = setup_pool().await;
        let store = LotteryTemplateStore::new(pool);

        store
            .upsert(
                &LotteryTemplateRow {
                    template: "first".to_string(),
                    enabled: false,
                    updated_at: 100,
                },
                1,
            )
            .await
            .unwrap();

        store
            .upsert(
                &LotteryTemplateRow {
                    template: "second".to_string(),
                    enabled: true,
                    updated_at: 200,
                },
                1,
            )
            .await
            .unwrap();

        let fetched = store.get(1).await.unwrap();
        assert_eq!(fetched.template, "second");
        assert!(fetched.enabled);
        assert_eq!(fetched.updated_at, 200);
    }
}
