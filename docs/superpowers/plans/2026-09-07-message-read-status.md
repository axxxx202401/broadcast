# 消息已读/未读功能 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为监控面板消息列表增加已读/未读状态，支持一键标已读、人工滚动批量标记、排序方向切换，所有样式跟随黑白主题。

**Architecture:** 在 `messages` 表加 `read_at` 列（0=未读，>0=Unix ms时间戳）。后端新增 `mark_group_read` 命令，前端通过 `useMonitor` composable 管理未读计数、排序方向，`MessagePanel` 组件渲染浮窗按钮和排序切换图标。

**Tech Stack:** Rust (Tauri + sqlx), Vue 3 + TypeScript, TanStack Vue Virtual, CSS custom properties (dark/light theme)

**Spec:** [docs/superpowers/specs/2026-09-07-message-read-status-design.md](../specs/2026-09-07-message-read-status-design.md)

## Global Constraints

- `read_at = 0` 表示未读，`read_at > 0` 表示已读时间（Unix 毫秒）
- 只有 `matched ≠ 0` 的消息才会有未读状态；不匹配的消息 `read_at` 始终为 0 但不可见
- 全群视图和单群视图使用全局统一的 `read_at` 追踪，不区分群维度
- 所有新增 CSS 类必须在 `[data-theme="light"]` 中有对应的亮色主题规则
- 图标风格与现有 `.icon-button` / `.eye-toggle svg` 保持一致（38×38px，圆角 `var(--radius)`，SVG stroke-linecap="round" stroke-width="2"）
- 后端迁移代码必须检查列是否存在再执行 ALTER TABLE，与现有 `migrate_groups_available` 模式一致

---

### Task 1: 数据库迁移 — 添加 `read_at` 列

**Files:**
- Modify: `im-store/src/schema.rs` — 在 `SCHEMA_SQL` 中加列定义
- Modify: `im-store/src/lib.rs` — 加迁移函数 `migrate_messages_read_at`
- Modify: `im-store/src/tests.rs` — 加迁移测试

**Interfaces:**
- Consumes: 无
- Produces: `SqliteStore::new` 自动执行迁移，新表含 `read_at INTEGER NOT NULL DEFAULT 0`

- [ ] **Step 1: 在 SCHEMA_SQL 中添加 `read_at` 列定义**

```rust
// 在 messages 表定义中添加：
    read_at     INTEGER NOT NULL DEFAULT 0,
```

找到 `im-store/src/schema.rs` 中 `messages` 表的 CREATE 语句，在 `content_text TEXT DEFAULT ''` 行后面添加 `read_at` 列。新库直接建表时包含该列；老库由迁移函数补齐。

- [ ] **Step 2: 在 lib.rs 中添加迁移函数**

在 `migrate_lottery_config_issues` 之后添加：

```rust
/// 检查 `messages` 表，并在缺失时补充 `read_at` 列。
async fn migrate_messages_read_at(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'read_at'",
    )
    .fetch_one(pool)
    .await?;
    if column_count == 0 {
        sqlx::query("ALTER TABLE messages ADD COLUMN read_at INTEGER NOT NULL DEFAULT 0")
            .execute(pool)
            .await?;
    }
    Ok(())
}
```

- [ ] **Step 3: 在 `SqliteStore::new` 中调用迁移函数**

在 `migrate_lottery_config_issues(&pool).await?;` 之后添加：

```rust
migrate_messages_read_at(&pool).await?;
```

- [ ] **Step 4: 添加迁移测试**

在 `im-store/src/tests.rs` 末尾添加：

```rust
#[tokio::test]
async fn test_migrate_messages_read_at_column() {
    // 全新内存库：SCHEMA_SQL 应包含 read_at 列。
    let store = SqliteStore::new(":memory:").await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'read_at'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);

    // 插入消息时 read_at 默认 0。
    let row: (i64,) = sqlx::query_as(
        "SELECT read_at FROM messages WHERE msg_id = 1",
    )
    .bind(1)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(row.0, 0);
}
```

- [ ] **Step 5: 运行测试验证**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo test -p im-store test_migrate_messages_read_at_column -- --nocapture`

Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add im-store/src/schema.rs im-store/src/lib.rs im-store/src/tests.rs
git commit -m "feat(store): add read_at column to messages table with migration"
```

---

### Task 2: MessageStore — 支持 read_at 查询和标记

**Files:**
- Modify: `im-store/src/message.rs` — `MessageRow` 加字段，SQL 补列，新增 `mark_read` 方法
- Modify: `im-store/src/tests.rs` — 加 `mark_read` 测试

**Interfaces:**
- Consumes: Task 1 的 `read_at` 列
- Produces: `MessageRow.read_at: i64`，`MessageStore::mark_read(group_id, to_msg_id) -> sqlx::Result<usize>`

- [ ] **Step 1: `MessageRow` 加 `read_at` 字段**

在 `im-store/src/message.rs` 中，`MessageRow` struct 的 `content_text: String` 后面添加：

```rust
/// 已读时间戳（Unix ms）；0 表示未读。
pub read_at: i64,
```

- [ ] **Step 2: 所有 SELECT 查询补充 `m.read_at`**

`MessageStore` 中有 4 个 SELECT 查询（`get_by_group` 两次、`get_recent` 两次、`get_by_id`）。每个查询的 SELECT 列表末尾补充 `, m.read_at`。同时 `message_page()` 函数中的 `map` 闭包和 `get_by_id` 的 map 闭包补充 `read_at: row.get("read_at")`。

具体改动位置：
- `get_by_group` cursor 分支 SQL：在 `m.content_text` 后加 `, m.read_at`
- `get_by_group` 无 cursor 分支 SQL：同上
- `get_recent` cursor 分支 SQL：同上
- `get_recent` 无 cursor 分支 SQL：同上
- `get_by_id` SQL：在 `m.content_text` 后加 `, m.read_at`
- `message_page()` 函数的 map 闭包：加 `read_at: row.get("read_at")`
- `get_by_id` 的 map 闭包：加 `read_at: row.get("read_at")`

- [ ] **Step 3: 添加 `mark_read` 方法**

在 `MessageStore` impl 块中，`cleanup_old_messages` 之后添加：

```rust
/// 将指定群组中满足条件的未读匹配消息标记为已读。
///
/// `group_id` 为 `None` 时跨群标记；`Some(id)` 时只标该群。
/// 条件：`matched != 0 AND read_at = 0 AND msg_id <= to_msg_id`。
/// 返回受影响的行数。
pub async fn mark_read(
    &self,
    group_id: Option<i64>,
    to_msg_id: i64,
) -> sqlx::Result<usize> {
    let now = chrono::Utc::now().timestamp_millis();
    let result = if let Some(gid) = group_id {
        sqlx::query(
            "UPDATE messages SET read_at = ? WHERE group_id = ? AND matched != 0 AND read_at = 0 AND msg_id <= ?",
        )
        .bind(now)
        .bind(gid)
        .bind(to_msg_id)
        .execute(&self.pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE messages SET read_at = ? WHERE matched != 0 AND read_at = 0 AND msg_id <= ?",
        )
        .bind(now)
        .bind(to_msg_id)
        .execute(&self.pool)
        .await?
    };
    Ok(result.rows_affected() as usize)
}
```

- [ ] **Step 4: 添加 `mark_read` 测试**

在 `im-store/src/tests.rs` 末尾添加：

```rust
#[tokio::test]
async fn test_mark_read_updates_matching_unread_messages() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    // 插入 5 条匹配消息（read_at 默认为 0 = 未读）。
    for msg_id in 1..=5 {
        store.messages.insert(&MessageRecord {
            msg_id,
            group_id: 100,
            send_uid: 200,
            msg_type: 0,
            content: format!("msg-{msg_id}").into_bytes(),
            send_time: 1_000_000_000_000 + msg_id * 1000,
            content_md5: format!("md5-{msg_id}"),
            raw_proto: None,
            content_text: format!("msg-{msg_id}"),
        }).await.unwrap();
    }
    // 插入 1 条不匹配消息，不应被 mark_read 影响。
    store.messages.insert(&MessageRecord {
        msg_id: 999,
        group_id: 100,
        send_uid: 200,
        msg_type: 0,
        content: b"unmatched".to_vec(),
        send_time: 1_000_000_005_000,
        content_md5: "md5-999".to_string(),
        raw_proto: None,
        content_text: "unmatched".to_string(),
    }).await.unwrap();

    // 标记到 msg_id=3，应影响 1、2、3 三条。
    let affected = store.messages.mark_read(Some(100), 3).await.unwrap();
    assert_eq!(affected, 3);

    // 验证：1、2、3 的 read_at > 0，4、5、999 的 read_at = 0。
    for msg_id in [1, 2, 3] {
        let read_at: i64 = sqlx::query_scalar("SELECT read_at FROM messages WHERE msg_id = ?")
            .bind(msg_id)
            .fetch_one(&store.pool)
            .await.unwrap();
        assert!(read_at > 0, "msg_id {msg_id} should be marked read");
    }
    for msg_id in [4, 5, 999] {
        let read_at: i64 = sqlx::query_scalar("SELECT read_at FROM messages WHERE msg_id = ?")
            .bind(msg_id)
            .fetch_one(&store.pool)
            .await.unwrap();
        assert_eq!(read_at, 0, "msg_id {msg_id} should remain unread");
    }
}

#[tokio::test]
async fn test_mark_read_global_mode_ignores_group_boundary() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    for (gid, msg_id) in [(100, 1), (100, 2), (200, 1), (200, 2)] {
        store.messages.insert(&MessageRecord {
            msg_id,
            group_id: gid,
            send_uid: 200,
            msg_type: 0,
            content: b"x".to_vec(),
            send_time: 1_000_000_000_000 + msg_id,
            content_md5: "md5".to_string(),
            raw_proto: None,
            content_text: "x".to_string(),
        }).await.unwrap();
    }
    // 全局标记到 msg_id=2，应覆盖两个群的所有 msg_id<=2。
    let affected = store.messages.mark_read(None, 2).await.unwrap();
    assert_eq!(affected, 4);
}
```

- [ ] **Step 5: 运行测试验证**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo test -p im-store test_mark_read -- --nocapture`

Expected: 两个测试均 PASS

- [ ] **Step 6: 提交**

```bash
git add im-store/src/message.rs im-store/src/tests.rs
git commit -m "feat(store): add read_at field and mark_read method to MessageStore"
```

---

### Task 3: 后端 DTO 和 Tauri 命令

**Files:**
- Modify: `im-app/src/commands/chat.rs` — `MessageDto` 加 `read_at`，`message_dto_from_row` 补字段，新增 `mark_group_read` 命令
- Modify: `im-app/src/main.rs` — 注册新命令

**Interfaces:**
- Consumes: Task 2 的 `MessageRow.read_at`、`MessageStore::mark_read`
- Produces: `MessageDto.read_at: i64`，Tauri 命令 `mark_group_read(group_id: Option<String>, to_msg_id: String) -> Result<usize, String>`

- [ ] **Step 1: `MessageDto` 加 `read_at` 字段**

在 `im-app/src/commands/chat.rs` 的 `MessageDto` struct 中，`matched: i32` 之后添加：

```rust
/// 已读时间戳（Unix ms）；0 表示未读。
pub read_at: i64,
```

- [ ] **Step 2: `message_dto_from_row` 补 `read_at` 字段**

找到 `fn message_dto_from_row` 函数，在末尾的 `matched: row.matched,` 之后添加：

```rust
read_at: row.read_at,
```

- [ ] **Step 3: 实时消息批量写入时也设置 `read_at`**

在 `im-app/src/commands/chat.rs` 中找到实时消息构造 `MessageRecord` 的地方（约第 158 行附近的 `make_message_record` 或类似函数），在 `MessageRecord` 初始化中确认 `content_text` 字段后有默认值。`MessageRecord` 结构体没有 `read_at` 字段（那是 `MessageRow` 的），写入时不需要特别处理——INSERT 语句中 `read_at` 使用 DEFAULT 0，符合预期（实时消息入库时为未读）。

- [ ] **Step 4: 新增 `mark_group_read` 命令**

在 `im-app/src/commands/chat.rs` 中，`get_messages` 命令之后添加：

```rust
/// 将满足条件的未读匹配消息标记为已读。
///
/// `group_id` 为 `None` 时跨群标记；为 `Some` 时只标指定群。
/// 条件：`matched != 0 AND read_at = 0 AND msg_id <= to_msg_id`。
/// 返回受影响的行数（0 表示已全部标记或无匹配消息）。
#[tauri::command]
pub async fn mark_group_read(
    state: State<'_, AppState>,
    group_id: Option<String>,
    to_msg_id: String,
) -> Result<usize, String> {
    let session = authenticated_session_for_connect(&state.auth_session).await?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|error| error.to_string())?;
    let to_msg_id = parse_i64_id(&to_msg_id, "to_msg_id")?;
    let group_id = group_id.as_ref().map(|s| parse_i64_id(s, "group_id")).transpose()?;
    tracing::info!(
        uid = session.uid,
        group_id = group_id.map(|g| g.to_string()).as_deref().unwrap_or("all"),
        to_msg_id,
        "mark_group_read"
    );
    db.messages.mark_read(group_id, to_msg_id).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 5: 在 main.rs 注册新命令**

在 `im-app/src/main.rs` 的 `invoke_handler` 中，`commands::chat::download_message_attachment` 之后添加：

```rust
commands::chat::mark_group_read,
```

- [ ] **Step 6: 编译验证**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo check -p im-app`

Expected: 无编译错误

- [ ] **Step 7: 提交**

```bash
git add im-app/src/commands/chat.rs im-app/src/main.rs
git commit -m "feat(app): add mark_group_read Tauri command with read_at in MessageDto"
```

---

### Task 4: 前端类型和 Tauri API

**Files:**
- Modify: `im-app/ui/src/types/im.ts`
- Modify: `im-app/ui/src/services/tauri.ts`

**Interfaces:**
- Consumes: 无
- Produces: `MessageDto.read_at: number`，`api.markGroupRead(groupId: string | null, toMsgId: string) -> Promise<number>`

- [ ] **Step 1: `MessageDto` 加 `read_at` 字段**

在 `im-app/ui/src/types/im.ts` 的 `MessageDto` interface 中，`matched: number` 之后添加：

```typescript
  /** 已读时间戳（Unix ms）；0 表示未读。 */
  read_at: number
```

- [ ] **Step 2: `tauri.ts` 添加 `markGroupRead`**

在 `im-app/ui/src/services/tauri.ts` 的 `api` 对象中，`downloadMessageAttachment` 之后添加：

```typescript
  /**
   * 将指定范围内未读匹配消息标记为已读。
   * `groupId` 为 `null` 时全群标记；非空时只标该群。
   */
  markGroupRead: (groupId: string | null, toMsgId: string) =>
    invoke<number>('mark_group_read', { groupId, toMsgId }),
```

- [ ] **Step 3: 编译验证**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx tsc --noEmit`

Expected: 无类型错误

- [ ] **Step 4: 提交**

```bash
git add im-app/ui/src/types/im.ts im-app/ui/src/services/tauri.ts
git commit -m "feat(ui): add read_at to MessageDto and markGroupRead API"
```

---

### Task 5: `useMonitor` — 未读计数、排序方向、滚动缓冲

**Files:**
- Modify: `im-app/ui/src/composables/useMonitor.ts`

**Interfaces:**
- Consumes: Task 4 的 `api.markGroupRead`
- Produces: `unreadCount: Ref<number>`，`messageOrder: Ref<'newest-top'|'newest-bottom'>`，`markAllAsRead(toMsgId?: string): Promise<void>`，`handleScrollStopped(maxMsgId: string): Promise<void>`，`toggleMessageOrder(): void`

- [ ] **Step 1: 添加排序方向持久化**

在 `useMonitor` 函数体开头附近（`showMatchedOnly` 声明之后）添加：

```typescript
const MESSAGE_ORDER_KEY = 'im-message-order'
const messageOrder = ref<'newest-top' | 'newest-bottom'>('newest-top')
try {
  const stored = localStorage.getItem(MESSAGE_ORDER_KEY)
  if (stored === 'newest-bottom') messageOrder.value = 'newest-bottom'
} catch {}

function toggleMessageOrder() {
  const next = messageOrder.value === 'newest-top' ? 'newest-bottom' : 'newest-top'
  messageOrder.value = next
  try { localStorage.setItem(MESSAGE_ORDER_KEY, next) } catch {}
  void loadMessages(selectedGroupId.value)
}
```

- [ ] **Step 2: 添加 `unreadCount` computed**

在 `filteredMessages` computed 之后添加：

```typescript
const unreadCount = computed(() =>
  filteredMessages.value.filter(m => m.matched !== 0 && m.read_at === 0).length,
)
```

- [ ] **Step 3: 添加 `markAllAsRead` 方法**

```typescript
async function markAllAsRead(toMsgId?: string) {
  if (unreadCount.value === 0) return
  const groupId = selectedGroupId.value
  const targetMsgId = toMsgId ?? messages.value[messages.value.length - 1]?.msg_id
  if (!targetMsgId) return
  const affected = await api.markGroupRead(groupId, targetMsgId)
  if (affected === 0) return
  const now = Date.now()
  messages.value = messages.value.map(m =>
    m.matched !== 0 && m.read_at === 0 ? { ...m, read_at: now } : m
  )
  await loadMessages(groupId)
}
```

- [ ] **Step 4: 添加 `handleScrollStopped` 方法**

```typescript
async function handleScrollStopped(maxMsgId: string) {
  if (unreadCount.value === 0) return
  const groupId = selectedGroupId.value
  // 先检查是否已有更新：比较 maxMsgId 与最后一条消息的 read_at
  const latestMsg = messages.value[messages.value.length - 1]
  if (latestMsg && latestMsg.msg_id === maxMsgId && latestMsg.read_at !== 0) return
  await markAllAsRead(maxMsgId)
}
```

- [ ] **Step 5: 首次连接后初始化老数据为已读**

在 `onMounted` 中，`messageChannel.onmessage` 注册之后，`listen('message_keys_ready', ...)` 之前，添加：

```typescript
listen('message_keys_ready', () => {
  if (loggedIn.value) void loadMessages(selectedGroupId.value)
  // 首次连接后把历史消息全部标为已读，避免老数据满屏未读提示。
  if (loggedIn.value && messages.value.length > 0) {
    const lastMsg = messages.value[messages.value.length - 1]
    if (lastMsg) void api.markGroupRead(selectedGroupId.value, lastMsg.msg_id)
  }
})
```

- [ ] **Step 6: 在 return 中添加新导出**

在 `return {` 块末尾添加：

```typescript
    /** 当前未读匹配消息数。 */
    unreadCount,
    /** 消息排序方向：`newest-top`（默认，最新消息在顶部）或 `newest-bottom`。 */
    messageOrder,
    markAllAsRead,
    handleScrollStopped,
    toggleMessageOrder,
```

- [ ] **Step 7: 编译验证**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx tsc --noEmit`

Expected: 无类型错误

- [ ] **Step 8: 提交**

```bash
git add im-app/ui/src/composables/useMonitor.ts
git commit -m "feat(ui): add unreadCount, markAllAsRead, scroll buffer, messageOrder to useMonitor"
```

---

### Task 6: `MessagePanel` — 浮窗按钮、排序切换、滚动缓冲、reverse 虚拟列表

**Files:**
- Modify: `im-app/ui/src/components/MessagePanel.vue`

**Interfaces:**
- Consumes: Task 5 的 `unreadCount`、`messageOrder`、`markAllAsRead`、`handleScrollStopped`
- Produces: `unread-float-btn`（浮窗按钮）、`order-toggle-btn`（排序切换）、`scroll-stopped` 事件、`reverse` virtualizer 选项

- [ ] **Step 1: 新增 props 和 emits**

在 `withDefaults(defineProps<{...}>(), {...})` 中追加：

```typescript
  /** 当前未读匹配消息数。 */
  unreadCount?: number
  /** 消息排序方向。 */
  messageOrder?: 'newest-top' | 'newest-bottom'
}>(), {
  // ... 现有默认值
  unreadCount: 0,
  messageOrder: 'newest-top',
})
```

在 `defineEmits` 中追加：

```typescript
  'mark-read': []
  'toggle-order': []
  /** 人工滚动停止，携带视口内收集到的最大未读 msg_id。 */
  'scroll-stopped': [maxMsgId: string]
```

- [ ] **Step 2: 滚动缓冲状态**

在组件 script 中（`const emit = defineEmits` 之后）添加：

```typescript
const SCROLL_DEBOUNCE_MS = 300
const scrollBuffer = ref<string[]>([])
let scrollDebounceTimer: ReturnType<typeof setTimeout> | null = null

function onScroll(event: Event) {
  const element = event.currentTarget as HTMLElement
  // 收集视口内未读 msg_id：读取虚拟列表当前渲染的行。
  const currentIds = virtualItems.value
    .filter(item => {
      const msg = props.messages[item.index]
      return msg && msg.matched !== 0 && msg.read_at === 0
    })
    .map(item => props.messages[item.index]!.msg_id)

  if (currentIds.length > 0) {
    const next = [...new Set([...scrollBuffer.value, ...currentIds])]
    scrollBuffer.value = next
  }

  if (scrollDebounceTimer) clearTimeout(scrollDebounceTimer)
  scrollDebounceTimer = setTimeout(() => {
    if (scrollBuffer.value.length === 0) return
    const maxMsgId = scrollBuffer.value
      .map(id => props.messages.find(m => m.msg_id === id))
      .filter(Boolean)
      .map(m => parseInt(m!.msg_id))
      .reduce((a, b) => Math.max(a, b), 0)
    if (maxMsgId > 0) {
      emit('scroll-stopped', maxMsgId.toString())
    }
    scrollBuffer.value = []
  }, SCROLL_DEBOUNCE_MS)
}
```

注意：`handleScroll` 函数已存在，需要在其内部同时触发滚动缓冲逻辑（复用同一 `@scroll` 处理器，或在原有 `handleScroll` 中追加缓冲逻辑）。建议在原有 `@scroll="handleScroll"` 改为 `@scroll="handleScrollAndBuffer"`，内部同时调用原有逻辑和缓冲逻辑。

- [ ] **Step 3: `isAtBottom` 计算属性**

```typescript
const isAtBottom = computed(() => {
  const el = viewport.value
  if (!el) return false
  return el.scrollHeight - el.scrollTop - el.clientHeight <= 80
})
```

- [ ] **Step 4: 修改 `@scroll` 绑定和 virtualizer `reverse` 选项**

把 `<div ref="viewport" ... @scroll="handleScroll">` 改为 `@scroll="handleScrollAndBuffer"`，其中：

```typescript
function handleScrollAndBuffer(event: Event) {
  handleScroll(event)
  onScroll(event)
}
```

在 `virtualizerOptions` computed 中追加：

```typescript
reverse: props.messageOrder === 'newest-bottom',
```

- [ ] **Step 5: 添加浮窗按钮 HTML**

在 `<div ref="viewport">` 内部、`<ol class="message-log">` 之前添加：

```html
<button
  v-if="unreadCount > 0 && !isAtBottom"
  class="unread-float-btn"
  :class="messageOrder === 'newest-top' ? 'unread-float-btn--top' : 'unread-float-btn--bottom'"
  @click="emit('mark-read')"
  :aria-label="`标记全部已读，共 ${unreadCount} 条未读`"
>
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 5v14M5 12l7 7 7-7"/>
  </svg>
  <span v-if="unreadCount > 0" class="unread-float-btn__badge">{{ unreadCount > 99 ? '99+' : unreadCount }}</span>
</button>
```

- [ ] **Step 6: 添加排序切换按钮 HTML**

在 `<header class="message-header">` 内的 `.message-header-row` 右侧添加：

```html
<button
  class="icon-button order-toggle-btn"
  @click="emit('toggle-order')"
  :title="messageOrder === 'newest-top' ? '切换到从下往上排列' : '切换到从上往下排列'"
  aria-label="切换消息排序方向"
>
  <svg v-if="messageOrder === 'newest-top'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 19V5M5 12l7-7 7 7"/>
  </svg>
  <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 5v14M19 12l-7 7-7-7"/>
  </svg>
</button>
```

- [ ] **Step 7: 自动滚底逻辑根据排序方向条件判断**

在现有的自动滚底 watcher 中（监听 `messages.at(-1)?.msg_id` 的那个），添加排序方向检查：仅在 `messageOrder === 'newest-bottom'` 时才自动滚底。找到该 watcher 中的：

```typescript
if (!isInitialLoad && !wasNearBottom) return
```

之前添加：

```typescript
if (props.messageOrder === 'newest-top') return
```

- [ ] **Step 8: 提交**

```bash
git add im-app/ui/src/components/MessagePanel.vue
git commit -m "feat(ui): add unread float button, order toggle, scroll buffer, reverse virtualizer to MessagePanel"
```

---

### Task 7: CSS — 未读样式、浮窗按钮、排序切换、主题适配

**Files:**
- Modify: `im-app/ui/src/styles/console.css`

**Interfaces:**
- Consumes: Task 6 的 class 名（`.unread-float-btn`、`.unread-float-btn__badge`、`.unread-float-btn--top`、`.unread-float-btn--bottom`、`.order-toggle-btn`、`.message-card--unread`）
- Produces: 深色和亮色两套完整样式

- [ ] **Step 1: 未读消息卡片样式**

在 `.message-card--new` 的 `@keyframes` 之后添加：

```css
/* 未读匹配消息：左侧蓝色竖线指示器 */
.message-card--unread {
  border-left: 3px solid var(--info);
  background-color: rgba(88, 166, 255, 0.04);
}

[data-theme="light"] .message-card--unread {
  border-left: 3px solid var(--info);
  background-color: rgba(9, 105, 218, 0.05);
}
```

- [ ] **Step 2: 浮窗按钮样式**

在 `.order-toggle-btn:hover` 之后添加：

```css
/* 未读浮窗按钮：圆形，右下角/右上角 */
.unread-float-btn {
  position: absolute;
  bottom: 20px;
  right: 20px;
  z-index: 10;
  width: 44px;
  height: 44px;
  border-radius: 50%;
  background: var(--accent);
  color: var(--bg-canvas);
  border: none;
  cursor: pointer;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: opacity 200ms, transform 200ms, background 150ms;
}
.unread-float-btn:hover {
  transform: scale(1.08);
  background: var(--accent-soft);
}
.unread-float-btn svg {
  width: 20px;
  height: 20px;
}
.unread-float-btn--top {
  top: 80px;
  bottom: auto;
}
.unread-float-btn--bottom {
  top: auto;
  bottom: 20px;
}
.unread-float-btn__badge {
  position: absolute;
  top: -4px;
  right: -4px;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  border-radius: 9px;
  background: var(--danger);
  color: #fff;
  font-size: 11px;
  font-weight: 700;
  line-height: 18px;
  text-align: center;
  pointer-events: none;
}

/* 亮色主题浮窗按钮 */
[data-theme="light"] .unread-float-btn {
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.2);
}
[data-theme="light"] .unread-float-btn:hover {
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.25);
}
```

- [ ] **Step 3: 排序切换按钮 SVG 尺寸**

在 `.order-toggle-btn:hover` 之后添加：

```css
.order-toggle-btn svg {
  width: 16px;
  height: 16px;
}
```

- [ ] **Step 4: 提交**

```bash
git add im-app/ui/src/styles/console.css
git commit -m "feat(ui): add unread card styles, float button, order toggle CSS with light/dark themes"
```

---

### Task 8: `App.vue` — 传递新 props 和事件

**Files:**
- Modify: `im-app/ui/src/App.vue`

**Interfaces:**
- Consumes: Task 5 的 `unreadCount`、`messageOrder`、`markAllAsRead`、`handleScrollStopped`、`toggleMessageOrder`
- Produces: 无（仅透传）

- [ ] **Step 1: 在 `<MessagePanel>` 上添加新 props 和事件**

找到 `<MessagePanel ... />` 标签，在现有属性后追加：

```html
:unread-count="monitor.unreadCount"
:message-order="monitor.messageOrder"
@mark-read="monitor.markAllAsRead"
@toggle-order="monitor.toggleMessageOrder"
@scroll-stopped="(maxMsgId) => monitor.handleScrollStopped(maxMsgId)"
```

- [ ] **Step 2: 编译验证**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx tsc --noEmit`

Expected: 无类型错误

- [ ] **Step 3: 提交**

```bash
git add im-app/ui/src/App.vue
git commit -m "feat(ui): wire unreadCount, messageOrder, scroll-stopped through App.vue to MessagePanel"
```

---

### Task 9: 端到端验证

**Files:**
- 无代码变更，仅运行验证

- [ ] **Step 1: Rust 编译**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo build -p im-store -p im-app`

Expected: 构建成功，无 warning

- [ ] **Step 2: 运行 store 测试**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo test -p im-store`

Expected: 全部 PASS

- [ ] **Step 3: TypeScript 类型检查**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx tsc --noEmit`

Expected: 无错误

- [ ] **Step 4: 运行现有前端测试**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npm test`

Expected: 所有现有测试 PASS（新组件无需新增测试，旧测试不被破坏即通过）

- [ ] **Step 5: 手动功能验证**
  - 启动应用，登录后查看消息列表
  - 确认老数据消息无蓝色左边框（已初始化为已读）
  - 收到新匹配消息时，卡片出现蓝色左边框
  - 点击浮窗按钮（↓ + 徽章），消息变正常样式，未读数归零
  - 点击排序切换按钮（↑/↓），消息顺序反转，再次点击恢复
  - 快速滚动消息列表后停止，未读消息自动标为已读
  - 切换黑白主题，所有新增样式正确适配

- [ ] **Step 6: 最终提交**

```bash
git add -A
git commit -m "feat: end-to-end verification of read/unread feature"
```
