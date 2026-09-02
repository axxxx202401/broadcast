use sqlx::{SqlitePool, Row};

/// A record representing a group message to be persisted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRecord {
    pub msg_id: i64,
    pub group_id: i64,
    pub send_uid: i64,
    pub msg_type: i32,
    pub content: Vec<u8>,
    pub send_time: i64,
    pub content_md5: String,
}

/// A row returned from the messages table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub msg_id: i64,
    pub group_id: i64,
    pub send_uid: i64,
    pub msg_type: i32,
    pub content: Vec<u8>,
    pub send_time: i64,
    pub content_md5: String,
    pub stored_at: i64,
    pub raw_proto: Option<Vec<u8>>,
}

pub struct MessageStore {
    pool: SqlitePool,
}

impl MessageStore {
    pub async fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, record: &MessageRecord) -> sqlx::Result<i64> {
        let stored_at = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            r#"INSERT OR REPLACE INTO messages
               (msg_id, group_id, send_uid, msg_type, content, send_time, content_md5, stored_at, raw_proto)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&record.msg_id)
        .bind(&record.group_id)
        .bind(&record.send_uid)
        .bind(&record.msg_type)
        .bind(&record.content)
        .bind(&record.send_time)
        .bind(&record.content_md5)
        .bind(stored_at)
        .bind(None::<Vec<u8>>)
        .execute(&self.pool)
        .await?;
        Ok(record.msg_id)
    }

    pub async fn get_by_group(
        &self,
        group_id: i64,
        limit: usize,
        offset: usize,
    ) -> sqlx::Result<Vec<MessageRow>> {
        let rows = sqlx::query(
            r#"SELECT msg_id, group_id, send_uid, msg_type, content, send_time,
                      content_md5, stored_at, raw_proto
               FROM messages
               WHERE group_id = ?
               ORDER BY send_time DESC
               LIMIT ? OFFSET ?"#,
        )
        .bind(group_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(MessageRow {
                msg_id: row.get("msg_id"),
                group_id: row.get("group_id"),
                send_uid: row.get("send_uid"),
                msg_type: row.get("msg_type"),
                content: row.get("content"),
                send_time: row.get("send_time"),
                content_md5: row.get("content_md5"),
                stored_at: row.get("stored_at"),
                raw_proto: row.get("raw_proto"),
            });
        }
        Ok(result)
    }
}
