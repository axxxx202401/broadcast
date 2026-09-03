//! SQLite 存储的建表、消息分页、群组同步与兼容迁移测试。
//!
//! 除基础读写外，本模块重点覆盖事务回滚、用户监控选择保留、远端快照软隐藏与恢复、
//! 旧表结构迁移，以及分页参数边界。

use super::*;
use crate::group::GroupRow;
use crate::message::{MessageCursor, MessageRecord};

fn batch_message(msg_id: i64, content: &str) -> MessageRecord {
    MessageRecord {
        msg_id,
        group_id: 13537,
        send_uid: 109477,
        msg_type: 0,
        content: content.as_bytes().to_vec(),
        send_time: 1_788_420_000_000 + msg_id,
        content_md5: format!("md5-{msg_id}"),
        raw_proto: Some(vec![msg_id as u8]),
    }
}

#[tokio::test]
async fn test_create_tables() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_insert_and_fetch_message() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    let msg_id = store
        .messages
        .insert(&MessageRecord {
            msg_id: 1001,
            group_id: 12345,
            send_uid: 779562,
            msg_type: 0, // 文本消息
            content: b"Hello, World!".to_vec(),
            send_time: 1725292800000,
            content_md5: "d41d8cd98f00b204e9800998ecf8427e".to_string(),
            raw_proto: None,
        })
        .await
        .unwrap();
    assert_eq!(msg_id, 1001);

    // 直接查询底层表，确认写入确实落库。
    let row: Option<(i64,)> = sqlx::query_as("SELECT msg_id FROM messages WHERE msg_id = ?")
        .bind(1001)
        .fetch_optional(&store.pool)
        .await
        .unwrap();
    assert!(row.is_some());
    assert_eq!(row.unwrap().0, 1001);
}

#[tokio::test]
async fn message_batch_upserts_all_rows_in_one_transaction() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .messages
        .insert_batch(&[batch_message(1, "old"), batch_message(2, "second")])
        .await
        .unwrap();
    store
        .messages
        .insert_batch(&[batch_message(1, "new")])
        .await
        .unwrap();

    let page = store.messages.get_by_group(13537, 10, None).await.unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(
        page.messages
            .iter()
            .find(|row| row.msg_id == 1)
            .unwrap()
            .content,
        b"new"
    );
}

#[tokio::test]
async fn sqlite_batch_upserts_ten_thousand_rows_without_duplicate_primary_keys() {
    const MESSAGE_COUNT: i64 = 10_000;
    let store = SqliteStore::new(":memory:").await.unwrap();
    let initial = (1..=MESSAGE_COUNT)
        .map(|msg_id| batch_message(msg_id, "initial"))
        .collect::<Vec<_>>();
    let replacements = (1..=MESSAGE_COUNT)
        .rev()
        .map(|msg_id| batch_message(msg_id, "updated"))
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();

    store.messages.insert_batch(&initial).await.unwrap();
    store.messages.insert_batch(&replacements).await.unwrap();

    let (rows, distinct_ids, updated_rows): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(DISTINCT msg_id),
                SUM(CASE WHEN content = CAST('updated' AS BLOB) THEN 1 ELSE 0 END)
         FROM messages",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(rows, MESSAGE_COUNT);
    assert_eq!(distinct_ids, MESSAGE_COUNT);
    assert_eq!(updated_rows, MESSAGE_COUNT);
    eprintln!(
        "10k SQLite batch/upsert load: elapsed={:?}, rows={rows}, distinct_ids={distinct_ids}",
        started.elapsed()
    );
}

#[tokio::test]
async fn message_batch_rolls_back_every_row_when_one_upsert_fails() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_message_two BEFORE INSERT ON messages
         WHEN NEW.msg_id = 2 BEGIN SELECT RAISE(ABORT, 'rejected'); END",
    )
    .execute(&store.pool)
    .await
    .unwrap();

    let error = store
        .messages
        .insert_batch(&[batch_message(1, "first"), batch_message(2, "second")])
        .await
        .unwrap_err();

    assert!(error.to_string().contains("rejected"));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn file_database_uses_wal_normal_sync_and_busy_timeout() {
    let suffix = chrono::Utc::now().timestamp_nanos_opt().unwrap();
    let path = std::env::temp_dir().join(format!(
        "im-monitor-store-{}-{suffix}.db",
        std::process::id()
    ));
    let store = SqliteStore::new(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(&store.pool)
        .await
        .unwrap();

    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(synchronous, 1);
    assert_eq!(busy_timeout, 5_000);

    drop(store);
    for candidate in [
        path.clone(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        let _ = std::fs::remove_file(candidate);
    }
}

#[tokio::test]
async fn test_get_by_group() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    // 为目标群组写入三条不同发送时间的消息。
    for (i, time) in [1725292800000i64, 1725292801000, 1725292802000]
        .iter()
        .enumerate()
    {
        store
            .messages
            .insert(&MessageRecord {
                msg_id: 2000 + i as i64,
                group_id: 12345,
                send_uid: 779562,
                msg_type: 0,
                content: format!("message {}", i).into_bytes(),
                send_time: *time,
                content_md5: format!("md5-{}", i),
                raw_proto: None,
            })
            .await
            .unwrap();
    }

    // 另一个群组的消息不应混入查询结果。
    store
        .messages
        .insert(&MessageRecord {
            msg_id: 9999,
            group_id: 99999,
            send_uid: 111,
            msg_type: 1,
            content: b"other group".to_vec(),
            send_time: 1725292803000,
            content_md5: "other-md5".to_string(),
            raw_proto: None,
        })
        .await
        .unwrap();

    let first = store.messages.get_by_group(12345, 2, None).await.unwrap();
    assert_eq!(first.messages.len(), 2);
    // 同一群组内按 (send_time, msg_id) 降序返回，并以本页最老消息作为下一页游标。
    assert_eq!(first.messages[0].msg_id, 2002);
    assert_eq!(first.messages[1].msg_id, 2001);
    assert!(first.has_more);
    assert_eq!(
        first.next_cursor,
        Some(MessageCursor {
            send_time: 1725292801000,
            msg_id: 2001,
        })
    );

    let second = store
        .messages
        .get_by_group(12345, 2, first.next_cursor)
        .await
        .unwrap();
    assert_eq!(
        second
            .messages
            .iter()
            .map(|row| row.msg_id)
            .collect::<Vec<_>>(),
        [2000]
    );
    assert!(!second.has_more);
    assert_eq!(second.next_cursor, None);

    // 不存在的群组返回空列表。
    let page = store.messages.get_by_group(0, 10, None).await.unwrap();
    assert!(page.messages.is_empty());
}

#[tokio::test]
async fn message_cursor_paginates_equal_send_times_without_duplicates_or_gaps() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    for msg_id in 1..=5 {
        store
            .messages
            .insert(&MessageRecord {
                msg_id,
                group_id: 7,
                send_uid: 1,
                msg_type: 0,
                content: vec![],
                send_time: 100,
                content_md5: String::new(),
                raw_proto: None,
            })
            .await
            .unwrap();
    }

    let first = store.messages.get_by_group(7, 2, None).await.unwrap();
    let second = store
        .messages
        .get_by_group(7, 2, first.next_cursor)
        .await
        .unwrap();
    let third = store
        .messages
        .get_by_group(7, 2, second.next_cursor)
        .await
        .unwrap();
    let ids = first
        .messages
        .iter()
        .chain(&second.messages)
        .chain(&third.messages)
        .map(|row| row.msg_id)
        .collect::<Vec<_>>();

    assert_eq!(ids, [5, 4, 3, 2, 1]);
    assert!(first.has_more);
    assert!(second.has_more);
    assert!(!third.has_more);
}

#[tokio::test]
async fn test_get_recent_returns_all_groups_with_names() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    for (group_id, name, monitored) in [
        (10, "研发群", 1),
        (20, "运维群", 1),
        (30, "未监控群", 0),
        (40, "不可用群", 1),
    ] {
        store
            .groups
            .insert_or_update(&GroupRow {
                group_id,
                name: name.to_string(),
                pic: String::new(),
                host_id: None,
                member_count: 0,
                created_at: 0,
                monitored,
                updated_at: 0,
            })
            .await
            .unwrap();
        store
            .messages
            .insert(&MessageRecord {
                msg_id: group_id,
                group_id,
                send_uid: 1,
                msg_type: 0,
                content: name.as_bytes().to_vec(),
                send_time: group_id,
                content_md5: String::new(),
                raw_proto: None,
            })
            .await
            .unwrap();
    }
    sqlx::query("UPDATE groups SET available = 0 WHERE group_id = 40")
        .execute(&store.pool)
        .await
        .unwrap();

    let first = store.messages.get_recent(1, None).await.unwrap();
    let second = store
        .messages
        .get_recent(1, first.next_cursor)
        .await
        .unwrap();

    assert_eq!(
        first
            .messages
            .iter()
            .chain(&second.messages)
            .map(|row| row.msg_id)
            .collect::<Vec<_>>(),
        [20, 10]
    );
    assert_eq!(first.messages[0].group_name, "运维群");
    assert_eq!(second.messages[0].group_name, "研发群");
    assert!(first.has_more);
    assert!(!second.has_more);
}

#[tokio::test]
async fn test_get_message_by_id_keeps_raw_proto() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .messages
        .insert(&MessageRecord {
            msg_id: 30,
            group_id: 10,
            send_uid: 1,
            msg_type: 7,
            content: vec![1, 2],
            send_time: 3,
            content_md5: String::new(),
            raw_proto: Some(vec![4, 5, 6]),
        })
        .await
        .unwrap();

    let row = store.messages.get_by_id(30).await.unwrap().unwrap();

    assert_eq!(row.raw_proto, Some(vec![4, 5, 6]));
}

// 密钥生命周期：同一账号的新版本替换“最新值”，私钥能够在应用重启后从 SQLite 恢复。
#[tokio::test]
async fn user_key_pair_store_restores_latest_version() {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    store
        .key_pairs
        .set(&crate::key_pair::UserKeyPairRecord {
            uid: 109_477,
            key_version: 1,
            public_key: "public-v1".to_string(),
            private_key: "private-v1".to_string(),
        })
        .await
        .unwrap();
    store
        .key_pairs
        .set(&crate::key_pair::UserKeyPairRecord {
            uid: 109_477,
            key_version: 2,
            public_key: "public-v2".to_string(),
            private_key: "private-v2".to_string(),
        })
        .await
        .unwrap();

    let actual = store.key_pairs.get_latest(109_477).await.unwrap().unwrap();

    assert_eq!(actual.key_version, 2);
    assert_eq!(actual.public_key, "public-v2");
    assert_eq!(actual.private_key, "private-v2");
}

#[tokio::test]
async fn test_insert_or_update_group() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 12345,
            name: "Test Group".to_string(),
            pic: "http://example.com/pic.jpg".to_string(),
            host_id: Some(779562),
            member_count: 10,
            created_at: 1725292800000,
            monitored: 1,
            updated_at: 1725292800000,
        })
        .await
        .unwrap();

    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 1);
    assert_eq!(monitored[0].group_id, 12345);
    assert_eq!(monitored[0].name, "Test Group");
}

#[tokio::test]
async fn list_all_returns_monitored_and_unmonitored_groups() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    for (group_id, name, monitored) in [(1, "Beta", 1), (2, "Alpha", 0)] {
        store
            .groups
            .insert_or_update(&GroupRow {
                group_id,
                name: name.to_string(),
                pic: String::new(),
                host_id: None,
                member_count: 1,
                created_at: 1,
                monitored,
                updated_at: 1,
            })
            .await
            .unwrap();
    }

    let groups = store.groups.list_all().await.unwrap();

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].name, "Alpha");
    assert_eq!(groups[0].monitored, 0);
    assert_eq!(groups[1].name, "Beta");
    assert_eq!(groups[1].monitored, 1);
}

#[tokio::test]
async fn upsert_preserves_existing_monitored_switch() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    let mut group = GroupRow {
        group_id: 12345,
        name: "Original".to_string(),
        pic: String::new(),
        host_id: None,
        member_count: 1,
        created_at: 1,
        monitored: 1,
        updated_at: 1,
    };
    store.groups.insert_or_update(&group).await.unwrap();

    group.name = "Refreshed".to_string();
    group.monitored = 0;
    group.updated_at = 2;
    store.groups.insert_or_update(&group).await.unwrap();

    let groups = store.groups.list_all().await.unwrap();
    assert_eq!(groups[0].name, "Refreshed");
    assert_eq!(groups[0].monitored, 1);
}

#[tokio::test]
async fn test_toggle_monitored() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 12345,
            name: "Test Group".to_string(),
            pic: "".to_string(),
            host_id: None,
            member_count: 5,
            created_at: 1725292800000,
            monitored: 1,
            updated_at: 1725292800000,
        })
        .await
        .unwrap();

    // 初始状态为已监控。
    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 1);

    // 关闭监控后不再出现在监控列表。
    assert!(store.groups.toggle_monitored(12345, false).await.unwrap());
    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 0);

    // 再次开启后恢复到监控列表。
    assert!(store.groups.toggle_monitored(12345, true).await.unwrap());
    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 1);
}

#[tokio::test]
async fn toggle_monitored_reports_missing_group() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    assert!(!store.groups.toggle_monitored(999, true).await.unwrap());
}

#[tokio::test]
async fn sync_remote_groups_rolls_back_entire_batch_on_failure() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    // 用触发器令批次中第二条 INSERT 失败，证明此前的软隐藏和第一条写入一并回滚。
    sqlx::query(
        "CREATE TRIGGER reject_group_two
         BEFORE INSERT ON groups
         WHEN NEW.group_id = 2
         BEGIN
           SELECT RAISE(ABORT, 'rejected test group');
         END",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let groups = [1, 2].map(|group_id| GroupRow {
        group_id,
        name: format!("Group {group_id}"),
        pic: String::new(),
        host_id: None,
        member_count: 0,
        created_at: 0,
        monitored: 0,
        updated_at: 1,
    });

    assert!(store.groups.sync_remote_groups(&groups).await.is_err());
    assert!(store.groups.list_all().await.unwrap().is_empty());
}

#[tokio::test]
async fn sync_remote_groups_preserves_concurrent_user_monitor_choice() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    let mut group = GroupRow {
        group_id: 7,
        name: "Before".to_string(),
        pic: String::new(),
        host_id: None,
        member_count: 0,
        created_at: 0,
        monitored: 0,
        updated_at: 0,
    };
    store.groups.insert_or_update(&group).await.unwrap();
    // 模拟用户在远端快照到达前开启监控；快照携带的旧值不得覆盖这一选择。
    assert!(store.groups.toggle_monitored(7, true).await.unwrap());

    group.name = "After".to_string();
    group.monitored = 0;
    group.updated_at = 1;
    store.groups.sync_remote_groups(&[group]).await.unwrap();

    let stored = &store.groups.list_all().await.unwrap()[0];
    assert_eq!(stored.name, "After");
    assert_eq!(stored.monitored, 1);
}

#[tokio::test]
async fn remote_snapshot_hides_missing_group_without_deleting_history_and_restores_on_reappearance()
{
    let store = SqliteStore::new(":memory:").await.unwrap();
    let groups = [1, 2].map(|group_id| GroupRow {
        group_id,
        name: format!("Group {group_id}"),
        pic: String::new(),
        host_id: None,
        member_count: 0,
        created_at: 0,
        monitored: 1,
        updated_at: 1,
    });
    store.groups.sync_remote_groups(&groups).await.unwrap();
    store
        .messages
        .insert(&MessageRecord {
            msg_id: 200,
            group_id: 2,
            send_uid: 7,
            msg_type: 0,
            content: b"history".to_vec(),
            send_time: 1,
            content_md5: String::new(),
            raw_proto: None,
        })
        .await
        .unwrap();

    store.groups.sync_remote_groups(&groups[..1]).await.unwrap();

    // 快照缺失只把群组标记为不可见，原监控值和消息历史仍保留。
    assert_eq!(
        store
            .groups
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .map(|group| group.group_id)
            .collect::<Vec<_>>(),
        [1]
    );
    let retained: (i64, i64) =
        sqlx::query_as("SELECT available, monitored FROM groups WHERE group_id = 2")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(retained, (0, 1));
    assert_eq!(
        store
            .messages
            .get_by_group(2, 10, None)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );

    store.groups.sync_remote_groups(&groups[1..]).await.unwrap();

    // 群组再次出现在快照时恢复可见，并沿用软隐藏前的监控选择。
    let restored = store.groups.list_all().await.unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].group_id, 2);
    assert_eq!(restored[0].monitored, 1);
}

#[tokio::test]
async fn legacy_groups_table_is_migrated_with_existing_rows_available() {
    use std::str::FromStr;

    let path = std::env::temp_dir().join(format!(
        "im-monitor-store-migration-{}-{}.db",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let dsn = format!("sqlite://{}", path.display());
    let options = sqlx::sqlite::SqliteConnectOptions::from_str(&dsn)
        .unwrap()
        .create_if_missing(true);
    let legacy_pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query(
        "CREATE TABLE groups (
            group_id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            pic TEXT DEFAULT '',
            host_id INTEGER,
            member_count INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL,
            monitored INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(&legacy_pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO groups
         (group_id, name, created_at, monitored, updated_at)
         VALUES (7, 'Legacy', 1, 1, 1)",
    )
    .execute(&legacy_pool)
    .await
    .unwrap();
    legacy_pool.close().await;

    // 重新打开旧库时应补列；DEFAULT 1 使旧行迁移后立即可见。
    let store = SqliteStore::new(&dsn).await.unwrap();

    let available: i64 = sqlx::query_scalar("SELECT available FROM groups WHERE group_id = 7")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(available, 1);
    assert_eq!(store.groups.list_all().await.unwrap().len(), 1);
    store.pool.close().await;
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn message_store_rejects_invalid_pagination_limit() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    // 游标字段已是 i64；存储层只需拒绝零页长和超过 200 的页长。
    assert!(store.messages.get_by_group(1, 0, None).await.is_err());
    assert!(store.messages.get_by_group(1, 201, None).await.is_err());
}

#[tokio::test]
async fn test_upsert_group_updates_fields() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 12345,
            name: "Original Name".to_string(),
            pic: "http://old.com/pic.jpg".to_string(),
            host_id: Some(111),
            member_count: 5,
            created_at: 1725292800000,
            monitored: 1,
            updated_at: 1725292800000,
        })
        .await
        .unwrap();

    // 冲突更新远端字段，但保留首次写入的监控选择。
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 12345,
            name: "Updated Name".to_string(),
            pic: "http://new.com/pic.jpg".to_string(),
            host_id: Some(222),
            member_count: 20,
            created_at: 1725292800000,
            monitored: 0,
            updated_at: 1725292900000,
        })
        .await
        .unwrap();

    let groups = store.groups.list_all().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "Updated Name");
    assert_eq!(groups[0].pic, "http://new.com/pic.jpg");
    assert_eq!(groups[0].host_id, Some(222));
    assert_eq!(groups[0].member_count, 20);
    assert_eq!(groups[0].updated_at, 1725292900000);
    assert_eq!(groups[0].monitored, 1);
}

#[tokio::test]
async fn test_message_content_and_md5() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    store
        .messages
        .insert(&MessageRecord {
            msg_id: 5001,
            group_id: 12345,
            send_uid: 779562,
            msg_type: 7, // 文件消息
            content: b"binary file content".to_vec(),
            send_time: 1725292800000,
            content_md5: "abc123".to_string(),
            raw_proto: Some(vec![0x08, 0x89, 0x27]),
        })
        .await
        .unwrap();

    let page = store.messages.get_by_group(12345, 10, None).await.unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].content, b"binary file content");
    assert_eq!(page.messages[0].content_md5, "abc123");
    assert_eq!(page.messages[0].msg_type, 7);
    assert_eq!(page.messages[0].raw_proto, Some(vec![0x08, 0x89, 0x27]));
}

#[tokio::test]
async fn creates_missing_database_file() {
    let unique = format!(
        "im-monitor-store-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let directory = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("new.db");

    assert!(!database.exists());
    let store = SqliteStore::new(database.to_str().unwrap()).await.unwrap();
    assert!(database.exists());

    store.pool.close().await;
    std::fs::remove_dir_all(directory).unwrap();
}
