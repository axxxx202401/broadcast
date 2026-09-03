use sqlx::{Row, SqlitePool};

/// 单页最多可读取的消息数。
pub const MAX_MESSAGE_PAGE_LIMIT: usize = 200;
/// 分页查询允许的最大偏移量。
pub const MAX_MESSAGE_PAGE_OFFSET: usize = 1_000_000;

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

    /// 写入或替换一条消息，并返回传入的消息主键。
    ///
    /// 写入使用 SQLite `INSERT OR REPLACE`：发生主键冲突时，SQLite 先删除冲突行，再插入
    /// 新行，而不是原位执行 `UPDATE`。因此相关触发器与外键按删除、插入语义处理；其中
    /// 删除触发器是否执行还受 SQLite `recursive_triggers` 设置影响。
    ///
    /// `stored_at` 不取自 [`MessageRecord`]，而是在每次写入时记录当前 UTC Unix
    /// 毫秒时间戳；`send_time` 则原样写入，其单位尚未由客户端契约验证。
    ///
    /// SQL 执行失败时返回 [`sqlx::Error`]。
    pub async fn insert(&self, record: &MessageRecord) -> sqlx::Result<i64> {
        let stored_at = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            r#"INSERT OR REPLACE INTO messages
               (msg_id, group_id, send_uid, msg_type, content, send_time, content_md5, stored_at, raw_proto)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
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
        .execute(&self.pool)
        .await?;
        Ok(record.msg_id)
    }

    /// 分页读取指定群组的消息。
    ///
    /// 结果按 `send_time DESC` 排列；发送时间相同时，SQL 未指定次级顺序。
    /// `limit` 必须位于 `1..=`[`MAX_MESSAGE_PAGE_LIMIT`]，`offset` 不得超过
    /// [`MAX_MESSAGE_PAGE_OFFSET`]。代码随后将二者转换为 SQLite 绑定使用的 `i64`；由于
    /// 当前两个上限都可由 `i64` 表示，通过前述检查后，转换失败不是实际的用户输入错误
    /// 路径，转换检查仅作防御性保留。参数越界时返回 [`sqlx::Error::Protocol`]，查询失败
    /// 时返回对应的 SQLx 错误；没有匹配消息时返回空列表。
    pub async fn get_by_group(
        &self,
        group_id: i64,
        limit: usize,
        offset: usize,
    ) -> sqlx::Result<Vec<MessageRow>> {
        if !(1..=MAX_MESSAGE_PAGE_LIMIT).contains(&limit) {
            return Err(sqlx::Error::Protocol(format!(
                "message limit must be between 1 and {MAX_MESSAGE_PAGE_LIMIT}"
            )));
        }
        if offset > MAX_MESSAGE_PAGE_OFFSET {
            return Err(sqlx::Error::Protocol(format!(
                "message offset exceeds maximum {MAX_MESSAGE_PAGE_OFFSET}"
            )));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| sqlx::Error::Protocol("message limit exceeds i64".to_string()))?;
        let offset = i64::try_from(offset)
            .map_err(|_| sqlx::Error::Protocol("message offset exceeds i64".to_string()))?;
        let rows = sqlx::query(
            r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                      m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name
               FROM messages m
               LEFT JOIN groups g ON g.group_id = m.group_id
               WHERE m.group_id = ?
               ORDER BY m.send_time DESC, m.msg_id DESC
               LIMIT ? OFFSET ?"#,
        )
        .bind(group_id)
        .bind(limit)
        .bind(offset)
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
                group_name: row.get("group_name"),
            });
        }
        Ok(result)
    }

    /// 分页读取当前可用且仍受监控群组的最近消息，并关联群组显示名称。
    ///
    /// 结果按发送时间和消息 ID 降序排列；没有群记录、已不可用或已关闭监控的消息
    /// 不进入全量监控视图。分页边界与 [`Self::get_by_group`] 相同。
    pub async fn get_recent(&self, limit: usize, offset: usize) -> sqlx::Result<Vec<MessageRow>> {
        if !(1..=MAX_MESSAGE_PAGE_LIMIT).contains(&limit) {
            return Err(sqlx::Error::Protocol(format!(
                "message limit must be between 1 and {MAX_MESSAGE_PAGE_LIMIT}"
            )));
        }
        if offset > MAX_MESSAGE_PAGE_OFFSET {
            return Err(sqlx::Error::Protocol(format!(
                "message offset exceeds maximum {MAX_MESSAGE_PAGE_OFFSET}"
            )));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| sqlx::Error::Protocol("message limit exceeds i64".to_string()))?;
        let offset = i64::try_from(offset)
            .map_err(|_| sqlx::Error::Protocol("message offset exceeds i64".to_string()))?;
        let rows = sqlx::query(
            r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                      m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name
               FROM messages m
               JOIN groups g ON g.group_id = m.group_id
               WHERE g.monitored = 1 AND g.available = 1
               ORDER BY m.send_time DESC, m.msg_id DESC
               LIMIT ? OFFSET ?"#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
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
            })
            .collect())
    }

    /// 按消息 ID 读取单条消息及其完整原始 Protobuf。
    ///
    /// 附件下载流程使用原始 Protobuf 重新取得 `attachment_key`、版本及媒体 URL。
    /// 消息不存在时返回 `Ok(None)`；群记录不存在不会阻止消息返回。
    pub async fn get_by_id(&self, msg_id: i64) -> sqlx::Result<Option<MessageRow>> {
        let row = sqlx::query(
            r#"SELECT m.msg_id, m.group_id, m.send_uid, m.msg_type, m.content, m.send_time,
                      m.content_md5, m.stored_at, m.raw_proto, COALESCE(g.name, '') AS group_name
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
        }))
    }
}
