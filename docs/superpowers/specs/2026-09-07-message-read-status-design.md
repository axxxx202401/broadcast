# 消息已读/未读功能设计

## 概述

为监控面板的消息列表增加已读/未读状态管理，支持：

1. 只有匹配消息（`matched ≠ 0`）才会有未读状态；不匹配的消息入库即视为已读
2. 消息卡片以视觉样式区分已读/未读
3. 浮动按钮「一键到最新」：点击后标记当前群所有匹配消息为已读，并滚动到最新消息
4. 消息排序配置：顶部为最新消息（默认）或底部为最新消息
5. 所有样式跟随黑白主题切换

## 数据库变更

### `messages` 表新增列

```sql
ALTER TABLE messages ADD COLUMN read_at INTEGER NOT NULL DEFAULT 0;
```

- `read_at = 0` 表示未读（老数据全为 0，但老数据均为匹配消息，由前端初始化时批量标为已读一次）
- `read_at > 0` 表示已读时间（Unix 毫秒时间戳）
- 不匹配消息（`matched = 0`）的 `read_at` 无意义，但统一存储不影响查询性能

### 迁移处理

在 `im-store/src/schema.rs` 的 `SqliteStore::new` 中检查 `read_at` 列是否存在，若不存在则执行 ALTER。

## 后端接口

### 新增 Tauri 命令：`mark_group_read`

```rust
#[tauri::command]
async fn mark_group_read(
    state: State<'_, AppState>,
    group_id: i64,
    to_msg_id: i64,
) -> Result<usize, String>
```

**语义**：将满足条件的消息标记为已读。支持两种粒度：

- **单群模式**：`group_id = ? AND matched != 0 AND read_at = 0 AND msg_id <= to_msg_id`
- **全群模式**：`matched != 0 AND read_at = 0 AND msg_id <= to_msg_id`（不限制 group_id）

更新 `read_at` 为当前 UTC 毫秒时间戳，返回受影响的行数。

**参数**：`group_id` 为 `null` 时表示全群模式。

**调用方**：
1. 前端点击「一键到最新」浮窗按钮时触发，`to_msg_id` 取当前最新消息的 `msg_id`
2. 前端人工滚动停止时触发，`to_msg_id` 取视口内收集到的最大未读 msg_id

### 新增 Tauri 命令：`get_read_cursor`（可选，供前端判断是否需要标已读）

```rust
#[tauri::command]
async fn get_read_cursor(
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<Option<i64>, String>
```

返回该群最后一条已读消息的 `msg_id`（`read_at > 0` 的最大 `msg_id`）；若无已读记录返回 `None`。

前端在每次触发 `mark_group_read` 前可先比较 `cursor_msg_id` 与目标 `to_msg_id`，若相同则跳过，避免无效写库。

### 修改现有查询

`get_by_group` 和 `get_recent` 的 SELECT 列表中补充 `m.read_at`，映射到 `MessageRow.read_at`。

`get_messages` 命令返回值 `MessageDto` 需包含 `read_at` 字段。

## 前端类型

### `types/im.ts`

```typescript
export interface MessageDto {
  // ... 现有字段
  /** 已读时间戳（Unix ms）；0 表示未读。 */
  read_at: number
}
```

### `services/tauri.ts`

```typescript
markGroupRead: (groupId: string | null, toMsgId: string) =>
  invoke<number>('mark_group_read', { groupId, toMsgId }),
```

## 组合式函数：`useMonitor.ts`

### 新增状态

```typescript
const unreadCount = ref(0)
```

### 计算属性

```typescript
const unreadCount = computed(() =>
  filteredMessages.value.filter(m => m.matched !== 0 && m.read_at === 0).length,
)
```

### 新方法：`markAllAsRead(toMsgId?: string)`

```typescript
async function markAllAsRead(toMsgId?: string) {
  if (unreadCount.value === 0) return
  // 全群模式 groupId=null，单群模式传当前选中群
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

### 人工滚动处理：滚动缓冲 + 停止时批量标记

`MessagePanel` 在 `@scroll` 中收集视口内所有未读 msg_id 到缓冲数组（全群模式下混存，单群模式下按群分组但取各组最大 msg_id）。用 300ms `setTimeout` 防抖，每次触发时：取缓冲中最大 `msg_id`，发出 `'scroll-stopped'` 事件携带 `{ maxMsgId, groupId }`；`useMonitor` 收到后调 `markAllAsRead(maxMsgId)`，清空缓冲。

### 排序方向配置

```typescript
const MESSAGE_ORDER_KEY = 'im-message-order'
const messageOrder = ref<'newest-top' | 'newest-bottom'>('newest-top')

function toggleMessageOrder() {
  const next = messageOrder.value === 'newest-top' ? 'newest-bottom' : 'newest-top'
  messageOrder.value = next
  localStorage.setItem(MESSAGE_ORDER_KEY, next)
  // 切换排序后重新加载
  void loadMessages(selectedGroupId.value)
}

// 初始化时读取已保存值
try {
  const stored = localStorage.getItem(MESSAGE_ORDER_KEY)
  if (stored === 'newest-bottom') messageOrder.value = 'newest-bottom'
} catch {}
```

## 组件改动

### `MessagePanel.vue`

**新增 props**：

```typescript
const props = withDefaults(defineProps<{
  // ... 现有 props
  unreadCount?: number
  messageOrder?: 'newest-top' | 'newest-bottom'
}>(), {
  unreadCount: 0,
  messageOrder: 'newest-top',
})

const emit = defineEmits<{
  // ... 现有 events
  'mark-read': []
  'toggle-order': []
}>()
```

**排序方向对应的 CSS**：
- `newest-top`（默认）：`flex-direction: column`（虚拟列表 normal flow，CSS order 不变）
- `newest-bottom`：虚拟列表使用 `reverse: true` 选项，或 CSS `flex-direction: column-reverse`

TanStack Virtualizer 原生支持 `reverse: true`，传入后列表从底部开始渲染，最新消息在底部。

**浮窗按钮**：圆形浮动按钮，位于视口右下角（`newest-bottom`）或右上角（`newest-top`），仅在 `unreadCount > 0` 且用户未处于视口底部时显示。SVG 向下箭头图标，未读数以红色上标徽章叠加在图标右上角。点击后触发 `mark-read` 事件，同时滚动到最新消息位置。

```html
<button
  v-if="unreadCount > 0 && !isAtBottom"
  class="unread-float-btn"
  @click="emit('mark-read')"
  :aria-label="`标记全部已读，共 ${unreadCount} 条未读`"
>
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 5v14M5 12l7 7 7-7"/>
  </svg>
  <span v-if="unreadCount > 0" class="unread-float-btn__badge">{{ unreadCount > 99 ? '99+' : unreadCount }}</span>
</button>
```

检测是否在底部（视口 scrollTop 接近 scrollHeight - clientHeight）来决定是否隐藏浮窗。

**人工滚动触发已读标记**：

滚动过程中前端持续收集当前视口内所有 `matched ≠ 0 && read_at === 0` 的消息 ID，存入缓冲数组。滚动停止后（用 `requestIdleCallback` 或 `setTimeout` 300ms 无新滚动事件触发），将缓冲数组去重、按时间升序取最大 `msg_id`，一次性调用后端 `markGroupRead(group_id, maxMsgId)` 批量标记。

这样无论用户怎么滑，只会在真正停下来的时候发一次请求，不会高频触发 IPC。

**排序切换按钮**（放在 header 右侧，与现有 `icon-button` 风格一致）：

```html
<button
  class="icon-button order-toggle-btn"
  @click="emit('toggle-order')"
  :title="messageOrder === 'newest-top' ? '切换到从下往上' : '切换到从上往下'"
  aria-label="切换消息排序方向"
>
  <!-- newest-top：箭头朝上 -->
  <svg v-if="messageOrder === 'newest-top'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 19V5M5 12l7-7 7 7"/>
  </svg>
  <!-- newest-bottom：箭头朝下 -->
  <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 5v14M19 12l-7 7-7-7"/>
  </svg>
</button>
```

**虚拟列表 reverse 支持**：

```typescript
const virtualizerOptions = computed(() => ({
  // ... 现有配置
  reverse: props.messageOrder === 'newest-bottom',
}))
```

### `App.vue`

传递新 props 和事件：

```html
<MessagePanel
  :unread-count="monitor.unreadCount"
  :message-order="monitor.messageOrder"
  @mark-read="monitor.markAllAsRead"
  @toggle-order="monitor.toggleMessageOrder"
/>
```

## 样式规范

### 未读消息卡片

```css
/* 深色主题 */
.message-card--unread {
  border-left: 3px solid var(--info);
  background-color: rgba(88, 166, 255, 0.04);
}

/* 亮色主题 */
[data-theme="light"] .message-card--unread {
  border-left: 3px solid var(--info);
  background-color: rgba(9, 105, 218, 0.05);
}
```

### 浮窗按钮

图标按钮，圆形或圆角正方形，带红色未读数上标徽章。

```css
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
  box-shadow: 0 2px 10px rgba(0,0,0,0.35);
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
```

`newest-top` 模式下将 `bottom` 改为 `top: 20px`，`right` 保持 `20px`。

亮色主题下 badge 颜色不变（`var(--danger)`），阴影略微加深。

### 排序切换按钮

复用现有 `.icon-button` 样式（38×38，圆角 `var(--radius)`，bg-elevated-2 背景，hover 时 elevated），新增 `.order-toggle-btn` 仅控制 SVG 尺寸：

```css
.order-toggle-btn svg {
  width: 16px;
  height: 16px;
}
```

与 `eye-toggle svg` 保持一致的尺寸规范。

## 消息顺序逻辑

当前消息从最新消息向旧消息排列（降序）。

- **`newest-top`**：正常顺序，最旧消息在顶部，最新在底部（与现有行为一致）
- **`newest-bottom`**：倒序，最新消息在底部

注意：现有 `MessagePanel.vue` 的虚拟列表 watcher 在收到新实时消息时会自动滚底，这是符合 `newest-bottom` 直觉的行为。`newest-top` 模式下收到新消息不应自动滚底（需切换 watcher 逻辑，或仅在 `newest-bottom` 时启用自动滚底）。

实际影响：切换回 `newest-top` 后，新消息出现在顶部，用户需要手动滚到顶部查看。

## 文件变更清单

| 文件 | 改动类型 |
|------|---------|
| `im-store/src/schema.rs` | 加 `read_at` 列 + 迁移检查 |
| `im-store/src/message.rs` | `MessageRow` 加字段，SQL 补列 |
| `im-app/src/commands/chat.rs` | 新增 `mark_group_read` 命令（可选 `get_read_cursor`） |
| `im-app/ui/src/types/im.ts` | `MessageDto` 加 `read_at` |
| `im-app/ui/src/services/tauri.ts` | 加 `markGroupRead` |
| `im-app/ui/src/composables/useMonitor.ts` | 加 `unreadCount`、`markAllAsRead`、`handleScrollNearBottom`、`messageOrder` |
| `im-app/ui/src/components/MessagePanel.vue` | 浮窗按钮、排序切换、reverse 支持、人工滚动事件 |
| `im-app/ui/src/styles/console.css` | 未读样式、浮窗样式、主题适配 |
| `im-app/ui/src/App.vue` | 传递新 props/events |

## 边界情况

1. **老数据初始化**：首次升级后所有现有消息 `read_at=0`，应在连接成功后（`message_keys_ready`）前端调一次 `markGroupRead(null, 最大msgId)` 全量标已读，避免老数据全部显示为未读。
2. **匹配消息切换**：用户修改开奖规则后部分消息 `matched` 会变，已读状态不受影响（`read_at` 只与时间相关）。
3. **多群视图**：`unreadCount` 统计的是当前可见消息的未读数（单群模式只看该群，全群模式看所有监控群）。
4. **浮窗按钮位置**：`newest-bottom` 模式浮窗在底部，`newest-top` 模式浮窗在顶部（用 CSS `top`/`bottom` 动态切换）。
5. **全群视图已读**：全局统一追踪，不区分群；缓冲数组混存所有群未读 msg_id，停止时取最大值一次标记。
6. **人工滚动防抖**：`@scroll` 时收集视口内未读 msg_id 到缓冲数组，300ms 无新滚动事件后取最大 msg_id 一次性调后端，不再高频触发 IPC。
