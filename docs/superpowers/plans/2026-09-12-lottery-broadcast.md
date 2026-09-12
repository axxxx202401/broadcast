# 开奖广播消息功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现彩票广播消息功能——自动检测新期号、按模板组装消息、通过 TCP 发送至所有监控群组，并在 `messages` 表中以 `matched=1` + `broadcast_status` 跟踪发送状态。

**Architecture:** 在现有 Tauri 桌面应用（多 crate Rust 工作区）上扩展：protobuf 新增 `SendGroupMessage` / `PushGroupMessageSendSuccess`；`im-http` 扩展 `DrawItem` 字段；`im-store` 新增模板表和 `broadcast_status` 列迁移；`im-app` 新增广播后台任务、2201 回调处理及两个环境配置开关；前端 LotteryPanel 新增广播开关、模板编辑器和发送状态日志。

**Tech Stack:** Rust (Tokio, sqlx, prost), Tauri 2, Vue 3 + TypeScript, SQLite

**Spec:** [specs/2026-09-12-lottery-broadcast-design.md](specs/2026-09-12-lottery-broadcast-design.md)

## Global Constraints

- 保持现有代码风格：中文注释、tracing 日志、sqlx 参数绑定（`.`bind()`链式）、`#[serde(rename_all = "camelCase")]` DTO 命名。
- 不引入新依赖（prost/sqlx/tokio 已在工作区）。
- 所有 Tauri 命令返回值用 `Result<T, String>`，错误信息带上下文。
- `broadcast_status` 列默认值为 `0`，仅广播消息使用非零值；普通接收消息保持 `0`。
- protobuf 生成代码位于 `$OUT_DIR`，不得手动修改。

---

## Task 1: Protobuf 定义 + 导出

**Files:**
- Modify: `proto/broadcast.proto`
- Modify: `im-proto/src/lib.rs`

**Interfaces:**
- Produces: `im_proto::SendGroupMessage`, `im_proto::PushGroupMessageSendSuccess`（经构建脚本生成后导出）

- [ ] **Step 1: 在 broadcast.proto 末尾追加两条消息**

在 `proto/broadcast.proto` 文件末尾（最后一个 `message` 定义之后）追加：

```protobuf
// 客户端发送群聊消息 2101
message SendGroupMessage {
    GroupMessage group_msg = 1;
    int64 flag = 2;
}

// 服务端推送群聊消息发送成功 2201
message PushGroupMessageSendSuccess {
    int64 flag = 1;
    int64 msg_id = 2;
    int64 group_id = 3;
    int32 member_count = 4;
    int32 snapchat_time = 5;
    int64 sent_over_time = 6;
}
```

- [ ] **Step 2: 重新生成 protobuf 代码**

在项目根目录运行：
```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo build -p im-proto
```
确认 `target/debug/build/im-proto-*/out/_.rs` 中存在 `SendGroupMessage` 和 `PushGroupMessageSendSuccess` 类型。

- [ ] **Step 3: 在 im-proto/src/lib.rs 的 pub use 中导出新类型**

在 `pub use pb::{...}` 行末尾添加两个导出：
```rust
SendGroupMessage, PushGroupMessageSendSuccess,
```
完整行变为：
```rust
pub use pb::{
    AudioObj, AuthTradeLimit, ClientInfo, CommonResult,
    CommonResultReq, DetailReq, DetailResp, ErrrMessage, FileObj, GetKeyPairReq, GetKeyPairResp,
    GroupBase, GroupContactListReq, GroupContactListResp, GroupMemberBase, GroupMessage, ImageObj,
    KeyPairBase, KeyPairType, LoginReq, LoginResp, LoginSessionMessage, MessageType, Platform,
    PushGroupMessage, PushGroupMessageSendSuccess, PushLoginSuccessMessage, ReceiveGroupMessage,
    TextObj, TranslationInfo, UpdateKeyPairReq, UpdateKeyPairResp, UrlInfo, UserBase, VideoObj,
    SendGroupMessage,
};
```

- [ ] **Step 4: 验证编译通过**

```bash
cargo check -p im-proto
```
Expected: `checking im-proto v0.1.0` → `Finished`

- [ ] **Step 5: Commit**

```bash
git add proto/broadcast.proto im-proto/src/lib.rs
git commit -m "feat(proto): add SendGroupMessage and PushGroupMessageSendSuccess"
```

---

## Task 2: 扩展 DrawItem + 模板渲染工具函数

**Files:**
- Modify: `im-http/src/lottery.rs`

**Interfaces:**
- Consumes: `im_http::lottery::DrawItem`（现有）
- Produces: `big_small_to_str(v: i64) -> &'static str`, `single_double_to_str(v: i64) -> &'static str`, `pre_draw_code_to_display(code: &str) -> String`, `compute_pattern_desc(codes: &[i32]) -> String`

- [ ] **Step 1: 扩展 DrawItem 结构体**

在 `im-http/src/lottery.rs` 的 `DrawItem` 结构体中追加以下字段：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct DrawItem {
    #[serde(rename = "preDrawIssue")]
    pub pre_draw_issue: i64,
    #[serde(rename = "preDrawTime")]
    pub pre_draw_time: String,
    #[serde(rename = "preDrawCode", default)]
    pub pre_draw_code: String,
    #[serde(rename = "sumNum", default)]
    pub sum_num: i64,
    #[serde(rename = "sumBigSmall", default)]
    pub sum_big_small: i64,
    #[serde(rename = "sumSingleDouble", default)]
    pub sum_single_double: i64,
}
```

- [ ] **Step 2: 添加渲染工具函数**

在文件末尾（`#[cfg(test)]` 之前）追加以下函数：

```rust
/// 将 API 返回的逗号分隔号码字符串转换为 "+」连接的补零格式。
/// 例如 `"8,8,2"` → `"08+08+02"`。
pub fn pre_draw_code_to_display(code: &str) -> String {
    code.split(',')
        .map(|n| format!("{:02}", n.trim().parse::<u32>().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join("+")
}

/// 大/小/中枚举转换：1→"大"，0→"小"，其他→"中"。
pub fn big_small_to_str(v: i64) -> &'static str {
    match v { 1 => "大", 0 => "小", _ => "中" }
}

/// 单/双/中枚举转换：1→"单"，0→"双"，其他→"中"。
pub fn single_double_to_str(v: i64) -> &'static str {
    match v { 1 => "单", 0 => "双", _ => "中" }
}

/// 根据三位开奖号码判断组合特征：
/// - 三数相同 → "豹子"
/// - 三数连续且非 8,9,0/9,0,1 → "顺子"
/// - 恰好两数相同 → "对子"
/// - 以上均不满足 → ""
pub fn compute_pattern_desc(codes: &[i32]) -> String {
    if codes.len() != 3 {
        return String::new();
    }
    let mut sorted = codes.to_vec();
    sorted.sort_unstable();
    // 豹子：三数相同
    if sorted[0] == sorted[1] && sorted[1] == sorted[2] {
        return "豹子".to_string();
    }
    // 顺子：排序后相邻差值均为1，排除 8,9,0 和 9,0,1
    if sorted[1] - sorted[0] == 1 && sorted[2] - sorted[1] == 1 {
        // 8,9,0 排序后为 [0,8,9]，差值为 8,1，不满足全1条件，已排除
        // 9,0,1 排序后为 [0,1,9]，差值为 1,8，不满足全1条件，已排除
        // 只有连续三数才满足此条件
        return "顺子".to_string();
    }
    // 对子：恰好两数相同
    if sorted[0] == sorted[1] || sorted[1] == sorted[2] {
        return "对子".to_string();
    }
    String::new()
}
```

- [ ] **Step 3: 添加单元测试**

在 `#[cfg(test)] mod tests` 中追加测试：

```rust
#[test]
fn pre_draw_code_to_display_formats_correctly() {
    assert_eq!(pre_draw_code_to_display("8,8,2"), "08+08+02");
    assert_eq!(pre_draw_code_to_display("7,2,6"), "07+02+06");
    assert_eq!(pre_draw_code_to_display("10,5,3"), "10+05+03");
}

#[test]
fn big_small_to_str_returns_correct_chinese() {
    assert_eq!(big_small_to_str(1), "大");
    assert_eq!(big_small_to_str(0), "小");
    assert_eq!(big_small_to_str(-1), "中");
}

#[test]
fn single_double_to_str_returns_correct_chinese() {
    assert_eq!(single_double_to_str(1), "单");
    assert_eq!(single_double_to_str(0), "双");
    assert_eq!(single_double_to_str(-1), "中");
}

#[test]
fn compute_pattern_desc_identifies_triple() {
    assert_eq!(compute_pattern_desc(&[8, 8, 8]), "豹子");
}

#[test]
fn compute_pattern_desc_identifies_pair() {
    assert_eq!(compute_pattern_desc(&[8, 8, 2]), "对子");
    assert_eq!(compute_pattern_desc(&[2, 8, 8]), "对子");
}

#[test]
fn compute_pattern_desc_identifies_shunzi() {
    assert_eq!(compute_pattern_desc(&[3, 4, 5]), "顺子");
    assert_eq!(compute_pattern_desc(&[5, 3, 4]), "顺子");
    assert_eq!(compute_pattern_desc(&[1, 2, 3]), "顺子");
    assert_eq!(compute_pattern_desc(&[6, 7, 8]), "顺子");
    assert_eq!(compute_pattern_desc(&[7, 8, 9]), "顺子");
}

#[test]
fn compute_pattern_desc_excludes_special_sequences() {
    // 8,9,0 排序后 [0,8,9]，差值 8,1 → 不满足全1 → 无描述
    assert_eq!(compute_pattern_desc(&[8, 9, 0]), "");
    // 9,0,1 排序后 [0,1,9]，差值 1,8 → 不满足全1 → 无描述
    assert_eq!(compute_pattern_desc(&[9, 0, 1]), "");
}

#[test]
fn compute_pattern_desc_no_match_for_random() {
    assert_eq!(compute_pattern_desc(&[1, 5, 9]), "");
    assert_eq!(compute_pattern_desc(&[2, 5, 8]), "");
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p im-http
```
Expected: all new tests PASS

- [ ] **Step 5: Commit**

```bash
git add im-http/src/lottery.rs
git commit -m "feat(lottery): extend DrawItem with rendering fields and pattern detection"
```

---

## Task 3: 扩展 AppConfig 新增两个环境开关

**Files:**
- Modify: `im-common/src/config.rs`

**Interfaces:**
- Consumes: 现有 `AppConfig` 结构体和 `from_build_env` 方法
- Produces: `AppConfig.persist_received_messages: bool`, `AppConfig.match_lottery_messages: bool`

- [ ] **Step 1: 在 AppConfig 结构体中添加两个字段**

在 `im-common/src/config.rs` 的 `AppConfig` 结构体中（`lottery_default_api_url` 之后）追加：

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

- [ ] **Step 2: 更新 Default impl**

在 `impl Default for AppConfig` 的 `lottery_default_api_url: String::new(),` 之后追加：

```rust
persist_received_messages: true,
match_lottery_messages: true,
```

- [ ] **Step 3: 更新 from_build_env 读取环境变量**

在 `from_build_env` 的 `values` 向量末尾追加两个可选环境变量条目：

```rust
("IM_PERSIST_RECEIVED_MESSAGES", option_env!("IM_PERSIST_RECEIVED_MESSAGES")),
("IM_MATCH_LOTTERY_MESSAGES", option_env!("IM_MATCH_LOTTERY_MESSAGES")),
```

并在函数末尾（`Ok(Self { ... })` 构造前）读取这两个值：

```rust
let persist_received_messages = values
    .iter()
    .find_map(|(name, value)| (*name == "IM_PERSIST_RECEIVED_MESSAGES").then_some(*value))
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(true);
let match_lottery_messages = values
    .iter()
    .find_map(|(name, value)| (*name == "IM_MATCH_LOTTERY_MESSAGES").then_some(*value))
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(true);
```

然后在 `Ok(Self { ... })` 中追加这两个字段：

```rust
persist_received_messages,
match_lottery_messages,
```

- [ ] **Step 4: 更新 from_values 中的 Default 分支**

在 `Self::from_values(&values)?` 之后的 `Ok(Self { ... })` 中（用于 `from_pairs` 测试路径）追加：

```rust
persist_received_messages: true,
match_lottery_messages: true,
```

- [ ] **Step 5: 运行编译检查**

```bash
cargo check -p im-common
```
Expected: 无错误

- [ ] **Step 6: Commit**

```bash
git add im-common/src/config.rs
git commit -m "feat(config): add persist_received_messages and match_lottery_messages switches"
```

---

## Task 4: 数据库 schema — lottery_message_templates 表 + broadcast_status 迁移

**Files:**
- Modify: `im-store/src/schema.rs`
- Modify: `im-store/src/lib.rs`
- Create: `im-store/src/lottery_template.rs`

**Interfaces:**
- Consumes: `sqlx::SqlitePool`, 现有 `migrate_*` 模式
- Produces: `LotteryTemplateRow`, `LotteryTemplateStore::get(uid)`, `LotteryTemplateStore::upsert(&row)`

- [ ] **Step 1: 在 SCHEMA_SQL 中追加 lottery_message_templates 表**

在 `im-store/src/schema.rs` 的 `SCHEMA_SQL` 常量末尾（`;` 之后、`";` 之前）追加：

```sql
CREATE TABLE IF NOT EXISTS lottery_message_templates (
    id          INTEGER PRIMARY KEY CHECK(id = 1),
    template    TEXT    NOT NULL DEFAULT '加拿大 PC 第${preDrawIssue}期开奖结果：\n${preDrawCode}=${sumNum}  ${sumBigSmall}${sumSingleDouble}${patternDesc}\n近10期：${lastTenDraws}\n顶赔对赌\n大小单双：2.17\n小双大单：4.32\n大双小单：4.76\n\n大将军CU交易1群 @bkkn7mqkn0\n大将军CU交易2群 @93158hello\n大将军CU交易3群 @az8t88eeqg\n大将军上押担保频道 @flyin3037s\n大将军担保官方网站 https://djidb.com\n\n——团队担保信至上服务至上——',
    enabled     INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL
);
```

- [ ] **Step 2: 在 lib.rs 中添加 broadcast_status 迁移函数**

参考现有 `migrate_messages_matched` 的模式，在 `im-store/src/lib.rs` 中添加：

```rust
/// 检查 `messages` 表，并在缺失时补充 `broadcast_status` 列。
async fn migrate_messages_broadcast_status(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'broadcast_status'",
    )
    .fetch_one(pool)
    .await?;
    if column_count == 0 {
        sqlx::query(
            "ALTER TABLE messages ADD COLUMN broadcast_status INTEGER NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}
```

- [ ] **Step 3: 在 SqliteStore::new() 中调用迁移**

在 `migrate_index_group_time_matched(&pool).await?;` 之后（WAL checkpoint 任务之前）追加：

```rust
migrate_messages_broadcast_status(&pool).await?;
```

- [ ] **Step 4: 创建 lottery_template.rs**

```rust
use sqlx::{Row, SqlitePool};

/// 单条广播模板记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LotteryTemplateRow {
    /// 模板内容；含占位符如 `${preDrawIssue}`。
    pub template: String,
    /// 广播功能是否启用。
    pub enabled: bool,
    /// 最后更新时间（UTC Unix 毫秒）。
    pub updated_at: i64,
}

/// 广播模板数据访问入口。
pub struct LotteryTemplateStore {
    pool: SqlitePool,
}

impl LotteryTemplateStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 读取当前账号的模板；未配置时返回默认行（enabled=false）。
    pub async fn get(&self, uid: i64) -> sqlx::Result<LotteryTemplateRow> {
        let row = sqlx::query(
            "SELECT template, enabled, updated_at FROM lottery_message_templates WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(match row {
            Some(row) => LotteryTemplateRow {
                template: row.get("template"),
                enabled: row.get("enabled") != 0,
                updated_at: row.get("updated_at"),
            },
            None => LotteryTemplateRow {
                template: String::new(),
                enabled: false,
                updated_at: 0,
            },
        })
    }

    /// 插入或更新当前账号的模板。
    pub async fn upsert(&self, row: &LotteryTemplateRow, uid: i64) -> sqlx::Result<()> {
        sqlx::query(
            r#"INSERT INTO lottery_message_templates (id, template, enabled, updated_at)
               VALUES (1, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   template = excluded.template,
                   enabled = excluded.enabled,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&row.template)
        .bind(if row.enabled { 1 } else { 0 })
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

- [ ] **Step 5: 在 lib.rs 中导出新模块并添加到 SqliteStore**

在 `im-store/src/lib.rs` 中添加：

```rust
/// 广播模板数据访问类型。
pub mod lottery_template;
```

并在 `SqliteStore` 结构体中添加字段：

```rust
/// 使用同一连接池的广播模板数据访问入口。
pub lottery_template: LotteryTemplateStore,
```

在 `SqliteStore::new()` 的返回语句中添加：

```rust
lottery_template: LotteryTemplateStore::new(pool_clone.clone()),
```

- [ ] **Step 6: 运行编译检查**

```bash
cargo check -p im-store
```
Expected: 无错误

- [ ] **Step 7: Commit**

```bash
git add im-store/src/schema.rs im-store/src/lib.rs im-store/src/lottery_template.rs
git commit -m "feat(store): add lottery_message_templates table and broadcast_status migration"
```

---

## Task 5: MessageStore 新增 update_broadcast_status_by_flag

**Files:**
- Modify: `im-store/src/message.rs`

**Interfaces:**
- Consumes: 现有 `MessageStore` 和 `SqlitePool`
- Produces: `MessageStore::update_broadcast_status_by_flag(uid: i64, flag: i64, status: i32) -> sqlx::Result<()>`

- [ ] **Step 1: 添加 update_broadcast_status_by_flag 方法**

在 `MessageStore` 的 `impl` 块末尾（`cleanup_old_messages` 之后）追加：

```rust
/// 按 flag 更新广播消息的发送状态。
///
/// `flag = uid * 1_000_000_000 + timestamp_ms`，与广播任务发送时的 flag 计算方式一致。
/// 仅在 `broadcast_status = 0`（发送中）的记录上更新，避免覆盖已成功或已失败的状态。
/// `status`: 1=成功, 2=失败。
pub async fn update_broadcast_status_by_flag(
    &self,
    uid: i64,
    flag: i64,
    status: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE messages SET broadcast_status = ?
         WHERE uid = ? AND flag = ? AND broadcast_status = 0",
    )
    .bind(status)
    // 注意：messages 表目前没有 flag 列，需要同时添加。
    // 但根据设计，我们改用 msg_id 关联：
    // 实际实现改为通过 msg_id 匹配（由调用方传入 msg_id 或用 flag 推导）
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

> **注意**：`messages` 表目前没有 `flag` 列。根据 spec，`flag` 只用于匹配 2201 回调，不存库。实际实现中，我们在广播消息入库时同时写入 `msg_id`（客户端生成），然后在 2201 回调中通过 `ack.msg_id` 来更新状态。

修正后的实现：

```rust
/// 按 msg_id 更新广播消息的发送状态。
///
/// 仅在 `broadcast_status = 0`（发送中）的记录上更新，避免覆盖已成功或已失败的状态。
/// `status`: 1=成功, 2=失败。
pub async fn update_broadcast_status_by_msg_id(
    &self,
    msg_id: i64,
    status: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE messages SET broadcast_status = ?
         WHERE msg_id = ? AND broadcast_status = 0",
    )
    .bind(status)
    .bind(msg_id)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 2: 运行编译检查**

```bash
cargo check -p im-store
```
Expected: 无错误

- [ ] **Step 3: Commit**

```bash
git add im-store/src/message.rs
git commit -m "feat(store): add update_broadcast_status_by_msg_id to MessageStore"
```

---

## Task 6: Tauri 命令 — 模板读写 + 广播日志查询

**Files:**
- Modify: `im-app/src/commands/lottery.rs`

**Interfaces:**
- Consumes: `AppState`, `LotteryTemplateStore`, `MessageStore`
- Produces: `get_lottery_template`, `set_lottery_template`, `get_broadcast_send_log`

- [ ] **Step 1: 添加 DTO 类型**

在 `im-app/src/commands/lottery.rs` 中添加：

```rust
/// 暴露给前端的广播模板。
#[derive(serde::Serialize, Clone)]
pub struct LotteryTemplateDto {
    pub template: String,
    pub enabled: bool,
}

/// 暴露给前端的广播发送状态记录。
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSendLogDto {
    pub msg_id: String,
    pub group_id: String,
    pub issue: i64,
    pub broadcast_status: i32,
    pub send_time: i64,
    pub content_text: String,
}
```

- [ ] **Step 2: 添加 get_lottery_template 命令**

```rust
/// 读取当前账号的广播模板。
#[tauri::command]
pub async fn get_lottery_template(state: State<'_, AppState>) -> Result<LotteryTemplateDto, String> {
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    let row = db
        .lottery_template
        .get(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    Ok(LotteryTemplateDto {
        template: row.template,
        enabled: row.enabled,
    })
}
```

- [ ] **Step 3: 添加 set_lottery_template 命令**

```rust
/// 保存当前账号的广播模板及启用状态。
#[tauri::command]
pub async fn set_lottery_template(
    state: State<'_, AppState>,
    template: String,
    enabled: bool,
) -> Result<(), String> {
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    let updated_at = chrono::Utc::now().timestamp_millis();
    db.lottery_template
        .upsert(
            &LotteryTemplateRow {
                template,
                enabled,
                updated_at,
            },
            session.uid,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
```

需要追加 import：
```rust
use im_store::lottery_template::LotteryTemplateRow;
```

- [ ] **Step 4: 添加 get_broadcast_send_log 命令**

```rust
/// 查询当前账号的广播发送日志（broadcast_status != 0 且 matched = 1 的消息）。
#[tauri::command]
pub async fn get_broadcast_send_log(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<BroadcastSendLogDto>, String> {
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    let limit = limit.unwrap_or(50) as usize;
    let rows = sqlx::query(
        r#"SELECT m.msg_id, m.group_id, m.send_time, m.content_text, m.broadcast_status
           FROM messages m
           WHERE m.broadcast_status != 0 AND m.matched = 1
           ORDER BY m.send_time DESC, m.msg_id DESC
           LIMIT ?"#,
    )
    .bind(limit as i64)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        let msg_id: i64 = row.get("msg_id");
        // 从 msg_id 无法直接还原 issue；通过 content_text 中的期号解析，或存储时额外记录。
        // 简化方案：issue 字段留 0，由前端从 content_text 中提取。
        result.push(BroadcastSendLogDto {
            msg_id: msg_id.to_string(),
            group_id: row.get("group_id").to_string(),
            issue: 0, // 暂不填充，后续可从 content_text 解析
            broadcast_status: row.get("broadcast_status"),
            send_time: row.get("send_time"),
            content_text: row.get("content_text"),
        });
    }
    Ok(result)
}
```

需要追加 import：
```rust
use sqlx::Row;
```

- [ ] **Step 5: 运行编译检查**

```bash
cargo check -p im-app
```
Expected: 可能有 `sqlx` not in scope 错误，需添加 import

- [ ] **Step 6: 添加必要 import**

在 `im-app/src/commands/lottery.rs` 顶部追加：

```rust
use sqlx::Row;
```

- [ ] **Step 7: 再次编译检查**

```bash
cargo check -p im-app
```
Expected: 无错误

- [ ] **Step 8: Commit**

```bash
git add im-app/src/commands/lottery.rs
git commit -m "feat(commands): add lottery template and broadcast send log commands"
```

---

## Task 7: chat.rs — 两个开关控制 + 2201 处理 + 广播任务

**Files:**
- Modify: `im-app/src/commands/chat.rs`

**Interfaces:**
- Consumes: `ConnectionContext`, `ChatSender`, `CancellationToken`
- Produces: `start_lottery_broadcast(...)` 函数；消息循环中新增 2201 分支；`persist_monitored_batch` 中新增两处开关判断

- [ ] **Step 1: 在 heartbeat.rs 中添加消息 ID 常量**

在 `im-chat/src/heartbeat.rs` 末尾追加：

```rust
/// 发送群聊消息（客户端→服务端）。
pub const SEND_GROUP_MESSAGE: u16 = 2101;
/// 推送群聊消息发送成功（服务端→客户端）。
pub const PUSH_GROUP_MESSAGE_SEND_SUCCESS: u16 = 2201;
```

- [ ] **Step 2: 在消息循环中添加 2201 处理分支**

在 `im-app/src/commands/chat.rs` 的消息匹配 `match frame.message_id` 块中，在 `PUSH_RECALL_GROUP_MESSAGE` 分支之后、`message_id => tracing::debug!(...)` 之前追加：

```rust
im_chat::heartbeat::PUSH_GROUP_MESSAGE_SEND_SUCCESS => {
    let ack = match im_proto::PushGroupMessageSendSuccess::decode(frame.content.as_slice()) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to decode PushGroupMessageSendSuccess: {e}");
            continue;
        }
    };
    let msg_id = ack.msg_id;
    if let Err(e) = effects.broadcast_ack_success(msg_id).await {
        tracing::warn!("Failed to mark broadcast success: {e}");
    }
}
```

- [ ] **Step 3: 在 MessageEffects trait 中添加 broadcast_ack_success 方法**

在 `MessageEffects` trait 定义中追加：

```rust
/// 收到 2201 确认后更新广播消息状态为成功。
async fn broadcast_ack_success(&self, msg_id: i64) -> Result<(), String>;
```

在 `ConnectionMessageEffects` impl 中实现：

```rust
async fn broadcast_ack_success(&self, msg_id: i64) -> Result<(), String> {
    self.context
        .db
        .messages
        .update_broadcast_status_by_msg_id(msg_id, 1)
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 4: 在 persist_monitored_batch 中添加两处开关判断**

在 `persist_monitored_batch` 方法开头（`let mut records` 之前）添加 `persist_received_messages` 检查：

```rust
// 检查消息入库开关。
let persist_enabled = {
    let config = self.context.config.read().await;
    config.persist_received_messages
};
if !persist_enabled {
    // 跳过入库，但仍需回执（由上层处理）。
    tracing::debug!(
        message_count = records.len(),
        "persist_monitored_batch: skipped (persist_received_messages=false)"
    );
    // 返回 true 表示"已处理"，上层会继续发回执。
    return true;
}
```

在 lottery 匹配逻辑之前（`if has_config || records.iter().any(...)` 之前）添加 `match_lottery_messages` 检查：

```rust
// 检查消息匹配开关。
let match_enabled = {
    let config = self.context.config.read().await;
    config.match_lottery_messages
};
if !match_enabled {
    tracing::debug!(
        message_count = records.len(),
        "persist_monitored_batch: skipped lottery match (match_lottery_messages=false)"
    );
} else if has_config || records.iter().any(|r| r.content_text.is_empty()) {
    // 原有匹配逻辑保持不变...
}
```

- [ ] **Step 5: 添加 start_lottery_broadcast 函数**

在 `start_heartbeat` 函数之后追加：

```rust
/// 启动彩票广播后台任务。
///
/// 随聊天连接生命周期运行；连接断开或账号切换时自动停止。
pub(crate) fn start_lottery_broadcast(
    context: ConnectionContext,
    auth_session: crate::state::AuthSession,
    generation: u64,
    attempt_id: u64,
    generation_cancellation: CancellationToken,
    connection_cancellation: CancellationToken,
    sender: im_chat::ChatSender,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(20));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = generation_cancellation.cancelled() => break,
                _ = connection_cancellation.cancelled() => break,
                _ = ticker.tick() => {}
            }
            if let Err(e) = run_one_broadcast_cycle(
                &context,
                &auth_session,
                &sender,
                &connection_cancellation,
            )
            .await
            {
                tracing::warn!(error = %e, "Lottery broadcast cycle failed");
            }
        }
    });
}
```

- [ ] **Step 6: 添加 run_one_broadcast_cycle 函数**

```rust
/// 执行一轮广播检测与发送。
async fn run_one_broadcast_cycle(
    context: &ConnectionContext,
    auth_session: &tokio::sync::RwLockWriteGuard<'_, Option<crate::state::AuthSession>>,
    sender: &im_chat::ChatSender,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    // 此处简化：实际需要获取 auth_session 的读锁而非写锁
    // 正确实现见下方
    Ok(())
}
```

> 实际实现中需要从 `context.auth_session.read().await` 获取会话，以下是完整正确版本：

```rust
/// 执行一轮广播检测与发送。
async fn run_one_broadcast_cycle(
    context: &ConnectionContext,
    sender: &im_chat::ChatSender,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let session = context
        .auth_session
        .read()
        .await
        .clone()
        .ok_or("Not logged in")?;
    let uid = session.uid;

    // 1. 读取模板
    let template_row = context
        .db
        .lottery_template
        .get(uid)
        .await
        .map_err(|e| format!("Failed to load template: {e}"))?;
    if !template_row.enabled || template_row.template.is_empty() {
        return Ok(());
    }

    // 2. 读取开奖配置
    let config = context
        .db
        .lottery_config
        .get(uid)
        .await
        .map_err(|e| format!("Failed to load lottery config: {e}"))?;
    if config.api_url.is_empty() {
        return Ok(());
    }

    // 3. 获取历史列表
    let draws = im_http::lottery::fetch_draw_history(&config.api_url)
        .await
        .map_err(|e| format!("Failed to fetch lottery history: {e}"))?;
    if draws.is_empty() {
        return Ok(());
    }

    // 4. 找出新期号
    let new_draw = draws
        .iter()
        .find(|d| !config.current_issues.contains(&d.pre_draw_issue));
    let Some(draw) = new_draw else {
        return Ok(());
    };

    // 5. 准备近10期和值
    let last_ten: Vec<String> = draws
        .iter()
        .take(10)
        .map(|d| d.sum_num.to_string())
        .collect();
    let last_ten_draws = last_ten.join(" ");

    // 6. 渲染模板
    let text = template_row
        .template
        .replace("${preDrawIssue}", &draw.pre_draw_issue.to_string())
        .replace(
            "${preDrawCode}",
            &im_http::lottery::pre_draw_code_to_display(&draw.pre_draw_code),
        )
        .replace("${sumNum}", &draw.sum_num.to_string())
        .replace("${sumBigSmall}", im_http::lottery::big_small_to_str(draw.sum_big_small))
        .replace(
            "${sumSingleDouble}",
            im_http::lottery::single_double_to_str(draw.sum_single_double),
        )
        .replace(
            "${patternDesc}",
            &im_http::lottery::compute_pattern_desc(&[
                draw.pre_draw_code
                    .split(',')
                    .next()
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(0),
                draw.pre_draw_code
                    .split(',')
                    .nth(1)
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(0),
                draw.pre_draw_code
                    .split(',')
                    .nth(2)
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(0),
            ]),
        )
        .replace("${lastTenDraws}", &last_ten_draws);

    // 7. 获取监控群组列表
    let groups = context.monitoring_groups.read().await;
    if groups.is_empty() {
        return Ok(());
    }

    // 8. 逐群发送
    let now_ms = chrono::Utc::now().timestamp_millis();
    let flag = uid * 1_000_000_000 + now_ms;
    let msg_id = now_ms; // 简化：用时间戳作为 msg_id

    for group_id in groups.iter() {
        // 构造 GroupMessage
        let group_msg = im_proto::GroupMessage {
            send_uid: uid,
            group_id: *group_id,
            msg_type: im_proto::MessageType::Text as i32,
            content: text.as_bytes().to_vec(),
            send_time: now_ms,
            msg_id,
            ..Default::default()
        };

        // 构造 SendGroupMessage
        let send_msg = im_proto::SendGroupMessage {
            group_msg: Some(group_msg.clone()),
            flag,
        };

        let bytes = send_msg.encode_to_vec();

        // 先入库（broadcast_status=0）
        let record = im_store::message::MessageRecord {
            msg_id,
            group_id: *group_id,
            send_uid: uid,
            msg_type: im_proto::MessageType::Text as i32,
            content: text.as_bytes().to_vec(),
            send_time: now_ms,
            content_md5: md5::compute(&text).to_string(),
            raw_proto: Some(bytes.clone()),
            matched: 1,
            content_text: text.clone(),
            broadcast_status: 0,
        };
        if let Err(e) = context.db.messages.insert(&record).await {
            tracing::warn!(group_id, error = %e, "Failed to insert broadcast message");
            continue;
        }

        // 发送
        let send_result = sender
            .send_cancellable(
                im_chat::heartbeat::SEND_GROUP_MESSAGE,
                &bytes,
                cancellation,
                std::time::Duration::from_secs(15),
            )
            .await;

        match send_result {
            Ok(()) => {
                // 发送成功，启动 30 秒超时协程
                let db = context.db.clone();
                let msg_id = msg_id;
                let cancel = cancellation.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        cancel.cancelled(),
                    )
                    .await;
                    if result.is_ok() {
                        // 连接已取消，不更新状态
                        return;
                    }
                    // 超时：检查是否仍为发送中
                    let current = db
                        .messages
                        .get_broadcast_status(msg_id)
                        .await
                        .unwrap_or(0);
                    if current == 0 {
                        let _ = db
                            .messages
                            .update_broadcast_status_by_msg_id(msg_id, 2)
                            .await;
                        tracing::warn!(msg_id, group_id, "Broadcast message timed out");
                    }
                });
            }
            Err(e) => {
                // 发送失败
                let _ = context
                    .db
                    .messages
                    .update_broadcast_status_by_msg_id(msg_id, 2)
                    .await;
                tracing::warn!(group_id, error = %e, "Failed to send broadcast message");
            }
        }
    }

    // 9. 更新 current_issues
    let mut updated_issues = config.current_issues.clone();
    updated_issues.push(draw.pre_draw_issue);
    let updated_at = chrono::Utc::now().timestamp_millis();
    if let Err(e) = context
        .db
        .lottery_config
        .upsert(
            &im_store::lottery_config::LotteryConfigRow {
                uid,
                api_url: config.api_url,
                current_issues: updated_issues,
                updated_at,
            },
        )
        .await
    {
        tracing::warn!(error = %e, "Failed to update lottery config");
    }

    Ok(())
}
```

- [ ] **Step 7: 在 MessageRecord 中添加 broadcast_status 字段**

在 `im-store/src/message.rs` 的 `MessageRecord` 结构体中追加：

```rust
/// 广播发送状态：0=发送中, 1=成功, 2=失败；仅广播消息非零。
pub broadcast_status: i32,
```

并更新所有 `insert_batch` SQL 语句中的列列表和值列表：

在 `insert_batch` 方法的 SQL 中：
- INSERT 列列表追加 `, broadcast_status`
- VALUES 追加 `, ?`
- ON CONFLICT DO UPDATE SET 追加 `, broadcast_status = excluded.broadcast_status`

- [ ] **Step 8: 添加 get_broadcast_status 辅助方法**

在 `MessageStore` 中追加：

```rust
/// 读取消息的 broadcast_status。
pub async fn get_broadcast_status(&self, msg_id: i64) -> sqlx::Result<i32> {
    sqlx::query_scalar("SELECT broadcast_status FROM messages WHERE msg_id = ?")
        .bind(msg_id)
        .fetch_one(&self.pool)
        .await
}
```

- [ ] **Step 9: 在连接成功后调用 start_lottery_broadcast**

在 `connect_chat_inner` 函数中，`start_heartbeat(...)` 调用之后追加：

```rust
start_lottery_broadcast(
    context.clone(),
    auth_session.clone(),
    generation,
    attempt_id,
    generation_cancellation.clone(),
    connection_cancellation.clone(),
    sender.clone(),
);
```

- [ ] **Step 10: 运行编译检查**

```bash
cargo check -p im-app
```
Expected: 可能有多个编译错误，逐一修复

- [ ] **Step 11: 修复编译错误**

根据错误提示，可能需要：
1. 在 `im-app/src/commands/chat.rs` 顶部添加必要的 import：
```rust
use im_store::message::MessageRecord;
use im_store::lottery_config::LotteryConfigRow;
use md5;
```
2. 确保 `ConnectionContext` 可访问 `db` 字段（目前已是 public）
3. 修复 `broadcast_status` 字段在 `MessageRecord` 中的位置

- [ ] **Step 12: 再次编译检查**

```bash
cargo check -p im-app
```
Expected: 无错误

- [ ] **Step 13: Commit**

```bash
git add im-chat/src/heartbeat.rs im-store/src/message.rs im-app/src/commands/chat.rs
git commit -m "feat(chat): add broadcast task, 2201 handler, and persist/match switches"
```

---

## Task 8: 注册新 Tauri 命令

**Files:**
- Modify: `im-app/src/main.rs`

**Interfaces:**
- Consumes: 现有命令注册列表
- Produces: 新增 `get_lottery_template`, `set_lottery_template`, `get_broadcast_send_log` 命令

- [ ] **Step 1: 在 invoke_handler 中注册新命令**

在 `im-app/src/main.rs` 的 `.with_commands(...)` 调用中，在 `fetch_lottery_history` 之后追加：

```rust
commands::lottery::get_lottery_template,
commands::lottery::set_lottery_template,
commands::lottery::get_broadcast_send_log,
```

- [ ] **Step 2: 运行编译检查**

```bash
cargo check -p im-app
```
Expected: 无错误

- [ ] **Step 3: Commit**

```bash
git add im-app/src/main.rs
git commit -m "feat(app): register lottery template and broadcast log commands"
```

---

## Task 9: 前端 — tauri.ts 接口扩展

**Files:**
- Modify: `im-app/ui/src/services/tauri.ts`
- Modify: `im-app/ui/src/composables/useLottery.ts`

**Interfaces:**
- Produces: `DrawItem` 接口新增字段；`LotteryTemplate`、`BroadcastSendLog` 接口；对应 API 函数

- [ ] **Step 1: 扩展 DrawItem 接口**

在 `im-app/ui/src/services/tauri.ts` 中修改 `DrawItem`：

```typescript
/** 单条开奖历史条目。 */
export interface DrawItem {
  /** 期号。 */
  preDrawIssue: number
  /** 开奖时间字符串，格式为 `"YYYY-MM-DD HH:MM:SS"`。 */
  preDrawTime: string
  /** 开奖号码，逗号分隔原始值（如 `"8,8,2"`），前端渲染时补零转 "+" 连接。 */
  preDrawCode: string
  /** 和值。 */
  sumNum: number
  /** 大/小/中：1=大，0=小，-1=中。 */
  sumBigSmall: number
  /** 单/双/中：1=单，0=双，-1=中。 */
  sumSingleDouble: number
}
```

- [ ] **Step 2: 新增 LotteryTemplate 和 BroadcastSendLog 接口**

```typescript
/** 广播模板。 */
export interface LotteryTemplate {
  template: string
  enabled: boolean
}

/** 广播发送状态记录。 */
export interface BroadcastSendLog {
  /** 消息 ID（十进制字符串）。 */
  msgId: string
  /** 群 ID（十进制字符串）。 */
  groupId: string
  /** 期号。 */
  issue: number
  /** 广播状态：0=发送中, 1=成功, 2=失败。 */
  broadcastStatus: number
  /** 发送时间（Unix ms）。 */
  sendTime: number
  /** 消息文本。 */
  contentText: string
}
```

- [ ] **Step 3: 在 api 对象中追加新函数**

```typescript
  /** 获取广播模板。 */
  getLotteryTemplate: () => invoke<LotteryTemplate>('get_lottery_template'),
  /** 保存广播模板。 */
  setLotteryTemplate: (template: string, enabled: boolean) =>
    invoke<void>('set_lottery_template', { template, enabled }),
  /** 获取广播发送日志。 */
  getBroadcastSendLog: (limit?: number) =>
    invoke<BroadcastSendLog[]>('get_broadcast_send_log', { limit }),
```

- [ ] **Step 4: 更新 useLottery.ts 中的 fetchHistory**

在 `im-app/ui/src/composables/useLottery.ts` 中，将 `DrawItem` 映射改为包含新字段：

```typescript
const items = await api.fetchLotteryHistory()
drawHistory.value = items.slice(0, 20)
```

由于后端 `DrawItemDto` 也需要扩展，先确认后端是否已返回新字段（Task 7 已完成）。

- [ ] **Step 5: 运行 TypeScript 类型检查**

```bash
cd im-app/ui && npx vue-tsc --noEmit
```
Expected: 无类型错误

- [ ] **Step 6: Commit**

```bash
git add im-app/ui/src/services/tauri.ts im-app/ui/src/composables/useLottery.ts
git commit -m "feat(ui): extend DrawItem and add lottery template/broadcast APIs"
```

---

## Task 10: 前端 — LotteryPanel.vue 新增广播 UI

**Files:**
- Modify: `im-app/ui/src/components/LotteryPanel.vue`

**Interfaces:**
- Consumes: `useLottery()` composable（已有）
- Produces: 广播开关、模板编辑器、发送状态日志、环境配置开关

- [ ] **Step 1: 在 script 中新增状态和方法**

```typescript
import { computed, ref, watch } from 'vue'
import { api } from '../services/tauri'
import type { BroadcastSendLog, LotteryTemplate } from '../services/tauri'
import { useLottery } from '../composables/useLottery'
import { errorMessage } from '../utils/protocol'

const props = withDefaults(defineProps<{
  lottery?: { /* 现有 prop */ }
}>(), {})

const src = props.lottery ?? useLottery()
// ... 现有代码 ...

// 广播模板状态
const template = ref<LotteryTemplate>({ template: '', enabled: false })
const templateEditing = ref(false)
const templateEditValue = ref('')

// 广播发送日志
const sendLogs = ref<BroadcastSendLog[]>([])
const sendLogsLoading = ref(false)

// 环境配置开关
const persistMessages = ref(true)
const matchMessages = ref(true)

/** 加载广播模板。 */
async function loadTemplate() {
  try {
    template.value = await api.getLotteryTemplate()
  } catch (e) {
    console.error('Failed to load template:', errorMessage(e))
  }
}

/** 保存广播模板。 */
async function saveTemplate() {
  try {
    await api.setLotteryTemplate(templateEditValue.value, template.value.enabled)
    await loadTemplate()
    templateEditing.value = false
  } catch (e) {
    console.error('Failed to save template:', errorMessage(e))
  }
}

/** 取消编辑。 */
function cancelTemplateEdit() {
  templateEditValue.value = template.value.template
  templateEditing.value = false
}

/** 加载广播发送日志。 */
async function loadSendLogs() {
  sendLogsLoading.value = true
  try {
    sendLogs.value = await api.getBroadcastSendLog(20)
  } catch (e) {
    console.error('Failed to load send logs:', errorMessage(e))
  } finally {
    sendLogsLoading.value = false
  }
}

/** 切换广播启用状态。 */
async function toggleBroadcastEnabled(enabled: boolean) {
  template.value.enabled = enabled
  try {
    await api.setLotteryTemplate(template.value.template, enabled)
  } catch (e) {
    console.error('Failed to save template:', errorMessage(e))
    template.value.enabled = !enabled
  }
}

/** 切换消息入库开关。 */
async function togglePersistMessages(value: boolean) {
  persistMessages.value = value
  // 注意：此开关由编译期环境变量控制，运行时无法动态更改
  // UI 仅作为信息展示，不实际调用后端 API
}

/** 切换消息匹配开关。 */
async function toggleMatchMessages(value: boolean) {
  matchMessages.value = value
  // 同上，仅展示
}

// 挂载时加载
import { onMounted } from 'vue'
onMounted(() => {
  void loadTemplate()
  void loadSendLogs()
})
```

- [ ] **Step 2: 在模板中追加广播区域**

在现有 `lottery-strip` / `lottery-edit` 之后追加广播控制面板：

```html
<!-- 广播控制面板 -->
<div class="broadcast-section">
  <div class="broadcast-header">
    <span class="section-title">广播设置</span>
    <label class="toggle-label">
      <input type="checkbox" :checked="template.enabled" @change="toggleBroadcastEnabled($event.target.checked)" />
      <span class="toggle-slider"></span>
      自动广播
    </label>
  </div>

  <!-- 模板编辑器 -->
  <div v-if="templateEditing" class="template-edit">
    <textarea v-model="templateEditValue" class="template-textarea" rows="12"></textarea>
    <div class="edit-actions">
      <button class="btn-ghost" @click="cancelTemplateEdit">取消</button>
      <button class="btn-primary" @click="saveTemplate">保存</button>
    </div>
  </div>
  <button v-else class="btn-ghost btn-sm" @click="() => { templateEditValue = template.template; templateEditing = true }">
    编辑模板
  </button>

  <!-- 发送状态日志 -->
  <div class="send-logs" v-if="sendLogs.length > 0">
    <div class="log-header">
      <span>发送日志</span>
      <button class="btn-icon" @click="loadSendLogs" :disabled="sendLogsLoading">↻</button>
    </div>
    <div v-for="log in sendLogs" :key="log.msgId" class="log-row" :class="`status-${log.broadcastStatus}`">
      <span class="log-group">{{ log.groupId }}</span>
      <span class="log-issue">#{{ log.issue || '—' }}</span>
      <span class="log-status">
        <span v-if="log.broadcastStatus === 0" class="badge sending">发送中</span>
        <span v-else-if="log.broadcastStatus === 1" class="badge success">成功</span>
        <span v-else class="badge failed">失败</span>
      </span>
      <span class="log-time">{{ formatTime(log.sendTime) }}</span>
    </div>
  </div>

  <!-- 环境配置 -->
  <div class="env-config">
    <div class="config-row">
      <span class="config-label">消息入库</span>
      <span class="config-value">{{ persistMessages ? '是' : '否' }}</span>
      <span class="config-note">（编译期配置，需重新构建生效）</span>
    </div>
    <div class="config-row">
      <span class="config-label">消息匹配</span>
      <span class="config-value">{{ matchMessages ? '是' : '否' }}</span>
      <span class="config-note">（编译期配置，需重新构建生效）</span>
    </div>
  </div>
</div>
```

- [ ] **Step 3: 添加样式**

```scss
.broadcast-section {
  border-top: 1px solid var(--border-subtle);
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.broadcast-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.section-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.toggle-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-secondary);
  cursor: pointer;
}

.template-edit {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.template-textarea {
  width: 100%;
  font-family: "IBM Plex Mono", monospace;
  font-size: 11px;
  padding: 6px 8px;
  border: 1px solid var(--border-medium);
  border-radius: var(--radius);
  background: var(--bg-surface);
  color: var(--text-primary);
  resize: vertical;
  outline: none;
}

.template-textarea:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(240, 180, 70, 0.12);
}

.send-logs {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.log-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 10px;
  color: var(--text-tertiary);
}

.log-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  padding: 3px 6px;
  background: var(--bg-elevated);
  border-radius: 3px;
}

.log-group {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
}

.log-issue {
  font-family: "IBM Plex Mono", monospace;
  color: var(--text-tertiary);
}

.log-status {
  flex: 0 0 auto;
}

.badge {
  display: inline-block;
  padding: 1px 6px;
  border-radius: 2px;
  font-size: 10px;
  font-weight: 500;
}

.badge.sending {
  background: rgba(240, 180, 70, 0.15);
  color: var(--accent);
}

.badge.success {
  background: rgba(82, 196, 26, 0.15);
  color: var(--success);
}

.badge.failed {
  background: rgba(239, 68, 68, 0.15);
  color: var(--danger);
}

.log-time {
  color: var(--text-tertiary);
  font-size: 10px;
}

.env-config {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px 8px;
  background: var(--bg-elevated);
  border-radius: 4px;
}

.config-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
}

.config-label {
  color: var(--text-secondary);
  width: 60px;
}

.config-value {
  color: var(--text-primary);
  font-weight: 500;
}

.config-note {
  color: var(--text-tertiary);
  font-size: 10px;
}

.btn-sm {
  font-size: 11px;
  padding: 2px 8px;
}

.btn-icon {
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--text-tertiary);
  font-size: 12px;
  padding: 2px 4px;
}

.btn-icon:hover {
  color: var(--text-primary);
}
```

- [ ] **Step 4: 添加 formatTime 辅助函数**

在 script 中追加：

```typescript
function formatTime(timestamp: number): string {
  if (!timestamp) return ''
  const date = new Date(timestamp)
  const hours = date.getHours().toString().padStart(2, '0')
  const minutes = date.getMinutes().toString().padStart(2, '0')
  return `${hours}:${minutes}`
}
```

- [ ] **Step 5: 运行开发服务器检查**

```bash
cd im-app/ui && npm run dev
```
Expected: 无编译错误，界面正常渲染

- [ ] **Step 6: Commit**

```bash
git add im-app/ui/src/components/LotteryPanel.vue
git commit -m "feat(ui): add broadcast panel with toggle, template editor, and send logs"
```

---

## Task 11: 端到端集成测试

**Files:**
- New: `im-http/src/lottery.rs` 中的测试（Task 2 已完成）
- New: `im-store/src/lottery_template.rs` 单元测试
- New: `im-app/src/commands/lottery.rs` 集成测试

**Interfaces:**
- 验证：API 解析、模板 CRUD、广播状态更新

- [ ] **Step 1: 为 lottery_template.rs 添加单元测试**

在 `im-store/src/lottery_template.rs` 中添加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePool;

    async fn test_pool() -> SqlitePool {
        SqlitePool::connect(":memory:").await.unwrap()
    }

    #[tokio::test]
    async fn insert_and_fetch_template() {
        let pool = test_pool().await;
        sqlx::query(
            "CREATE TABLE lottery_message_templates (
                id INTEGER PRIMARY KEY CHECK(id = 1),
                template TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = LotteryTemplateStore::new(pool);
        let row = LotteryTemplateRow {
            template: "测试模板".to_string(),
            enabled: true,
            updated_at: 1234567890,
        };
        store.upsert(&row, 1).await.unwrap();

        let fetched = store.get(1).await.unwrap();
        assert_eq!(fetched.template, "测试模板");
        assert!(fetched.enabled);
    }
}
```

- [ ] **Step 2: 运行所有测试**

```bash
cargo test
```
Expected: 所有测试 PASS

- [ ] **Step 3: 运行完整编译**

```bash
cargo build
```
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add .
git commit -m "test: add unit tests for lottery template and broadcasting"
```

---

## 验证清单

实现完成后，逐项验证：

- [ ] `cargo test` 全部通过
- [ ] `cargo build` 无警告
- [ ] 启动应用后，在 LotteryPanel 中能看到广播开关（默认关闭）
- [ ] 开启广播开关后，等待 20 秒内检测到新期号
- [ ] 检测到新期号后，消息以 `matched=1, broadcast_status=0` 写入数据库
- [ ] 消息发送后，在 UI 消息面板中能看到该消息（状态为"发送中"）
- [ ] 收到 2201 后，`broadcast_status` 更新为 1（成功）
- [ ] 若 30 秒内未收到 2201，`broadcast_status` 更新为 2（失败）
- [ ] 关闭「消息入库」开关后，收到的消息不写库（需重新构建生效）
- [ ] 关闭「消息匹配」开关后，收到的消息不设置 `matched=1`（需重新构建生效）
- [ ] 编辑模板后保存，下次广播使用新模板

---

## 注意事项

1. **flag 生成**：`flag = uid * 1_000_000_000 + timestamp_ms`，确保在同一毫秒内不重复（可用纳秒级时间戳或追加随机数）。
2. **msg_id 生成**：广播消息的 `msg_id` 与 `flag` 独立生成，2201 回调通过 `ack.msg_id` 匹配，不依赖 `flag`。
3. **并发安全**：广播任务与消息处理任务共享 `connection_cancellation`，确保断开时广播任务同步退出。
4. **超时协程泄漏**：30 秒超时协程在 `connection_cancellation` 触发时应被取消，避免无效数据库查询。
5. **编译期开关**：`persist_received_messages` 和 `match_lottery_messages` 是编译期配置，UI 开关仅做信息展示，实际修改需重新构建。
