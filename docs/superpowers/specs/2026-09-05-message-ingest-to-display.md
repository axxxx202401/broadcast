# 消息入库到展示完整流程

## 概述

本文档描述一条群消息从 TCP 推送到达，到写入 SQLite，再到前端展示到 UI 界面的完整生命周期。涉及两个子模块：**聊天监控**（通用消息流）和**开奖匹配**（特殊过滤）。

```
TCP 推送 → Protobuf 解码 → 微批 → 入库 SQLite
                                        ↓ 解密 + 开奖匹配
                                 Channel 批次 → Vue 索引 → 渲染
```

---

## 一、数据模型

### 1.1 数据库表

每个账号一个独立 SQLite 数据库，路径为 `~/.im-monitor/accounts/{uid}/im_monitor.db`。

**`messages` 表**

| 字段 | 类型 | 说明 |
|------|------|------|
| `msg_id` | INTEGER PRIMARY KEY | 服务端消息 ID |
| `group_id` | INTEGER NOT NULL | 群组 ID |
| `send_uid` | INTEGER NOT NULL | 发送者 UID |
| `msg_type` | INTEGER NOT NULL | 协议消息类型 |
| `content` | BLOB NOT NULL | 原始加密字节 |
| `send_time` | INTEGER NOT NULL | 服务端发送时间戳 |
| `content_md5` | TEXT | 内容摘要 |
| `stored_at` | INTEGER NOT NULL | 入库时 UTC 毫秒时间戳 |
| `raw_proto` | BLOB | Protobuf 序列化原包（用于附件解密等） |
| `matched` | INTEGER NOT NULL DEFAULT 0 | 开奖匹配标记：`1`=匹配，`0`=不匹配 |
| `content_text` | TEXT DEFAULT '' | 解密后明文；版本 0 明文消息直接存储 |

**`lottery_config` 表**

| 字段 | 类型 | 说明 |
|------|------|------|
| `uid` | INTEGER PRIMARY KEY | 账号 UID |
| `api_url` | TEXT NOT NULL DEFAULT '' | 开奖历史 API 地址 |
| `current_issues` | TEXT NOT NULL DEFAULT '[]' | JSON 数组，当前关注的期号列表 |
| `updated_at` | INTEGER NOT NULL | 最后更新时间 |

### 1.2 前端类型

**`MessageDto`** — 前端可见的消息对象：

```ts
interface MessageDto {
  msg_id: string          // 十进制字符串，避免 JS 精度损失
  group_id: string
  send_uid: string
  msg_type: number
  group_name: string      // 关联 groups.name，实时消息可能为空
  content_b64: string     // Base64 编码的原始加密字节
  decoded_content: DecodedMessageContent | null  // 解密后的结构化正文
  decode_error: string | null
  send_time: number
  content_md5: string
  stored_at: number | null  // 实时消息为 null（无回读）
  matched: number           // 1=匹配，0=不匹配
}
```

**`LotteryConfig`** — 开奖配置：

```ts
interface LotteryConfig {
  api_url: string
  current_issues: number[]   // 从 API 拉取的期号列表
}
```

---

## 二、入站消息处理（后端）

### 2.1 TCP 帧接收

**文件：** [im-app/src/commands/chat.rs](im-app/src/commands/chat.rs)

消息通过 TCP 长连接推送，协议格式为 `4字节长度 + Protobuf 变长长度 + 有效载荷`。两种关键帧类型：

- **1201** — 登录成功推送，包含 App 公钥
- **2202** — 群消息推送（`PushGroupMessage`），可含多条群消息
- **2205** — 群消息撤回（预留处理）

### 2.2 帧入队与大小限制

**入队预算：**

| 限制项 | 值 | 说明 |
|--------|-----|------|
| `MESSAGE_QUEUE_CAPACITY` | 64 条 | mpsc 缓冲槽数，满载时背压等待 |
| `MESSAGE_QUEUE_BYTE_BUDGET` | 32 MiB | 所有未处理帧正文总字节预算 |
| `MAX_QUEUED_MESSAGE_SIZE` | 8 MiB | 单帧最大字节数，超限拒绝 |

```
TCP 回调 → 校验帧大小 → 申请字节许可（Semaphore）→ mpsc::send(frame)
                                                        ↓ 满载则阻塞
                                                 run_message_worker
```

### 2.3 2202 帧处理循环

**核心函数：** `run_message_worker_with_effects`

```
loop {
  等待下一帧（或批截止时间到了）

  if 帧 == 1201:
    解码 PushLoginSuccessMessage → 发送 login_sender → 触发用户密钥同步

  if 帧 == 2202:
    ① Wire 结构预扫描（count_group_messages_before_decode）
       - 只维护游标+计数器，不分配 GroupMessage 对象
       - 超过 10,000 条顶层 group_msg 则丢弃整帧

    ② Prost 解码 → PushGroupMessage { group_msg: Vec<GroupMessage> }

    ③ 读取监控快照（monitoring_snapshot）

    ④ 逐条构建 PendingGroupMessage { message, monitored, frame_byte_permit }
       - 积累到 100 条或等待 25ms 后触发 flush

  if 帧 == 2205: 记录日志（预留）
  if 帧 == 其他:   debug 日志忽略
}
```

### 2.4 微批提交

**核心函数：** `flush_group_message_batch`

```
pending = [PendingGroupMessage, ...]

# 1. 分离监控/非监控消息
monitored_messages    = pending 中 monitored=true 的消息
非监控消息            = pending 中 monitored=false 的消息

# 2. 持久化监控消息（事务 + 解密 + 匹配 + 回执）
persisted = effects.persist_monitored_batch(monitored_messages)

# 3. 回执
if persisted:
  监控消息：发送 2102 回执（按群分组）
else:
  仅非监控消息：发送 2102 回执
  监控消息不回执

# 4. 投影排队（仅持久化成功的监控消息）
if persisted && 监控消息非空:
  projection_sender.send(ProjectionMessageBatch { messages, frame_byte_permits })
```

---

## 三、消息持久化与开奖匹配

### 3.1 入库（persist_monitored_batch）

**文件：** [im-app/src/commands/chat.rs:632-801](im-app/src/commands/chat.rs#L632-L801)

```rust
async fn persist_monitored_batch(&self, messages: &[im_proto::GroupMessage]) -> bool {
    // ──── 阶段 1：批量写入 SQLite（matched=0，content_text 暂留空）────
    let records: Vec<MessageRecord> = messages
        .iter()
        .map(|msg| stored_message_parts(msg).0)  // matched 硬写 0
        .collect();
    db.messages.insert_batch(&records).await?;
    // ON CONFLICT(msg_id) DO UPDATE — 幂等去重，同一消息多次推送只保留最新

    // ──── 阶段 2：解密 content_text（version > 0 时才需要）────
    if records 有任意一条 content_text 为空:
        获取解密密钥 → 逐条尝试解密 → UPDATE messages SET content_text = ? WHERE msg_id = ?

    // ──── 阶段 3：开奖匹配 ────
    读取 lottery_config（uid）
    if config 存在 且 current_issues 非空:
        for record in records:
            text = record.content_text
            is_matched = text.contains("开奖") && text 包含任意一个 issue
            if is_matched:
                UPDATE messages SET matched = 1 WHERE msg_id = ?
    return true
}
```

**关键设计：**
- `matched` 在入库时硬写 `0`，因为此时 `content_text` 可能尚未解密完成
- 解密和匹配在同一批次内串行完成，之后立即 UPDATE
- 历史消息（启动时加载）**不经过此路径**，它们的 `matched` 是入库时就已确定的历史值
- 没有 `recompute_matched_all` — config 变更不影响已有消息的 matched 状态

### 3.2 解密流程

两条路径均使用 `message_crypto.decode_group_message()`：

| 路径 | 场景 | 结果处理 |
|------|------|----------|
| `persist_monitored_batch` | 监控群实时消息入库后 | 更新 DB `content_text`，仅 version>0 的消息 |
| `publish_monitored_batch` | 向 Channel 发送前 | DTO 中填充 `decoded_content` |

`stored_message_parts()` 中对 `content_text` 的处理：
- `version == 0`（明文）：直接转为 UTF-8 字符串
- `version > 0`（加密）：置空字符串，后续解密回填

---

## 四、投影与实时推送

### 4.1 投影 Worker

**核心函数：** `run_message_projection_worker`

```
循环:
  从 projection_receiver 取 ProjectionMessageBatch
  if 连接取消: 退出

  publish_monitored_batch(messages):
    ① 有界并发（最多 8 路）解密每条消息 → 填充 DTO.decoded_content
    ② 从 DB 批量 SELECT msg_id, matched WHERE msg_id IN (...)
       修正 DTO.matched（覆盖 stored_message_parts 中的硬写 0）
    ③ 通过 Tauri Channel 发送 batch<Vec<MessageDto>> 给前端
```

**注意：** 即使解密失败，消息仍通过 Channel 发送（`decode_error` 字段记录原因），仅 `matched` 值为 `0`。

### 4.2 连接取消时的行为

| 取消时机 | 后果 |
|----------|------|
| 在 `persist_monitored_batch` 之前 | 消息不入库，不回执 |
| 在 `persist_monitored_batch` 完成后 | 消息已入库，不回执（监控消息），不投影 |
| 在投影排队中（未发送到 Channel） | 消息入库但前端看不到，被丢弃 |
| 在 Channel 发送后 | 成功投递到前端 |

---

## 五、前端展示

### 5.1 实时消息通道

**文件：** [im-app/ui/src/composables/useMonitor.ts](im-app/ui/src/composables/useMonitor.ts)

```typescript
// App.vue onMounted → useMonitor.onMounted
messageChannel = new Channel<MessageDto[]>()
messageChannel.onmessage = (batch) => {
  // 按当前选中群过滤
  const visible = selectedGroupId
    ? batch.filter(m => m.group_id === selectedGroupId)
    : batch

  if (activeOlderRequest) {
    // 正在向上翻页：缓冲到 bufferedRealtimeIndex，不直接改视图
    bufferedRealtimeIndex.merge(visible, 'keep-latest')
    return
  }

  mergeRealtimeAndPublish(visible)
}
```

**`mergeRealtimeAndPublish` 逻辑：**
```
mergeAndPublishMessages(incoming, 'keep-latest'):
  messageIndex.mergeWithResult(incoming, 'keep-latest')
  // 以 msg_id 去重，按 send_time 升序排列
  // 若已有消息在视口外，裁掉最旧的部分

messages.value = messageIndex.snapshot()  // 触发 Vue 响应式更新
```

### 5.2 历史消息加载

```typescript
// 连接成功后自动加载，或用户手动选择群组
async function loadMessages(groupId: string | null) {
  const requestId = ++messageRequestId        // 防止旧响应覆盖新请求
  clearMessages()                              // 清空当前视图
  messagesLoading.value = true

  const history = await api.getMessages(groupId ?? undefined, undefined, 200)
  // 首次加载，不限 matched_only，前端用 showMatchedOnly 控制显示

  mergeAndPublishMessages(history.messages)    // 合并到索引
  nextMessageCursor.value = history.nextCursor
  hasOlder.value = history.hasMore
}

// 上翻加载更多
async function loadOlderMessages() {
  const cursor = nextMessageCursor.value
  const history = await api.getMessages(groupId, cursor, 200)
  mergeAndPublishMessages(history.messages, 'keep-earliest')  // 保留旧消息
  nextMessageCursor.value = history.nextCursor
  hasOlder.value = history.hasMore
}
```

### 5.3 过滤显示

```typescript
// App.vue 中 showMatchedOnly 默认为 true
const filteredMessages = computed(() =>
  showMatchedOnly.value
    ? messages.value.filter(m => m.matched !== 0)
    : messages.value,
)
```

**重要：** 前端过滤基于内存中的 `messages.value`，**不是在数据库层过滤**。每次 `loadMessages` 加载历史时都带回所有消息（含 `matched=0`），由前端根据 `showMatchedOnly` 决定是否显示。`get_messages` 命令支持 `matched_only` 参数，但当前前端仅在 Keyset 翻页时不使用（始终返回全部，前端过滤）。

### 5.4 Keyset 分页

游标使用 `(send_time, msg_id)` 复合键，避免同一时间戳多消息的重叠问题：

```sql
-- 翻页查询（降序）
WHERE (send_time < ? OR (send_time = ? AND msg_id < ?))
ORDER BY send_time DESC, msg_id DESC
LIMIT ?
```

---

## 六、开奖配置管理

### 6.1 启动初始化

**文件：** [im-app/ui/src/composables/useLottery.ts](im-app/ui/src/composables/useLottery.ts)

```
App.vue onMounted
  └─ accounts.restore()
       └─ applyRestoreOutcome → monitor.acceptLogin()
            └─ syncConnectionStatus() + loadMessages()
            └─ lottery.runPrefetch()  ← 开奖模块独立运行

runPrefetch():
  if not loggedIn: return
  prefetchWithDefault([])
    └─ config.current_issues.length > 0 ? 跳过（已有配置）
    └─ fetchHistory()                    ← 先拉 API 拿实际期号
    └─ drawHistory.length > 0
         └─ setLotteryConfig(api_url, issues)  ← 再用实际期号保存
              └─ loadConfig()
    └─ schedulePoll()  ← 每 30 秒轮询
```

### 6.2 开奖匹配条件

```
一条消息被标记 matched=1，当且仅当：
  ① lottery_config 存在 且 current_issues 非空
  ② content_text 包含 "开奖"
  ③ content_text 包含 current_issues 中任意一个期号
```

`content_text` 的获取优先级：
1. `version == 0`（明文）：直接从原始字节转 UTF-8，入库即有值
2. `version > 0`（加密）：解密后回填，若解密失败则为空字符串

---

## 七、完整时序图

```
[服务端]          [TCP]         [Worker]         [SQLite]         [Channel]         [Vue]
   │               │               │                 │                │                │
   │── 2202帧 ───▶│               │                 │                │                │
   │               ├── 校验尺寸 ──▶│                 │                │                │
   │               │── Proto decode ┐               │                │                │
   │               │                ▼               │                │                │
   │               │         PendingGroupMessages   │                │                │
   │               │         (accumulating…)        │                │                │
   │               │                                │                │                │
   │               │  ── batch 满/25ms ───────────▶ │                │                │
   │               │                                │                │                │
   │               │                        ┌── persist_monitored_batch ──────────┐   │
   │               │                        │  INSERT batch (matched=0)           │   │
   │               │                        │  UPDATE content_text (decrypt)      │   │
   │               │                        │  UPDATE matched (lottery check)     │   │
   │               │                        └─────────────────────────────────────┘   │
   │               │                                │                                │
   │               │                        ┌── publish_monitored_batch ──────────┐   │
   │               │                        │  Decrypt each message (max 8 parallel)│   │
   │               │                        │  SELECT matched FROM DB              │   │
   │               │                        │  Channel.send(batch)                 │   │
   │               │                        └─────────────────────────────────────┘   │
   │               │                                │         │                      │
   │               │                                │         │  MessageDto[]         │
   │               │                                │         └─────────────────────▶│
   │               │                                │                       mergeAndPublish
   │               │                                │                       messages.value = ...
   │               │                                │                                       ▼
   │               │                                │                          UI 渲染（filteredMessages）
```

---

## 八、关键设计决策

| 决策 | 原因 |
|------|------|
| `matched` 入库时硬写 0，后续 UPDATE | 入库时 `content_text` 可能尚未解密（version>0），无法判断匹配 |
| 删除 `recompute_matched_all` | 历史消息的 matched 在入库时已确定；config 变更不应重扫全表 |
| `prefetchWithDefault` 先 fetchHistory 再 save | 避免空数组覆盖已有配置（上一版本时序 bug） |
| DB 不在查询时使用 `matched_only` 过滤 | 前端用内存 `showMatchedOnly` 控制；历史加载全量更灵活 |
| Keyset 分页用 `(send_time, msg_id)` 复合键 | 单字段 `send_time` 无法区分同一时刻的多条消息 |
| 投影 Worker 独立于消息 Worker | 慢解密不阻塞收帧，保持 TCP 背压可控 |
| 监控消息入库失败不回执 | 服务端会重推，重试时可重新持久化 |
| 非监控消息始终回执 | 不影响监控，减少服务端重推 |

---

## 九、相关文件索引

| 文件 | 职责 |
|------|------|
| [im-app/src/commands/chat.rs](im-app/src/commands/chat.rs) | TCP 帧处理、入库、解密、匹配、Channel 推送 |
| [im-app/src/commands/lottery.rs](im-app/src/commands/lottery.rs) | 开奖配置 GET/SET、历史 API 调用 |
| [im-store/src/message.rs](im-store/src/message.rs) | 消息 CRUD、keyset 分页查询 |
| [im-store/src/lottery_config.rs](im-store/src/lottery_config.rs) | 开奖配置 CRUD |
| [im-store/src/schema.rs](im-store/src/schema.rs) | SQLite 表结构定义 |
| [im-app/ui/src/composables/useMonitor.ts](im-app/ui/src/composables/useMonitor.ts) | 前端消息状态管理（索引、分页、过滤） |
| [im-app/ui/src/composables/useLottery.ts](im-app/ui/src/composables/useLottery.ts) | 前端开奖配置加载、轮询、历史拉取 |
| [im-app/ui/src/services/tauri.ts](im-app/ui/src/services/tauri.ts) | Tauri IPC 调用封装 |
| [im-app/ui/src/types/im.ts](im-app/ui/src/types/im.ts) | 前端 TypeScript 类型定义 |
