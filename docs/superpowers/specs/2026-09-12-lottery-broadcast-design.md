# 开奖广播消息功能设计（修订版 v3）

## 概述

在 IM Monitor 中，当检测到开奖接口返回新的期号时，自动按照用户可编辑的文本模板组装消息，并通过现有 TCP 长连接向所有受监控群组广播。消息以 `message_id=2101`（`SendGroupMessage` protobuf）发送至服务端，同时写入现有 `messages` 表（`matched=1`），使广播消息与接收到的匹配消息共用相同的查询和展示路径，在消息面板中统一显示。

发送后需等待服务端推送 `message_id=2201`（`PushGroupMessageSendSuccess`）确认才算真正成功，因此每条广播消息具有明确的状态生命周期：`sending → success | failure`。**广播消息先发「发送中」状态入库，再发出，最后由 2201 或超时更新状态**。UI 消息面板直接展示该消息（`matched=1`），并通过 `broadcast_status` 字段区分发送中/成功/失败。

广播功能可通过开关控制启停。默认模板随功能一起内置，用户首次使用时即可获得可用的消息内容。

---

## 数据模型变更

### 新增表：`lottery_message_templates`

```sql
CREATE TABLE IF NOT EXISTS lottery_message_templates (
    id          INTEGER PRIMARY KEY CHECK(id = 1),
    template    TEXT    NOT NULL DEFAULT '加拿大 PC 第${preDrawIssue}期开奖结果：\n${preDrawCode}=${sumNum}  ${sumBigSmall}${sumSingleDouble}${patternDesc}\n近10期：${lastTenDraws}\n顶赔对赌\n大小单双：2.17\n小双大单：4.32\n大双小单：4.76\n\n大将军CU交易1群 @bkkn7mqkn0\n大将军CU交易2群 @93158hello\n大将军CU交易3群 @az8t88eeqg\n大将军上押担保频道 @flyin3037s\n大将军担保官方网站 https://djidb.com\n\n——团队担保信至上服务至上——',
    enabled     INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL
);
```

- `id` 固定为 `1`（每个账号最多一条模板）。
- `template` 为用户可编辑的文本模板；默认值见"默认模板"一节。
- `enabled`：`1` 表示广播功能开启，`0` 表示关闭；对应 UI 中的开关。
- `updated_at` 为 UTC Unix 毫秒时间戳。

### 扩展 `messages` 表：新增 `broadcast_status` 列

广播消息写入现有 `messages` 表，`matched=1`，新增 `broadcast_status` 列记录发送状态：

```sql
-- 迁移：为 messages 表追加 broadcast_status 列
ALTER TABLE messages ADD COLUMN broadcast_status INTEGER NOT NULL DEFAULT 0;
-- 说明：0=发送中(sending)，1=成功(success)，2=失败(failed)
-- 仅在广播消息上非零，普通接收消息保持默认值 0，UI 据此区分广播消息。
```

迁移逻辑参考现有 `migrate_messages_matched` 模式：先 `pragma_table_info` 检查列是否存在，不存在则执行 `ALTER TABLE`。

- **`broadcast_status = 0`（发送中）**：消息已入库但尚未收到 2201 确认，也未超时。UI 展示为「发送中」。
- **`broadcast_status = 1`（成功）**：收到 2201 确认。UI 展示为「成功」。
- **`broadcast_status = 2`（失败）**：TCP 发送失败，或发送后 30 秒内未收到 2201 确认（超时）。UI 展示为「失败」。

广播消息的 `msg_id` 由客户端生成（基于时间戳的 i64），`flag` = `uid * 1_000_000_000 + timestamp_ms`，通过 `flag` 字段匹配 2201 回调并更新 `broadcast_status`。

### 扩展 `AppConfig`

在 `im-common/src/config.rs` 的 `AppConfig` 中新增两个布尔配置项（由编译期环境变量注入，运行时固定）：

```rust
/// 是否将收到的消息写入 SQLite messages 表。
/// false 时收到服务端消息直接发送 2102 回执，不写库，也不执行匹配逻辑。
#[serde(default)]
pub persist_received_messages: bool,
/// 是否对收到的群消息执行开奖匹配并设置 matched=1。
/// false 时仍入库（若 persist_received_messages 为 true），但不设置 matched。
#[serde(default)]
pub match_lottery_messages: bool,
```

两个字段默认值均为 `true`（保持当前行为），通过编译期环境变量注入：
- `IM_PERSIST_RECEIVED_MESSAGES`（可选，默认 `true`）
- `IM_MATCH_LOTTERY_MESSAGES`（可选，默认 `true`）

**`persist_received_messages=false` 时的行为**：收到 2202 消息后，直接发送 2102 回执给服务端（保证服务端不重推），跳过 `insert_batch` 和 lottery 匹配，消息不进入 UI 实时 Channel。

**`match_lottery_messages=false` 时的行为**：消息正常入库（`matched` 默认为 0），跳过 lottery 匹配逻辑（不执行 `UPDATE messages SET matched = 1`）。

### 扩展 `DrawItem`

当前 [`im-http/src/lottery.rs`](../im-http/src/lottery.rs) 的 `DrawItem` 仅包含 `pre_draw_issue` 和 `pre_draw_time`。需要扩展字段以支持模板渲染：

```rust
/// 大/小/中枚举：1=大，0=小，-1=中。
fn big_small_to_str(v: i64) -> &'static str {
    match v { 1 => "大", 0 => "小", _ => "中" }
}
/// 单/双/中枚举：1=单，0=双，-1=中。
fn single_double_to_str(v: i64) -> &'static str {
    match v { 1 => "单", 0 => "双", _ => "中" }
}
```

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct DrawItem {
    #[serde(rename = "preDrawIssue")]
    pub pre_draw_issue: i64,
    #[serde(rename = "preDrawTime")]
    pub pre_draw_time: String,
    #[serde(rename = "preDrawCode", default)]
    pub pre_draw_code: String,          // API 原始值：逗号分隔，如 "8,8,2"（渲染时补零转 "+" 连接）
    #[serde(rename = "sumNum", default)]
    pub sum_num: i64,                   // 和值
    #[serde(rename = "sumBigSmall", default)]
    pub sum_big_small: i64,             // 1=大，0=小，-1=中
    #[serde(rename = "sumSingleDouble", default)]
    pub sum_single_double: i64,         // 1=单，0=双，-1=中
}
```

模板渲染时：
- `${preDrawCode}`：将 API 返回的逗号分隔字符串（`"8,8,2"`）替换为 `+` 连接，**每位号码补零至两位数**（`"08+08+02"`）。
- `${sumBigSmall}`：调用 `big_small_to_str()` 转换为汉字。
- `${sumSingleDouble}`：调用 `single_double_to_str()` 转换为汉字。

> 字段命名基于 API 响应结构推断；若实际字段名不同，需对照真实接口返回调整 `serde(rename = ...)`。

---

## Protobuf 变更

在 [`proto/broadcast.proto`](../proto/broadcast.proto) 中追加：

```protobuf
// 发送群聊消息 2101
message SendGroupMessage {
    GroupMessage group_msg = 1;
    int64 flag = 2;
}

// 推送群聊消息发送成功 2201
message PushGroupMessageSendSuccess {
    int64 flag = 1;         // 客户端发送时传入的 flag
    int64 msg_id = 2;       // 服务端分配的消息 ID
    int64 group_id = 3;     // 群聊 ID
    int32 member_count = 4; // 群成员数量
    int32 snapchat_time = 5; // 阅后即焚设置时间
    int64 sent_over_time = 6; // 发送完成时间
}
```

重新运行构建脚本后，在 [`im-proto/src/lib.rs`](../im-proto/src/lib.rs) 的 `pub use pb::{...}` 中导出 `SendGroupMessage` 和 `PushGroupMessageSendSuccess`。

---

## 架构设计

### 后台广播任务

广播任务随聊天连接启动，跟随连接生命周期运行；连接断开或用户切换账号时自动停止。

#### 任务结构

```
start_lottery_broadcast(
    context: ConnectionContext,
    auth_session: AuthSession,
    generation: u64,
    attempt_id: u64,
    generation_cancellation: CancellationToken,
    connection_cancellation: CancellationToken,
    sender: ChatSender,
)
```

内部循环：

```
loop {
    tokio::select! {
        biased;
        _ = generation_cancellation.cancelled() => return,
        _ = connection_cancellation.cancelled() => return,
        _ = ticker.tick() => {}  // 每 20 秒触发一次
    }
    if let Err(e) = run_one_broadcast_cycle(&context, &auth_session).await {
        tracing::warn!(error = %e, "Lottery broadcast cycle failed");
    }
}
```

#### 单次轮询逻辑（`run_one_broadcast_cycle`）

1. 读取 `lottery_template.get(uid)`：模板为空或 `enabled=0` 则跳过本轮。
2. 读取 `lottery_config.get(uid)`（获取 `api_url` 与 `current_issues`）。
3. 调用 `im_http::lottery::fetch_draw_history(url)` 获取最新历史列表（降序）。
4. 找出 `draw.pre_draw_issue` 不在 `current_issues` 中的新条目（取第一条，即最新一期）。
5. 若无新期号，跳过本轮。
6. 若存在新期号：
   a. 从步骤3获取的历史列表中取前10条，拼接 `${lastTenDraws}` 字符串（各条 `sum_num` 空格分隔）。
   b. 用 `DrawItem` 字段及 `${lastTenDraws}` 填充模板占位符，生成消息文本。
   c. 读取当前监控群组列表（`monitoring_groups.read().await`）。
   d. 遍历每个监控群组：
      - 构造 `GroupMessage` protobuf：`send_uid=uid`, `group_id`, `msg_type=text(0)`, `content=文本字节(UTF-8)`，并生成 `msg_id`（基于时间戳的 i64，与 flag 生成方式对齐）。
      - 构造 `SendGroupMessage` protobuf：`group_msg`, `flag = uid * 1_000_000_000 + timestamp_ms`。
      - **先将消息以 `broadcast_status=0`（发送中）写入 `messages` 表**（`matched=1`，`content_text` 为明文文本），获得已分配的 `msg_id`。
      - 序列化后通过 `sender.send_cancellable(2101, &bytes, &cancellation, timeout)` 发送。
      - **发送失败时**：更新 `messages.broadcast_status=2`（failed），记录 `error_msg` 到日志。
      - **发送成功时**：启动一个 30 秒超时定时器（`tokio::time::timeout`），超时后若仍未收到 2201，则将 `messages.broadcast_status=2`（failed）。
   e. 将新期号追加到 `current_issues`，更新 `lottery_config.updated_at`，upsert 回数据库。

### 2201 回调处理

在聊天帧处理循环中新增对 `message_id=2201` 的处理：

```rust
im_chat::heartbeat::PUSH_GROUP_MESSAGE_SEND_SUCCESS => {
    let ack = match im_proto::PushGroupMessageSendSuccess::decode(frame.content.as_slice()) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to decode PushGroupMessageSendSuccess: {e}");
            continue;
        }
    };
    let flag = ack.flag;
    // 从 flag 反推 uid：uid = flag / 1_000_000_000
    let uid = flag / 1_000_000_000;
    // 更新 messages 表中对应 flag 的记录：broadcast_status=1, msg_id=ack.msg_id
    if let Err(e) = db.update_broadcast_status_by_flag(uid, flag, 1).await {
        tracing::warn!("Failed to mark broadcast success: {e}");
    }
}
```

**超时处理**：每条广播消息在发送成功后启动一个 30 秒 `tokio::time::timeout` 协程，超时后调用 `db.update_broadcast_status_by_flag(uid, flag, 2)` 将状态改为失败。30 秒超时结束后检查状态是否为 0（发送中），若是则标记失败；若已是 1（成功）则不覆盖。

> **注意**：`flag = uid * 1_000_000_000 + timestamp_ms`，可通过 `flag / 1_000_000_000` 还原 `uid`。数据库查询按 `(uid, flag)` 精确匹配，避免跨账号误更新。

### 入库与匹配开关控制

在 `persist_monitored_batch` 中新增两处条件判断，由 `AppConfig` 中的新字段控制：

1. **`persist_received_messages=false`**：跳过整批消息的 INSERT，不推 Channel，直接发 2102 回执。
2. **`match_lottery_messages=false`**：正常 INSERT（`matched=0`），跳过 lottery 匹配更新。

### 前端命令

| 命令 | 参数 | 返回值 |
|------|------|--------|
| `get_lottery_template` | — | `LotteryTemplateDto { template: String, enabled: bool }` |
| `set_lottery_template` | `template: String, enabled: bool` | `Result<(), String>` |
| `get_broadcast_send_log` | `limit: Option<u32>` | `Vec<BroadcastSendLogDto>`（从 `messages` 表查询 `broadcast_status != 0` 且 `matched=1` 的记录）|

新命令注册于 [`im-app/src/main.rs`](../im-app/src/main.rs) 的 `invoke_handler`。

### 前端 UI

在现有 `LotteryPanel.vue` 中新增：

1. **广播开关**：一个 toggle switch，控制模板的 `enabled` 字段，即时生效（开启后立即开始广播，关闭则停止）。
2. **模板编辑器**：多行文本框，展示并允许编辑模板；提供占位符提示列表。
3. **发送状态日志**：展示最近若干条广播发送记录，每条显示群组、期号、状态（发送中/成功/失败）、时间。
4. **环境配置区**（设置面板或广播面板内）：
   - 「消息入库」开关：对应 `persist_received_messages`，关闭后收到的消息不入库、不进 Channel，直接发 2102 回执。
   - 「消息匹配」开关：对应 `match_lottery_messages`，关闭后收到的消息不再进行开奖匹配，`matched` 不设置为 1。

---

## 模板占位符

| 占位符 | 含义 | 示例值 |
|--------|------|--------|
| `${preDrawIssue}` | 期号 | `3480990` |
| `${preDrawCode}` | 开奖号码（`+` 连接，每位补零至两位数）；由 API 逗号格式转换而来 | `08+08+02` |
| `${sumNum}` | 和值 | `18` |
| `${sumBigSmall}` | 大/小/中（`1→大，0→小，-1→中`） | `大` |
| `${sumSingleDouble}` | 单/双/中（`1→单，0→双，-1→中`） | `双` |
| `${lastTenDraws}` | 近10期和值列表（空格分隔，降序最新在前） | `15 16 10 05 13 21 26 08 07 08` |

**条件占位符**（根据三位号码的组合特征动态展示）：

| 占位符 | 触发条件 | 输出 | 未触发 |
|--------|----------|------|--------|
| `${patternDesc}` | 三数相同（如 `8,8,8`） | `豹子` | 空字符串 |
| `${patternDesc}` | 三数连续且非 `8,9,0`/`9,0,1` 环连（如 `3,4,5` 排序后差值均为1） | `顺子` | 空字符串 |
| `${patternDesc}` | 恰好两数相同，非豹子（如 `8,8,2`） | `对子` | 空字符串 |

> 三个条件互斥，只会输出其中一个值或为空。默认模板中的「对子」字面量在渲染后由 `${patternDesc}` 替代，由后端根据实际开奖号码动态计算。`${lastTenDraws}` 由后端从历史数据中取最近10条的 `sum_num` 字段拼接而成。

**顺子判断规则**：
1. 将三个号码升序排列。
2. 检查相邻差值是否均为 1（如 `3,4,5` → `4-3=1, 5-4=1` → 顺子）。
3. 特殊情况排除：`8,9,0`（排序后为 `0,8,9`，差值非全1）、`9,0,1`（排序后为 `0,1,9`，差值非全1）——这两组不视为顺子。

---

## 默认模板

功能上线时内置以下模板（作为 `lottery_message_templates.template` 的默认值）：

```
加拿大 PC 第${preDrawIssue}期开奖结果：
${preDrawCode}=${sumNum}  ${sumBigSmall}${sumSingleDouble}${patternDesc}
近10期：${lastTenDraws}
顶赔对赌
大小单双：2.17
小双大单：4.32
大双小单：4.76

大将军CU交易1群 @bkkn7mqkn0
大将军CU交易2群 @93158hello
大将军CU交易3群 @az8t88eeqg
大将军上押担保频道 @flyin3037s
大将军担保官方网站 https://djidb.com

——团队担保信至上服务至上——
```

---

## 文件改动清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `proto/broadcast.proto` | 新增 | 添加 `SendGroupMessage` 和 `PushGroupMessageSendSuccess` 消息定义 |
| `im-proto/src/lib.rs` | 修改 | 导出 `SendGroupMessage` 和 `PushGroupMessageSendSuccess` |
| `im-chat/src/heartbeat.rs` | 修改 | 新增常量 `SEND_GROUP_MESSAGE = 2101`、`PUSH_GROUP_MESSAGE_SEND_SUCCESS = 2201` |
| `im-common/src/config.rs` | 修改 | 新增 `persist_received_messages` 和 `match_lottery_messages` 字段及环境变量读取 |
| `im-http/src/lottery.rs` | 修改 | 扩展 `DrawItem` 字段 |
| `im-store/src/schema.rs` | 修改 | 新增 `lottery_message_templates` 表 |
| `im-store/src/lottery_template.rs` | 新增 | 模板读写 store |
| `im-store/src/lib.rs` | 修改 | 导出新模块；新增 `migrate_messages_broadcast_status` 迁移函数 |
| `im-store/src/message.rs` | 修改 | 新增 `update_broadcast_status_by_flag(uid, flag, status)` 方法 |
| `im-app/src/commands/lottery.rs` | 修改 | 新增 `get_lottery_template` / `set_lottery_template` / `get_broadcast_send_log` 命令 |
| `im-app/src/commands/chat.rs` | 修改 | `persist_monitored_batch` 增加两处开关判断（`persist_received_messages` / `match_lottery_messages`）；新增 2201 帧处理分支（按 flag 更新 broadcast_status）；新增 `start_lottery_broadcast` 函数并在连接成功后调用；广播消息先发中状态入库再发送，发送后启动 30 秒超时协程 |
| `im-app/src/main.rs` | 修改 | 注册新 Tauri 命令 |
| `im-app/ui/src/components/LotteryPanel.vue` | 修改 | 新增广播开关、模板编辑器、发送状态日志、环境配置开关 |
| `im-app/ui/src/composables/useLottery.ts` | 修改 | 新增 `getLotteryTemplate` / `setLotteryTemplate` / `getBroadcastSendLog` API 调用 |
| `im-app/ui/src/services/tauri.ts` | 修改 | 扩展 `DrawItem` 接口；新增 `LotteryTemplate`、`BroadcastSendLog` 接口及 API 函数；`BroadcastSendLog` 从 `messages` 表查询结果映射 |

---

## 错误处理

- **未登录**：广播任务不启动（连接本身需要登录）。
- **API URL 为空**：跳过本轮，记录警告日志。
- **模板为空或 `enabled=0`**：跳过本轮。
- **无监控群组**：跳过本轮。
- **单群发送失败**：更新内存发送状态为 `failed`，记录错误信息，继续处理其他群。
- **整个轮询 panic**：由 `tokio::spawn` 隔离，不会崩溃整个应用；仅记录 error 日志。
- **连接断开**：`connection_cancellation` 触发，广播任务优雅退出；重连后重新启动。
- **数据库写入失败**：消息已发送但落库失败时，记录警告（消息已送达服务端，用户可在服务端查看）。
- **2201 回调解码失败**：记录警告，丢弃当前帧。
- **2201 匹配不到记录**：记录调试日志，忽略（可能是其他客户端发送的消息）。

---

## 注意事项

1. **flag 唯一性**：`flag = uid * 1_000_000_000 + timestamp_ms`，确保同一账号不同时间发送的消息 flag 不冲突。需校验 `uid * 1_000_000_000 + timestamp_ms` 不超过 `i64::MAX`（UID 一般远小于 `i64::MAX / 1_000_000_000 ≈ 9×10^9`，安全）。
2. **模板存储表主键**：`id` 固定为 `1`，upsert 时需处理 `CHECK(id = 1)` 约束——插入时显式传 `id = 1`。
3. **广播任务不应阻塞心跳任务**：两者为独立 tokio 协程。
4. **`current_issues` 更新**：每次更新时应追加新期号而非全量替换，避免同轮多次触发时重复发送。
5. **matched=1**：广播消息入库时 `matched=1`，使其与接收到的匹配消息共用相同的查询和展示路径。
6. **2201 回调与内存状态联动**：收到 2201 后按 `flag` 精确匹配并更新内存状态，避免误更新其他消息的状态。
7. **发送状态持久化**：`broadcast_status` 写入 `messages` 表，应用重启后仍可恢复；UI 通过查询 `messages` 表获取广播消息及状态，无需额外轮询接口。
8. **`persist_received_messages=false` 不影响广播**：该开关只控制接收消息的入库，广播消息始终入库（broadcast_status 独立管理）。
