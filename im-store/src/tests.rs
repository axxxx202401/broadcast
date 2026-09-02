use super::*;
use crate::message::MessageRecord;
use crate::group::GroupRow;

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
    let msg_id = store.messages.insert(&MessageRecord {
        msg_id: 1001,
        group_id: 12345,
        send_uid: 779562,
        msg_type: 0, // text
        content: b"Hello, World!".to_vec(),
        send_time: 1725292800000,
        content_md5: "d41d8cd98f00b204e9800998ecf8427e".to_string(),
    })
    .await
    .unwrap();
    assert_eq!(msg_id, 1001);

    // Verify the row was written
    let row: Option<(i64,)> = sqlx::query_as("SELECT msg_id FROM messages WHERE msg_id = ?")
        .bind(1001)
        .fetch_optional(&store.pool)
        .await
        .unwrap();
    assert!(row.is_some());
    assert_eq!(row.unwrap().0, 1001);
}

#[tokio::test]
async fn test_get_by_group() {
    let store = SqliteStore::new(":memory:").await.unwrap();

    // Insert three messages for group 12345
    for (i, time) in [1725292800000i64, 1725292801000, 1725292802000].iter().enumerate() {
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
            })
            .await
            .unwrap();
    }

    // Insert one message for a different group
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
        })
        .await
        .unwrap();

    let rows = store.messages.get_by_group(12345, 10, 0).await.unwrap();
    assert_eq!(rows.len(), 3);
    // Should be ordered by send_time DESC
    assert_eq!(rows[0].msg_id, 2002);
    assert_eq!(rows[1].msg_id, 2001);
    assert_eq!(rows[2].msg_id, 2000);

    // Offset and limit
    let rows = store.messages.get_by_group(12345, 1, 2).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].msg_id, 2000);

    // Non-existent group returns empty
    let rows = store.messages.get_by_group(0, 10, 0).await.unwrap();
    assert!(rows.is_empty());
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

    // Initially monitored
    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 1);

    // Toggle off
    store.groups.toggle_monitored(12345, false).await.unwrap();
    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 0);

    // Toggle back on
    store.groups.toggle_monitored(12345, true).await.unwrap();
    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 1);
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

    // Upsert with updated values
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 12345,
            name: "Updated Name".to_string(),
            pic: "http://new.com/pic.jpg".to_string(),
            host_id: Some(222),
            member_count: 20,
            created_at: 1725292800000,
            monitored: 1,
            updated_at: 1725292900000,
        })
        .await
        .unwrap();

    let monitored = store.groups.list_monitored().await.unwrap();
    assert_eq!(monitored.len(), 1);
    assert_eq!(monitored[0].name, "Updated Name");
    assert_eq!(monitored[0].member_count, 20);
    assert_eq!(monitored[0].updated_at, 1725292900000);
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
            msg_type: 7, // file
            content: b"binary file content".to_vec(),
            send_time: 1725292800000,
            content_md5: "abc123".to_string(),
        })
        .await
        .unwrap();

    let rows = store.messages.get_by_group(12345, 10, 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].content, b"binary file content");
    assert_eq!(rows[0].content_md5, "abc123");
    assert_eq!(rows[0].msg_type, 7);
}
