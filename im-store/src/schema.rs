//! `im-store` 使用的 SQLite 表结构。

/// 创建群组表、消息表及查询索引的 SQL。
///
/// `groups.available` 是远端快照中的软可见标记：`1` 表示当前可见，`0` 表示保留行但不在
/// 当前群组列表中。`groups.monitored` 保存用户的监控选择；部分索引
/// `idx_groups_monitored` 只收录值为 `1` 的行，用于缩小监控群组查询的索引范围。
///
/// `messages.raw_proto` 可为空，用于保存原始协议字节；`idx_messages_group_time` 按
/// `(group_id, send_time)` 建索引，对应按群组筛选并按发送时间读取消息的查询。
///
/// 本常量只声明初始结构；旧版 `groups` 表缺少 `available` 列时，由
/// [`crate::SqliteStore::new`] 另行迁移，并创建当前可见群组的部分索引。
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS groups (
    group_id    INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    pic         TEXT DEFAULT '',
    host_id     INTEGER,
    member_count INTEGER DEFAULT 0,
    created_at  INTEGER NOT NULL,
    monitored   INTEGER NOT NULL DEFAULT 1,
    available   INTEGER NOT NULL DEFAULT 1,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    msg_id      INTEGER PRIMARY KEY,
    group_id    INTEGER NOT NULL,
    send_uid    INTEGER NOT NULL,
    msg_type    INTEGER NOT NULL,
    content     BLOB NOT NULL,
    send_time   INTEGER NOT NULL,
    content_md5 TEXT DEFAULT '',
    stored_at   INTEGER NOT NULL,
    raw_proto   BLOB,
    matched     INTEGER NOT NULL DEFAULT 0,
    content_text TEXT DEFAULT ''
);

CREATE TABLE IF NOT EXISTS user_key_pairs (
    uid          INTEGER NOT NULL,
    key_version  INTEGER NOT NULL,
    public_key   TEXT NOT NULL,
    private_key   TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (uid, key_version)
);

CREATE TABLE IF NOT EXISTS lottery_config (
    uid            INTEGER NOT NULL PRIMARY KEY,
    api_url        TEXT    NOT NULL DEFAULT '',
    current_issues TEXT    NOT NULL DEFAULT '[]',
    updated_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messages_group_time ON messages(group_id, send_time);
CREATE INDEX IF NOT EXISTS idx_messages_time ON messages(send_time DESC, msg_id DESC);
CREATE INDEX IF NOT EXISTS idx_groups_monitored ON groups(monitored) WHERE monitored = 1;
"#;
