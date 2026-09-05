use sqlx::{Row, SqlitePool};

/// 单个账号的开奖配置。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LotteryConfigRow {
    /// 所属账号 UID，与 `lottery_config.uid` 对应。
    pub uid: i64,
    /// 开奖历史 API URL，例如 `https://go124.com/api/hash/get28HistoryList/10091`。
    pub api_url: String,
    /// 当前关注的期号列表（从 API 历史获取的所有期号）；空列表表示尚未设置。
    #[serde(default)]
    pub current_issues: Vec<i64>,
    /// 最后更新时间（UTC Unix 毫秒）。
    pub updated_at: i64,
}

/// 基于共享 SQLite 连接池的开奖配置数据访问入口。
pub struct LotteryConfigStore {
    pool: SqlitePool,
}

impl LotteryConfigStore {
    /// 使用给定连接池创建开奖配置访问入口。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 读取指定账号的开奖配置；未设置时返回默认行（`current_issues = []`）。
    pub async fn get(&self, uid: i64) -> sqlx::Result<LotteryConfigRow> {
        let row = sqlx::query(
            "SELECT uid, api_url, current_issues, updated_at FROM lottery_config WHERE uid = ?",
        )
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(match row {
            Some(row) => LotteryConfigRow {
                uid: row.get("uid"),
                api_url: row.get("api_url"),
                current_issues: serde_json::from_str(row.get::<&str, _>("current_issues"))
                    .unwrap_or_default(),
                updated_at: row.get("updated_at"),
            },
            None => LotteryConfigRow {
                uid,
                api_url: String::new(),
                current_issues: Vec::new(),
                updated_at: 0,
            },
        })
    }

    /// 插入或更新指定账号的开奖配置。
    pub async fn upsert(&self, config: &LotteryConfigRow) -> sqlx::Result<()> {
        let current_issues_json = serde_json::to_string(&config.current_issues)
            .map_err(|e| sqlx::Error::Decode(format!("序列化期号列表失败: {e}").into()))?;
        sqlx::query(
            r#"INSERT INTO lottery_config (uid, api_url, current_issues, updated_at)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(uid) DO UPDATE SET
                   api_url = excluded.api_url,
                   current_issues = excluded.current_issues,
                   updated_at = excluded.updated_at"#,
        )
        .bind(config.uid)
        .bind(&config.api_url)
        .bind(&current_issues_json)
        .bind(config.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
