/// SQLite schema for the im-store layer.
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS groups (
    group_id    INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    pic         TEXT DEFAULT '',
    host_id     INTEGER,
    member_count INTEGER DEFAULT 0,
    created_at  INTEGER NOT NULL,
    monitored   INTEGER NOT NULL DEFAULT 1,
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
    raw_proto   BLOB
);

CREATE INDEX IF NOT EXISTS idx_messages_group_time ON messages(group_id, send_time);
CREATE INDEX IF NOT EXISTS idx_groups_monitored ON groups(monitored) WHERE monitored = 1;
"#;
