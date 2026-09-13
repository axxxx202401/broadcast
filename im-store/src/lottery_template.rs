use sqlx::{Row, SqlitePool};

/// 广播模板的默认内容；DB 无记录时用作降级值。
pub const DEFAULT_TEMPLATE: &str = "加拿大 PC 第${preDrawIssue}期开奖结果：\n\
     ${preDrawCode}=${sumNum}  ${sumBigSmall}${sumSingleDouble}${patternDesc}\n\
     近10期：${lastTenDraws}\n\
     顶赔对赌\n\
     大小单双：2.17\n小双大单：4.32\n大双小单：4.76\n\n\
     大将军CU交易1群 @bkkn7mqkn0\n\
     大将军CU交易2群 @93158hello\n\
     大将军CU交易3群 @az8t88eeqg\n\
     大将军上押担保频道 @flyin3037s\n\
     大将军担保官方网站 https://djidb.com\n\n\
     ——团队担保信至上服务至上——";

/// 单条广播模板记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LotteryTemplateRow {
    /// 模板内容；含占位符如 `${preDrawIssue}`。
    pub template: String,
    /// 广播功能是否启用。
    pub enabled: bool,
    /// 最后完成广播处理的期号；`None` 表示尚未建立首次启用基线。
    pub last_broadcast_issue: Option<i64>,
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

    /// 读取当前账号的模板；未配置时插入默认行并返回。
    ///
    /// 首次登录时自动写库，确保后续读取稳定，也避免重复 INSERT（ON CONFLICT 静默忽略）。
    pub async fn get(&self, _uid: i64) -> sqlx::Result<LotteryTemplateRow> {
        let row = sqlx::query(
            "SELECT template, enabled, last_broadcast_issue, updated_at
             FROM lottery_message_templates WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(LotteryTemplateRow {
                template: row.get("template"),
                enabled: row.get::<i32, _>("enabled") != 0,
                last_broadcast_issue: row.get("last_broadcast_issue"),
                updated_at: row.get("updated_at"),
            }),
            // DB 无记录时立即写入默认行，保证后续读取不经过此分支。
            None => {
                let now = chrono::Utc::now().timestamp_millis();
                sqlx::query(
                    "INSERT INTO lottery_message_templates (id, template, enabled, updated_at)
                     VALUES (1, ?, 0, ?)",
                )
                .bind(DEFAULT_TEMPLATE)
                .bind(now)
                .execute(&self.pool)
                .await?;
                Ok(LotteryTemplateRow {
                    template: DEFAULT_TEMPLATE.to_string(),
                    enabled: false,
                    last_broadcast_issue: None,
                    updated_at: now,
                })
            }
        }
    }

    /// 插入或更新当前账号的模板。
    pub async fn upsert(&self, row: &LotteryTemplateRow, _uid: i64) -> sqlx::Result<()> {
        sqlx::query(
            r#"INSERT INTO lottery_message_templates
               (id, template, enabled, last_broadcast_issue, updated_at)
               VALUES (1, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   template = excluded.template,
                   enabled = excluded.enabled,
                   last_broadcast_issue = excluded.last_broadcast_issue,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&row.template)
        .bind(if row.enabled { 1 } else { 0 })
        .bind(row.last_broadcast_issue)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 独立更新广播期号游标，不修改模板内容、启用状态或消息匹配配置。
    pub async fn set_last_broadcast_issue(&self, issue: i64, _uid: i64) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE lottery_message_templates
             SET last_broadcast_issue = ?, updated_at = ?
             WHERE id = 1",
        )
        .bind(issue)
        .bind(chrono::Utc::now().timestamp_millis())
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
                last_broadcast_issue INTEGER,
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
            last_broadcast_issue: Some(20260914001),
            updated_at: 1234567890,
        };
        store.upsert(&row, 1).await.unwrap();

        let fetched = store.get(1).await.unwrap();
        assert_eq!(fetched.template, "测试模板");
        assert!(fetched.enabled);
        assert_eq!(fetched.last_broadcast_issue, Some(20260914001));
        assert_eq!(fetched.updated_at, 1234567890);
    }

    #[tokio::test]
    async fn fetch_default_when_not_inserted() {
        let pool = setup_pool().await;
        let store = LotteryTemplateStore::new(pool);

        let fetched = store.get(42).await.unwrap();
        // 无记录时立即 INSERT 默认行，返回默认模板且 enabled=false，updated_at>0
        assert_eq!(fetched.template, DEFAULT_TEMPLATE);
        assert!(!fetched.enabled);
        assert_eq!(fetched.last_broadcast_issue, None);
        assert!(fetched.updated_at > 0);
    }

    #[tokio::test]
    async fn second_get_returns_already_inserted_default() {
        let pool = setup_pool().await;
        let store = LotteryTemplateStore::new(pool);

        let first = store.get(1).await.unwrap();
        let second = store.get(1).await.unwrap();
        // 第二次调用不应再次 INSERT，内容与第一次一致
        assert_eq!(first.template, second.template);
        assert_eq!(first.enabled, second.enabled);
        assert_eq!(first.updated_at, second.updated_at);
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
                    last_broadcast_issue: None,
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
                    last_broadcast_issue: Some(20260914002),
                    updated_at: 200,
                },
                1,
            )
            .await
            .unwrap();

        let fetched = store.get(1).await.unwrap();
        assert_eq!(fetched.template, "second");
        assert!(fetched.enabled);
        assert_eq!(fetched.last_broadcast_issue, Some(20260914002));
        assert_eq!(fetched.updated_at, 200);
    }

    #[tokio::test]
    async fn updates_broadcast_cursor_independently() {
        let pool = setup_pool().await;
        let store = LotteryTemplateStore::new(pool);

        store.get(1).await.unwrap();
        store
            .set_last_broadcast_issue(20260914003, 1)
            .await
            .unwrap();

        assert_eq!(
            store.get(1).await.unwrap().last_broadcast_issue,
            Some(20260914003)
        );
    }
}
