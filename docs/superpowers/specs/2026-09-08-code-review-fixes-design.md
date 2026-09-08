# 代码审查修复总 Spec

> 生成时间：2026-09-08
> 依据：性能 / 安全 / 规范 / 前端四个维度审计报告
> 执行顺序：A → B → C → D → E → F（F 可与 E 并行）

---

## 总览

| 子项目 | 优先级 | 类型 | 涉及模块 | 依赖 |
|--------|--------|------|----------|------|
| A | P0 | SQLite 索引补充 | `im-store` | 无 |
| B | P0 | WAL + 连接池 + 写背压隔离 | `im-store`, `im-app` | A |
| C | P1 | 安全加固 | `im-store`, `im-http`, `im-app` | 无 |
| D | P1 | 代码规范修复 | `im-chat`, `im-http`, `im-common` | 无 |
| E | P1 | 前端性能优化 | `im-app/ui` | 无 |
| F | P2 | Error Boundary + 日志脱敏完善 | `im-app/ui`, `im-http` | C（部分重叠） |

---

## 子项目 A：SQLite 索引补充（P0）

### 背景

消息表在亿级数据量下，`mark_read` 和 `get_by_group(matched_only=true)` 两条查询因缺少覆盖索引，退化为全组扫描。

### 改动

**文件：`im-store/src/schema.rs`**

在 `SCHEMA_SQL` 常量末尾追加两条索引：

```sql
-- mark_read 查询覆盖索引：WHERE group_id=? AND matched!=0 AND read_at=0 AND msg_id<=?
CREATE INDEX IF NOT EXISTS idx_messages_group_matched_read ON messages(group_id, matched, read_at, msg_id);

-- get_by_group 查询覆盖索引：WHERE group_id=? AND (matched=1 OR ...) AND ORDER BY send_time DESC, msg_id DESC
CREATE INDEX IF NOT EXISTS idx_messages_group_time_matched ON messages(group_id, matched, send_time DESC, msg_id DESC);
```

**文件：`im-store/src/lib.rs`**

`migrate_messages_read_at` 函数之后，追加两个迁移函数：

```rust
async fn migrate_index_group_matched_read(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_messages_group_matched_read \
         ON messages(group_id, matched, read_at, msg_id)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn migrate_index_group_time_matched(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_messages_group_time_matched \
         ON messages(group_id, matched, send_time DESC, msg_id DESC)",
    )
    .execute(pool)
    .await?;
    Ok(())
}
```

在 `SqliteStore::new` 的初始化链中，于 `migrate_messages_read_at` 调用之后追加：

```rust
migrate_index_group_matched_read(&pool).await?;
migrate_index_group_time_matched(&pool).await?;
```

### 验证

- `cargo test -p im-store` 通过
- `cargo clippy -p im-store` 无新增警告
- 新建数据库时两条索引自动创建；已有数据库启动时自动迁移

---

## 子项目 B：WAL + 连接池 + 写背压隔离（P0）

### 背景

1. 连接池未指定大小，默认 `num_cpus * 2`，读密集场景不理想。
2. 无定期 WAL checkpoint，WAL 文件无限增长。
3. SQLite 写操作阻塞全部读操作时，projection worker 无法继续，反向阻塞 TCP 读取链路。

### 改动

**文件：`im-store/src/lib.rs`**

1. 显式设置连接池大小（读:写 = 4:1 比例，总数 6）：

```rust
.use_pool(/*max_connections=*/ 6)
```

2. 追加定期 WAL checkpoint 任务（在 `new()` 返回前 spawn）：

```rust
// 每 5 分钟执行一次 WAL checkpoint(TRUNCATE)，防止 WAL 无限增长
let checkpoint_pool = pool.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&checkpoint_pool)
            .await;
    }
});
```

**文件：`im-app/src/commands/chat.rs`**

将 `persist_monitored_batch` 的 SQLite 写入操作从投影 worker 的同步路径中解耦——当前已是异步调用，问题在于写阻塞反压到 projection queue。新增一个独立的 SQLite 写入 semaphore，限制并发写数为 1（SQLite 单写者约束），确保 projection queue 不因写阻塞而无限增长：

在 `ConnectionMessageEffects` 实现中，`persist_monitored_batch` 内部：

```rust
// 使用 semaphore 限流，防止慢写阻塞整条投影链路
let write_permit = self.context.db.write_semaphore.clone().await?;
// ... 原有 insert_batch 逻辑不变 ...
drop(write_permit);
```

同时在 `AppState` 或 `ConnectionContext` 中新增：

```rust
pub write_semaphore: tokio::sync::Semaphore,
```

初始值 `Semaphore::new(1)`（SQLite 单写者语义，但借由 semaphore 将等待从"阻塞整条链路"转为"快速失败+重入队列"）。

> **注意**：此改动需谨慎评估对现有消息顺序的保证。如 semaphore 导致写入顺序错乱，回退为该函数直接 await 但增加超时：`tokio::time::timeout(Duration::from_secs(3), insert_batch(...))`，超时则记录 error 并丢弃本批次。

### 验证

- `cargo test` 全量通过
- 确认 WAL 文件大小不会持续增长（本地启动 app 观察 `*.wal` 文件）

---

## 子项目 C：安全加固（P1）

### C1：私钥加密存储

**文件：`im-store/src/key_pair.rs`**

将 `private_key: String` 改为存储加密后的 bytes。加密使用 `secret_cipher` 模块中已有的 `encrypt_secret` / `decrypt_secret` API（key 从 `.credential_key` 派生）。

改动点：
- `UserKeyPairRecord.private_key` 字段类型改为 `Vec<u8>`（密文），并提供 `encrypted_private_key_bytes` 和 `private_key_for_openssl`（解密后供 OpenSSL 使用）两个访问器
- `set()` 写入前先加密私钥
- `get_latest()` 返回时不解密（保持密文），仅在 openssl 需要时按需解密

**文件：`im-store/src/schema.rs`**

`private_key` 列类型由 `TEXT` 改为 `BLOB`（或保持 `TEXT`，存 hex 编码的密文，向后兼容）。

**迁移**：在 `lib.rs` 追加 `migrate_private_key_encryption`，检测旧明文私钥并逐条重新加密。

### C2：调试日志脱敏完善

**文件：`im-http/src/openchat_user.rs`**

将 `sanitize_debug_json` 从白名单改为**黑名单**策略：任何字段名（归一化后）包含以下关键字即脱敏：

```
token | password | secret | key | mac | phone | email | uid | session
```

同时将该函数移至 `im-common/src/utils.rs`，在 `im-http` 中 `use im_common::utils::sanitize_debug_json`。

**文件：`im-http/src/im_biz.rs`**

更新所有调用处：去掉 `super::openchat_user::` 前缀，改用 `im_common::utils::sanitize_debug_json`。

### C3：已跳过

抽奖 URL 校验不在本次修复范围内。

### C4：MD5 密码预处理（客户端侧说明）

`hash_verify_passwords`、`login_password_md5`、`double_md5`（[auth.rs:1011-1040](im-app/src/commands/auth.rs#L1011)）是**协议兼容层**，服务端决定最终哈希算法，客户端无法单方面更改。此问题在 spec 中标记为"已知限制，不改动"，仅添加注释说明。

### 验证

- `cargo test` 全量通过
- 密钥对测试覆盖加密/解密往返

---

## 子项目 D：代码规范修复（P1）

### D1：生产 eprintln! 屏蔽

**文件：`im-common/src/config.rs`**

将所有 `eprintln!` 替换为 `#[cfg(debug_assertions)]` 守卫：

```rust
#[cfg(debug_assertions)]
eprintln!("from_build_env: 开始读取环境变量");
```

或直接改用 `tracing::debug!`（项目已使用 tracing）。

### D2：魔法数字 2102 提取为常量

**文件：`im-chat/src/heartbeat.rs`**

追加常量：

```rust
/// 群消息回执消息 ID（客户端发送给服务端确认收到 2202 批次）。
pub const ACK_GROUP_MESSAGE: u16 = 2102;
```

**文件：`im-app/src/commands/chat.rs:957`**

```rust
// 原来
sender.send_cancellable(2102, ...)
// 改为
sender.send_cancellable(im_chat::heartbeat::ACK_GROUP_MESSAGE, ...)
```

### D3：业务成功码常量

**文件：`im-http/src/im_biz.rs`**

在文件顶部追加：

```rust
/// 服务端业务成功码。
pub const BUSINESS_SUCCESS_CODE: u16 = 200;
```

将两处 `result.err_code != 200` 替换为 `result.err_code != BUSINESS_SUCCESS_CODE`。

### D4：write_frame 去重

**文件：`im-chat/src/client.rs`**

提取模块级辅助函数：

```rust
/// 向已锁定的 TCP 写端写入一帧并刷新。
async fn write_frame_to_stream(
    stream: &mut tokio::net::tcp::OwnedWriteHalf,
    frame: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::io::AsyncWriteExt::write_all(stream, frame).await?;
    tokio::io::AsyncWriteExt::flush(stream).await?;
    Ok(())
}
```

`ChatSender::write_frame` 和 `ChatClient::write_frame` 均调用此函数。

### D5：lottery.rs 连接池复用

**文件：`im-http/src/lottery.rs`**

将 `reqwest::Client::new()` 改为模块级 static：

```rust
static LOTTERY_CLIENT: std::sync::LazyLock<reqwest::Client> =
    std::sync::LazyLock::new(reqwest::Client::new);
```

调用处改为 `LOTTERY_CLIENT.get(url)...`。

### 验证

- `cargo fmt --all --check` 通过
- `cargo clippy -p im-chat -p im-http -p im-common` 无新增警告

---

## 子项目 E：前端性能优化（P1）

### E1：移除热路径 console.log

**文件：`im-app/ui/src/composables/useLottery.ts`**

删除所有 `console.log` 调用（共约 21 处），或替换为：

```ts
// 仅 debug 模式下输出
if (import.meta.env.DEV) console.debug('[useLottery]', ...)
```

### E2：替换 parseInt(msg_id) 为精确比较

**文件：`im-app/ui/src/components/MessagePanel.vue:238`**

```ts
// 原来
const maxMsgId = ...parseInt(m!.msg_id)
// 改为使用已有工具函数
import { compareDecimalI64 } from '@/utils/message'
// 维护一个 maxMsgId ref，在合并消息时增量更新，而非每次滚动扫描
```

**文件：`im-app/ui/src/composables/useMonitor.ts:524`**

```ts
// 原来（全量 reduce + parseInt）
messages.value.reduce((a, b) => parseInt(a.msg_id) >= parseInt(b.msg_id) ? a : b)
// 改为使用 compareDecimalI64，或维护一个 peakMsgId ref
```

### E3：消除 virtualMessages 数组拷贝

**文件：`im-app/ui/src/components/MessagePanel.vue:98-106`**

```ts
// 原来（每次 messages 变化都分配新数组）
const virtualMessages = computed<MessageDto[]>(() => {
  const result = props.messageOrder === 'newest-top'
    ? [...props.messages].reverse()
    : props.messages
  return result
})

// 改为：维护一个 reversedMessages ref，在 order 切换时 O(n) 反转一次，
// 不在每次 messages 变化时重新分配
const reversedMessages = ref<MessageDto[]>([])
watch(
  [() => props.messages, () => props.messageOrder],
  ([msgs, order]) => {
    if (order === 'newest-top') {
      reversedMessages.value = [...msgs].reverse()
    } else {
      reversedMessages.value = msgs
    }
  },
  { immediate: true }
)
// template 中使用 reversedMessages
```

### E4：scroll 处理逻辑合并

**文件：`im-app/ui/src/components/MessagePanel.vue`**

将 `handleScroll`（~line 141）和 `updateAwayFromNewEnd`（~line 207）合并为一个函数 `handleScrollAndTrack`，消除重复的 `distToBottom` 计算。

### E5：scrollDebounceTimer 清理

**文件：`im-app/ui/src/components/MessagePanel.vue:181`**

```ts
onUnmounted(() => {
  if (scrollDebounceTimer !== null) {
    clearTimeout(scrollDebounceTimer)
  }
  // ... 其他清理
})
```

### E6：filteredMessages 改为增量计数

**文件：`im-app/ui/src/composables/useMonitor.ts`**

在 `mergeAndPublishMessages` 时维护 `matchedCount` 和 `unreadMatchedCount` 两个计数器，替代每次 `computed` 的全量 filter。

### 验证

- `npm run typecheck` 通过
- `npm run test` 通过
- Lighthouse / 性能面板确认滚动时不再有 JS 主线程阻塞

---

## 子项目 F：Error Boundary + 日志脱敏完善（P2）

### F1：Vue Error Boundary

**文件：`im-app/ui/src/main.ts`**

```ts
app.config.errorHandler = (err, instance, info) => {
  console.error('[Vue Error]', err, info)
  // 不中断应用，仅记录
}
```

在 `App.vue` 根节点包裹 `<ErrorBoundary>` 组件（Vue 3 内置），捕获渲染错误。

### F2：补全日志脱敏

承接子项目 C2，`sanitize_debug_json` 切换到黑名单策略后，确认 `im-http` 中所有 debug! 调用点的敏感字段均已覆盖。

---

## 执行顺序与依赖图

```
A (索引) ──→ B (WAL/背压)
C (安全)   ──────────────────────────────→ F (Error Boundary)
D (规范)   ──────────────────────────────────────────────────→ (lint 验证)
E (前端)   ──────────────────────────────────────────────────→ (typecheck + test)
```

每个子项目完成后：
1. `cargo test` 或对应前端测试全部通过
2. 单独 commit，message 格式：`fix(scope): <描述>`
3. 再进入下一个子项目
