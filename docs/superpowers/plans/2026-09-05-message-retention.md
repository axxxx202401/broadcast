# Message Retention Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete messages older than 7 days (by `send_time`) in batched chunks, triggered on login success and app exit, without impacting query performance.

**Architecture:** Add a `cleanup_old_messages` method to `MessageStore` in `im-store` that uses batched `DELETE ... LIMIT 200` in separate transactions. Call it asynchronously after successful login in `auth.rs`, and on shutdown in `main.rs`.

**Tech Stack:** Rust, sqlx (SQLite), chrono, tokio::spawn

**Spec:** [docs/superpowers/specs/2026-09-05-message-retention-design.md](../specs/2026-09-05-message-retention-design.md)

## Global Constraints

- Retention window is exactly 7 days (`MESSAGE_RETENTION_DAYS = 7`), measured by `send_time` (strictly less than cutoff → deleted).
- Batch size is 200 rows per transaction (`CLEANUP_BATCH_SIZE`), matching `MAX_MESSAGE_PAGE_LIMIT`.
- No new indexes; reuse existing `idx_messages_time (send_time DESC, msg_id DESC)`.
- Cleanup runs in a detached `tokio::spawn`; failures are logged as warnings and never propagate.
- `chrono` is already a workspace dependency with the `clock` feature — no new dependencies needed.
- All changes must compile and all existing tests must continue to pass.

---

### Task 1: Add constants and `cleanup_old_messages` to `MessageStore`

**Files:**
- Modify: `im-store/src/message.rs`

**Interfaces:**
- Produces: `pub const MESSAGE_RETENTION_DAYS: u64 = 7`
- Produces: `const CLEANUP_BATCH_SIZE: usize = 200`
- Produces: `pub async fn cleanup_old_messages(&self, keep_since: i64) -> sqlx::Result<usize>`

- [ ] **Step 1: Write the failing test**

Add to the end of `im-store/src/tests.rs` (before the final closing `}` of the file):

```rust
use chrono::TimeDelta;

/// Empty table: cleanup returns 0 and touches nothing.
#[tokio::test]
async fn test_cleanup_empty_table() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    // Insert a group so messages can be inserted (they need a valid group_id).
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 1,
            name: "g1".to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        })
        .await
        .unwrap();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let deleted = store.messages.cleanup_old_messages(now_ms).await.unwrap();
    assert_eq!(deleted, 0);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

/// All messages older than cutoff: every row is deleted.
#[tokio::test]
async fn test_cleanup_all_expired() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 1,
            name: "g1".to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        })
        .await
        .unwrap();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff = now_ms - 1000 * 60 * 60 * 24 * 8; // 8 days ago

    for msg_id in 1..=5 {
        store
            .messages
            .insert(&MessageRecord {
                msg_id,
                group_id: 1,
                send_uid: 100,
                msg_type: 0,
                content: format!("msg-{msg_id}").into_bytes(),
                send_time: cutoff - msg_id * 1000, // all before cutoff
                content_md5: format!("md5-{msg_id}"),
                raw_proto: None,
                content_text: format!("msg-{msg_id}"),
            })
            .await
            .unwrap();
    }

    let deleted = store.messages.cleanup_old_messages(cutoff).await.unwrap();
    assert_eq!(deleted, 5);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

/// All messages newer than cutoff: nothing is deleted.
#[tokio::test]
async fn test_cleanup_all_retained() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 1,
            name: "g1".to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        })
        .await
        .unwrap();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff = now_ms - 1000 * 60 * 60 * 24 * 6; // 6 days ago

    for msg_id in 1..=3 {
        store
            .messages
            .insert(&MessageRecord {
                msg_id,
                group_id: 1,
                send_uid: 100,
                msg_type: 0,
                content: format!("msg-{msg_id}").into_bytes(),
                send_time: now_ms - msg_id * 1000, // all after cutoff
                content_md5: format!("md5-{msg_id}"),
                raw_proto: None,
                content_text: format!("msg-{msg_id}"),
            })
            .await
            .unwrap();
    }

    let deleted = store.messages.cleanup_old_messages(cutoff).await.unwrap();
    assert_eq!(deleted, 0);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
}

/// Mixed old and new messages: only expired ones are removed.
#[tokio::test]
async fn test_cleanup_partial() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 1,
            name: "g1".to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        })
        .await
        .unwrap();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff = now_ms - 1000 * 60 * 60 * 24 * 7; // 7 days ago

    // 2 old messages (before cutoff)
    for msg_id in 1..=2 {
        store
            .messages
            .insert(&MessageRecord {
                msg_id,
                group_id: 1,
                send_uid: 100,
                msg_type: 0,
                content: format!("old-{msg_id}").into_bytes(),
                send_time: cutoff - msg_id * 1000,
                content_md5: format!("md5-old-{msg_id}"),
                raw_proto: None,
                content_text: format!("old-{msg_id}"),
            })
            .await
            .unwrap();
    }
    // 3 new messages (after cutoff)
    for msg_id in 3..=5 {
        store
            .messages
            .insert(&MessageRecord {
                msg_id,
                group_id: 1,
                send_uid: 100,
                msg_type: 0,
                content: format!("new-{msg_id}").into_bytes(),
                send_time: now_ms - (msg_id as i64) * 1000,
                content_md5: format!("md5-new-{msg_id}"),
                raw_proto: None,
                content_text: format!("new-{msg_id}"),
            })
            .await
            .unwrap();
    }

    let deleted = store.messages.cleanup_old_messages(cutoff).await.unwrap();
    assert_eq!(deleted, 2);

    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT msg_id FROM messages ORDER BY msg_id")
            .fetch_all(&store.pool)
            .await
            .unwrap();
    assert_eq!(ids, vec![3, 4, 5]);
}

/// Message whose send_time exactly equals cutoff is NOT deleted (strict less-than).
#[tokio::test]
async fn test_cleanup_boundary_exact_send_time_preserved() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 1,
            name: "g1".to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        })
        .await
        .unwrap();

    let cutoff = 1_700_000_000_000i64; // exact boundary
    // One message exactly at cutoff — must be preserved.
    store
        .messages
        .insert(&MessageRecord {
            msg_id: 10,
            group_id: 1,
            send_uid: 100,
            msg_type: 0,
            content: b"boundary".to_vec(),
            send_time: cutoff,
            content_md5: "boundary-md5".to_string(),
            raw_proto: None,
            content_text: "boundary".to_string(),
        })
        .await
        .unwrap();
    // One message 1ms before cutoff — must be deleted.
    store
        .messages
        .insert(&MessageRecord {
            msg_id: 11,
            group_id: 1,
            send_uid: 100,
            msg_type: 0,
            content: b"expired".to_vec(),
            send_time: cutoff - 1,
            content_md5: "expired-md5".to_string(),
            raw_proto: None,
            content_text: "expired".to_string(),
        })
        .await
        .unwrap();

    let deleted = store.messages.cleanup_old_messages(cutoff).await.unwrap();
    assert_eq!(deleted, 1);

    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT msg_id FROM messages ORDER BY msg_id")
            .fetch_all(&store.pool)
            .await
            .unwrap();
    assert_eq!(ids, vec![10]);
}

/// More than BATCH_SIZE (200) expired messages: multi-batch cleanup removes all.
#[tokio::test]
async fn test_cleanup_batches_exceed_batch_size() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    store
        .groups
        .insert_or_update(&GroupRow {
            group_id: 1,
            name: "g1".to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 0,
        })
        .await
        .unwrap();

    let cutoff = 1_700_000_000_000i64;
    // Insert 500 expired messages (more than one batch of 200).
    for msg_id in 1..=500 {
        store
            .messages
            .insert(&MessageRecord {
                msg_id,
                group_id: 1,
                send_uid: 100,
                msg_type: 0,
                content: format!("batch-{msg_id}").into_bytes(),
                send_time: cutoff - msg_id,
                content_md5: format!("md5-batch-{msg_id}"),
                raw_proto: None,
                content_text: format!("batch-{msg_id}"),
            })
            .await
            .unwrap();
    }

    let deleted = store.messages.cleanup_old_messages(cutoff).await.unwrap();
    assert_eq!(deleted, 500);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo test -p im-store -- cleanup`
Expected: FAIL with "cannot find function `cleanup_old_messages`" or "no method named `cleanup_old_messages`"

- [ ] **Step 3: Add constants and implement `cleanup_old_messages`**

In `im-store/src/message.rs`, after the existing `MAX_MESSAGE_PAGE_LIMIT` constant (line 4), add:

```rust
/// 消息保留天数。
pub const MESSAGE_RETENTION_DAYS: u64 = 7;
/// 每批次最大删除行数，与分页上限保持一致。
const CLEANUP_BATCH_SIZE: usize = 200;
```

Then add the method to the `impl MessageStore` block (after `get_by_id`, before the closing `}`):

```rust
/// 删除所有 `send_time` 严格早于 `keep_since` 的消息。
///
/// 采用分批删除策略：每批在一个独立事务中执行 `DELETE ... LIMIT BATCH_SIZE`，
/// 批次之间提交事务以释放行锁并允许 WAL checkpoint。当一批删除行数少于
/// `BATCH_SIZE` 时表示已全部清理完毕。
///
/// 返回实际删除的行数。SQL 执行失败时返回 [`sqlx::Error`]。
pub async fn cleanup_old_messages(&self, keep_since: i64) -> sqlx::Result<usize> {
    let mut total_deleted = 0usize;
    loop {
        let result = sqlx::query("DELETE FROM messages WHERE send_time < ? LIMIT ?")
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p im-store -- cleanup`
Expected: All 6 new tests PASS.

- [ ] **Step 5: Run full im-store test suite to confirm no regressions**

Run: `cargo test -p im-store`
Expected: All tests PASS.

- [ ] **Step 6: Commit**

```bash
git add im-store/src/message.rs im-store/src/tests.rs
git commit -m "feat(store): add batched message retention cleanup"
```

---

### Task 2: Trigger cleanup after successful login

**Files:**
- Modify: `im-app/src/commands/auth.rs` (around line 407–414)

**Interfaces:**
- Consumes: `im_store::message::MESSAGE_RETENTION_DAYS` (public constant from Task 1)
- Consumes: `state.account_db.open(...)` which returns `Arc<SqliteStore>`

- [ ] **Step 1: Locate the insertion point**

Open `im-app/src/commands/auth.rs` and find the `run_complete_account_login` function (starts at line 389). The database is opened at line 407:
```rust
let db = state.account_db.open(uid, generation).await?;
```
The function continues through line 414 with `after_publish.await;`. We need to spawn the cleanup task **after** the full login succeeds — the spec says after `ensure_login_generation_current` at line 414.

- [ ] **Step 2: Add the spawn call**

After line 414 (`ensure_login_generation_current(state, generation, uid).await?;`), insert:

```rust
// 后台清理超过保留窗口的旧消息；不阻塞登录响应。
let cleanup_db = db.clone();
tokio::spawn(async move {
    let cutoff = chrono::Utc::now()
        .timestamp_millis()
        .saturating_sub(im_store::message::MESSAGE_RETENTION_DAYS as i64 * 24 * 3600 * 1000);
    match cleanup_db.messages.cleanup_old_messages(cutoff).await {
        Ok(n) => tracing::info!(deleted = n, "message retention cleanup completed"),
        Err(e) => tracing::warn!(error = %e, "message retention cleanup failed"),
    }
});
```

- [ ] **Step 3: Build to verify compilation**

Run: `cargo build -p im-app`
Expected: Build succeeds with no errors.

- [ ] **Step 4: Run auth tests to confirm no regressions**

Run: `cargo test -p im-app -- auth`
Expected: All existing auth tests PASS.

- [ ] **Step 5: Commit**

```bash
git add im-app/src/commands/auth.rs
git commit -m "feat(auth): trigger message retention cleanup after login"
```

---

### Task 3: Trigger cleanup on app exit

**Files:**
- Modify: `im-app/src/main.rs` (lines 201–206)

**Interfaces:**
- Consumes: `im_store::message::MESSAGE_RETENTION_DAYS`
- Consumes: `AppState::account_db` via `app_handle.state::<AppState>()`

- [ ] **Step 1: Add the exit-time cleanup**

In `im-app/src/main.rs`, replace the `RunEvent::ExitRequested` arm (lines 202–205) with:

```rust
if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
    // 退出前尝试清理过期消息，作为登录路径之外的兜底。
    let state = app_handle.state::<AppState>();
    let cleanup_fut = async {
        if let Ok(db) = state.account_db.active().await {
            let cutoff = chrono::Utc::now()
                .timestamp_millis()
                .saturating_sub(im_store::message::MESSAGE_RETENTION_DAYS as i64 * 24 * 3600 * 1000);
            let _ = db.messages.cleanup_old_messages(cutoff).await;
        }
    };
    tokio::spawn(cleanup_fut);
    state.shutdown.cancel();
}
```

- [ ] **Step 2: Build to verify compilation**

Run: `cargo build -p im-app`
Expected: Build succeeds with no errors.

- [ ] **Step 3: Run full im-app test suite**

Run: `cargo test -p im-app`
Expected: All tests PASS.

- [ ] **Step 4: Run full project build and test**

Run: `cargo test`
Expected: All tests across all crates PASS.

- [ ] **Step 5: Commit**

```bash
git add im-app/src/main.rs
git commit -m "feat(app): trigger message retention cleanup on exit"
```
