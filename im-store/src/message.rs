use sqlx::{Row, SqlitePool};

/// 单页最多可读取的消息数。
pub const MAX_MESSAGE_PAGE_LIMIT: usize = 200;

/// 消息保留天数。
pub const MESSAGE_RETENTION_DAYS: u64 = 7;
/// 每批次最大删除行数，与分页上限保持一致。
const CLEANUP_BATCH_SIZE: usize = 200;

/// 消息 keyset 分页游标。
///
/// 两个字段共同标识降序结果中的唯一边界；仅按 `send_time` 翻页会遗漏同一发送时间的
/// 消息，仅按 `msg_id` 则不符合消息时间排序。下一页只读取严格早于该复合键的行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageCursor {
    /// 本页最老消息的发送时间。
    pub send_time: i64,
    /// 本页最老消息的主键。
    pub msg_id: i64,
}

/// 准备写入 `messages` 表的一条群消息。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRecord {
    /// 消息主键，对应 `messages.msg_id`。
    pub msg_id: i64,
    /// 消息所属群组，对应 `messages.group_id`。
    pub group_id: i64,
    /// 发送者标识，对应 `messages.send_uid`。
    pub send_uid: i64,
    /// 消息类型值，对应 `messages.msg_type`。
    pub msg_type: i32,
    /// 消息内容字节，对应 `messages.content`。
    pub content: Vec<u8>,
    /// 发送时间值，对应 `messages.send_time`；其单位尚未由客户端契约验证。
    pub send_time: i64,
    /// 内容摘要文本，对应 `messages.content_md5`；存储层不校验其格式或内容。
    pub content_md5: String,
    /// 可选的原始协议字节，对应 `messages.raw_proto`。
    pub raw_proto: Option<Vec<u8>>,
    /// 解密后的明文文本（`version == 0` 时直接存原始字节；`version > 0` 时需解密后提取）。
    /// 用于消息匹配查询，避免对 `content` 做解密操作。
    pub content_text: String,
}

/// 从 `messages` 表读取的一行消息。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    /// 消息主键，对应 `messages.msg_id`。
    pub msg_id: i64,
    /// 消息所属群组，对应 `messages.group_id`。
    pub group_id: i64,
    /// 发送者标识，对应 `messages.send_uid`。
    pub send_uid: i64,
    /// 消息类型值，对应 `messages.msg_type`。
    pub msg_type: i32,
    /// 消息内容字节，对应 `messages.content`。
    pub content: Vec<u8>,
    /// 发送时间值，对应 `messages.send_time`；其单位尚未由客户端契约验证。
    pub send_time: i64,
    /// 内容摘要文本，对应 `messages.content_md5`。
    pub content_md5: String,
    /// 本存储写入该行时记录的 UTC Unix 时间戳，单位为毫秒。
    pub stored_at: i64,
    /// 可选的原始协议字节，对应 `messages.raw_proto`。
    pub raw_proto: Option<Vec<u8>>,
    /// 群组显示名称；群记录不存在时为空字符串。
    pub group_name: String,
    /// 是否匹配当前账号的开奖规则；`1` 为匹配，`0` 为不匹配。
    pub matched: i32,
    /// 解密后的明文文本，对应 `messages.content_text`。
    pub content_text: String,
}

/// 一页按时间倒序排列的消息。
#[derive(Debug, Clone)]
pub struct MessagePage {
    /// 当前页消息，按 `(send_time, msg_id)` 降序排列。
    pub messages: Vec<MessageRow>,
    /// 尚有更早消息时，指向当前页最老一条消息的下一页游标。
    pub next_cursor: Option<MessageCursor>,
    /// 是否仍存在严格早于 [`Self::next_cursor`] 的消息。
    pub has_more: bool,
}

/// 基于共享 SQLite 连接池的消息数据访问入口。
pub struct MessageStore {
    pool: SqlitePool,
}

impl MessageStore {
    /// 使用给定连接池创建消息数据访问入口。
    ///
    /// 本方法只保存连接池句柄，不连接数据库、不建表，也不执行 SQL。
    pub async fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 在单条事务中写入或更新一条消息，并返回传入的消息主键。
    ///
    /// 本方法委托 [`Self::insert_batch`]，主键冲突使用 `ON CONFLICT DO UPDATE` 原位更新，
    /// 不采用 `INSERT OR REPLACE` 的删除再插入语义。
    ///
    /// `stored_at` 不取自 [`MessageRecord`]，而是在每次写入时记录当前 UTC Unix
    /// 毫秒时间戳；`send_time` 则原样写入，其单位尚未由客户端契约验证。
    ///
    /// SQL 执行失败时返回 [`sqlx::Error`]。
    pub async fn insert(&self, record: &MessageRecord) -> sqlx::Result<i64> {
        self.insert_batch(std::slice::from_ref(record)).await?;
        Ok(record.msg_id)
    }

    /// 在单个事务内写入或更新一批消息。
    ///
    /// 同一批使用相同 `stored_at`。主键冲突通过 `ON CONFLICT DO UPDATE` 原位更新，
    /// 避免 `INSERT OR REPLACE` 的删除再插入语义。任一行失败会回滚整批；空批次不访问
    /// 数据库并直接成功。
    pub async fn insert_batch(&self, records: &[MessageRecord]) -> sqlx::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let stored_at = chrono::Utc::now().timestamp_millis();
        let mut transaction = self.pool.begin().await?;
        for record in records {
            sqlx::query(
                r#"INSERT INTO messages
                   (msg_id, group_id, send_uid, msg_type, content, send_time, content_md5, stored_at, raw_proto, matched, content_text)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
                   ON CONFLICT(msg_id) DO UPDATE SET
                     group_id = excluded.group_id,
                     send_uid = excluded.send_uid,
                     msg_type = excluded.msg_type,
                     content = excluded.content,
                     send_time = excluded.send_time,
                     content_md5 = excluded.content_md5,
                     stored_at = excluded.stored_at,
                     raw_proto = excluded.raw_proto,
                     content_text = excluded.content_text"#,
            )
            .bind(record.msg_id)
            .bind(record.group_id)
            .bind(record.send_uid)
            .bind(record.msg_type)
            .bind(&record.content)
            .bind(record.send_time)
            .bind(&record.content_md5)
            .bind(stored_at)
            .bind(&record.raw_proto)
            .bind(&record.content_text)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// 使用复合 keyset 游标读取指定群组的消息。
    ///
    /// 无游标时从最新消息开始；有游标时只读取 `(send_time, msg_id)` 严格更小的行。
    /// 查询固定多读一行判断是否还有更早记录，不使用 `OFFSET`。`limit` 必须位于
    /// `1..=`[`MAX_MESSAGE_PAGE_LIMIT`]；参数越界或 SQL 执行失败时返回对应错误。
    /// `matched_only` 为 `true` 时只返回 `matched = 1` 的消息。
    pub async fn get_by_group(
        &self,
        group_id: i64,
        limit: usize,
        cursor: Option<MessageCursor>,
        matched_only: bool,
    ) -> sqlx::Result<MessagePage> {
        let fetch_limit = checked_fetch_limit(limit)?;
        let rows = if let Some(cursor) = cursor {
            sqlx::query(
                r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                          m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name, m.matched, m.content_text
                   FROM messages m
                   LEFT JOIN groups g ON g.group_id = m.group_id
                   WHERE m.group_id = ?
                     AND (m.send_time < ? OR (m.send_time = ? AND m.msg_id < ?))
                     AND (m.matched = 1 OR NOT ?)
                   ORDER BY m.send_time DESC, m.msg_id DESC
                   LIMIT ?"#,
            )
            .bind(group_id)
            .bind(cursor.send_time)
            .bind(cursor.send_time)
            .bind(cursor.msg_id)
            .bind(matched_only)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                          m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name, m.matched, m.content_text
                   FROM messages m
                   LEFT JOIN groups g ON g.group_id = m.group_id
                   WHERE m.group_id = ?
                     AND (m.matched = 1 OR NOT ?)
                   ORDER BY m.send_time DESC, m.msg_id DESC
                   LIMIT ?"#,
            )
            .bind(group_id)
            .bind(matched_only)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(message_page(rows, limit))
    }

    /// 分页读取当前可用且仍受监控群组的最近消息，并关联群组显示名称。
    ///
    /// 结果按发送时间和消息 ID 降序排列；没有群记录、已不可用或已关闭监控的消息
    /// 不进入全量监控视图。游标语义和页长边界与 [`Self::get_by_group`] 相同。
    /// `matched_only` 为 `true` 时只返回 `matched = 1` 的消息。
    pub async fn get_recent(
        &self,
        limit: usize,
        cursor: Option<MessageCursor>,
        matched_only: bool,
    ) -> sqlx::Result<MessagePage> {
        let fetch_limit = checked_fetch_limit(limit)?;
        let rows = if let Some(cursor) = cursor {
            sqlx::query(
                r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                          m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name, m.matched, m.content_text
                   FROM messages m
                   JOIN groups g ON g.group_id = m.group_id
                   WHERE g.monitored = 1 AND g.available = 1
                     AND (m.send_time < ? OR (m.send_time = ? AND m.msg_id < ?))
                     AND (m.matched = 1 OR NOT ?)
                   ORDER BY m.send_time DESC, m.msg_id DESC
                   LIMIT ?"#,
            )
            .bind(cursor.send_time)
            .bind(cursor.send_time)
            .bind(cursor.msg_id)
            .bind(matched_only)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                          m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name, m.matched, m.content_text
                   FROM messages m
                   JOIN groups g ON g.group_id = m.group_id
                   WHERE g.monitored = 1 AND g.available = 1
                     AND (m.matched = 1 OR NOT ?)
                   ORDER BY m.send_time DESC, m.msg_id DESC
                   LIMIT ?"#,
            )
            .bind(matched_only)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(message_page(rows, limit))
    }

    /// 按消息 ID 读取单条消息及其完整原始 Protobuf。
    ///
    /// 附件下载流程使用原始 Protobuf 重新取得 `attachment_key`、版本及媒体 URL。
    /// 消息不存在时返回 `Ok(None)`；群记录不存在不会阻止消息返回。
    pub async fn get_by_id(&self, msg_id: i64) -> sqlx::Result<Option<MessageRow>> {
        let row = sqlx::query(
            r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                      m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name, m.matched, m.content_text
               FROM messages m
               LEFT JOIN groups g ON g.group_id = m.group_id
               WHERE m.msg_id = ?"#,
        )
        .bind(msg_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| MessageRow {
            msg_id: row.get("msg_id"),
            group_id: row.get("group_id"),
            send_uid: row.get("send_uid"),
            msg_type: row.get("msg_type"),
            content: row.get("content"),
            send_time: row.get("send_time"),
            content_md5: row.get("content_md5"),
            stored_at: row.get("stored_at"),
            raw_proto: row.get("raw_proto"),
            group_name: row.get("group_name"),
            matched: row.get("matched"),
            content_text: row.get("content_text"),
        }))
    }

    /// 删除所有 `send_time` 严格早于 `keep_since` 的消息。
    ///
    /// 采用分批删除策略：每批执行一次 `DELETE ... LIMIT BATCH_SIZE`，每批在独立语句
    /// 中隐式提交，批次之间互不影响。当一批删除行数少于 `BATCH_SIZE` 时表示已全部
    /// 清理完毕。
    ///
    /// 返回实际删除的行数。SQL 执行失败时返回 [`sqlx::Error`]。
    pub async fn cleanup_old_messages(&self, keep_since: i64) -> sqlx::Result<usize> {
        let mut total_deleted = 0usize;
        loop {
            let result = sqlx::query(
                "DELETE FROM messages \
                 WHERE send_time < ? \
                 AND msg_id IN (\
                     SELECT msg_id FROM messages \
                     WHERE send_time < ? \
                     ORDER BY msg_id DESC \
                     LIMIT ?\
                 )",
            )
            .bind(keep_since)
            .bind(keep_since)
            .bind(CLEANUP_BATCH_SIZE as i64)
            .execute(&self.pool)
            .await?;
            let rows = result.rows_affected() as usize;
            total_deleted += rows;
            if rows < CLEANUP_BATCH_SIZE {
                break;
            }
        }
        Ok(total_deleted)
    }

}

/// 校验业务页长，并返回供 SQL 多读一行的绑定值。
fn checked_fetch_limit(limit: usize) -> sqlx::Result<i64> {
    if !(1..=MAX_MESSAGE_PAGE_LIMIT).contains(&limit) {
        return Err(sqlx::Error::Protocol(format!(
            "message limit must be between 1 and {MAX_MESSAGE_PAGE_LIMIT}"
        )));
    }
    i64::try_from(limit + 1)
        .map_err(|_| sqlx::Error::Protocol("message limit exceeds i64".to_string()))
}

/// 将 SQL 的 `limit + 1` 行裁成稳定页面，并从实际返回页尾构造下一页游标。
fn message_page(rows: Vec<sqlx::sqlite::SqliteRow>, limit: usize) -> MessagePage {
    let has_more = rows.len() > limit;
    let mut messages = rows
        .into_iter()
        .take(limit)
        .map(|row| MessageRow {
            msg_id: row.get("msg_id"),
            group_id: row.get("group_id"),
            send_uid: row.get("send_uid"),
            msg_type: row.get("msg_type"),
            content: row.get("content"),
            send_time: row.get("send_time"),
            content_md5: row.get("content_md5"),
            stored_at: row.get("stored_at"),
            raw_proto: row.get("raw_proto"),
            group_name: row.get("group_name"),
            matched: row.get("matched"),
            content_text: row.get("content_text"),
        })
        .collect::<Vec<_>>();
    let next_cursor = has_more && !messages.is_empty();
    MessagePage {
        next_cursor: next_cursor.then(|| {
            let oldest = messages.last().expect("non-empty page checked above");
            MessageCursor {
                send_time: oldest.send_time,
                msg_id: oldest.msg_id,
            }
        }),
        has_more,
        messages: std::mem::take(&mut messages),
    }
}
