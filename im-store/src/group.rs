use sqlx::{SqlitePool, Row};

/// A row from the groups table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupRow {
    pub group_id: i64,
    pub name: String,
    pub pic: String,
    pub host_id: Option<i64>,
    pub member_count: i64,
    pub created_at: i64,
    pub monitored: i32,
    pub updated_at: i64,
}

pub struct GroupStore {
    pool: SqlitePool,
}

impl GroupStore {
    pub async fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Upsert a group into the database.
    pub async fn insert_or_update(&self, group: &GroupRow) -> sqlx::Result<()> {
        sqlx::query(
            r#"INSERT INTO groups (group_id, name, pic, host_id, member_count, created_at, monitored, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(group_id) DO UPDATE SET
                   name = excluded.name,
                   pic = excluded.pic,
                   member_count = excluded.member_count,
                   updated_at = excluded.updated_at"#,
        )
        .bind(group.group_id)
        .bind(&group.name)
        .bind(&group.pic)
        .bind(group.host_id)
        .bind(group.member_count)
        .bind(group.created_at)
        .bind(group.monitored)
        .bind(group.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return all groups that are currently monitored.
    pub async fn list_monitored(&self) -> sqlx::Result<Vec<GroupRow>> {
        let rows = sqlx::query(
            "SELECT group_id, name, pic, host_id, member_count, created_at, monitored, updated_at
             FROM groups WHERE monitored = 1 ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(GroupRow {
                group_id: row.get("group_id"),
                name: row.get("name"),
                pic: row.get("pic"),
                host_id: row.get("host_id"),
                member_count: row.get("member_count"),
                created_at: row.get("created_at"),
                monitored: row.get("monitored"),
                updated_at: row.get("updated_at"),
            });
        }
        Ok(result)
    }

    /// Toggle the monitored flag for a group.
    pub async fn toggle_monitored(&self, group_id: i64, monitored: bool) -> sqlx::Result<()> {
        sqlx::query("UPDATE groups SET monitored = ? WHERE group_id = ?")
            .bind(monitored as i32)
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
