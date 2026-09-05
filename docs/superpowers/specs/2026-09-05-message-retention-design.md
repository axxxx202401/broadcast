# 消息保留策略设计文档

**日期：** 2026-09-05  
**状态：** 待评审  
**模块：** `im-store`、`im-app`

---

## 1. 背景与目标

应用客户端在本地 SQLite 中持久化群消息，用于监控和开奖规则匹配。随着运行时间增长，消息数据会持续累积，占用磁盘空间。

**目标：** 将本地消息保留窗口限定为 **7 天**（按消息发送时间 `send_time` 计算），超出部分自动清理，避免磁盘无限制增长。

**核心约束：** 清理操作不得阻塞或影响正常查询性能（分页读取、实时监控等）。

---

## 2. 关键决策

### 2.1 使用 `send_time` 而非 `stored_at` 作为保留基准

- **理由：** 用户感知是"只看最近 7 天的消息"，而不是"入库后 7 天删除"。`send_time` 反映消息实际发生时间，语义更直观。
- **边界：** 消息可存在于 7 天窗口外（用户离线时收到的历史消息），仍会被保留直到 `send_time` 超过阈值。

### 2.2 分批删除，避免单次大事务

- **理由：** SQLite 的 `DELETE` 在大表上会持有行级锁较长时间，同时导致 WAL 文件膨胀。分批小批量删除对查询线程干扰最小。
- **每批大小：** 200 条（与 `MAX_MESSAGE_PAGE_LIMIT` 对齐）。

### 2.3 利用现有索引，不新增索引

- `idx_messages_time (send_time DESC, msg_id DESC)` 已覆盖按时间范围的查询，无需额外索引。

---

## 3. 设计：`MessageStore::cleanup_old_messages`

### 3.1 接口

在 [`im-store/src/message.rs`](im-store/src/message.rs) 的 `MessageStore` 上新增方法：

```rust
/// 删除所有 `send_time` 早于阈值的消息。
///
/// 采用分批删除策略：每批最多删除 `BATCH_SIZE` 条，批次之间提交事务，
/// 避免长时间持锁或 WAL 文件膨胀。
///
/// `keep_since` 以 Unix 毫秒表示的截止时间；早于此时间的消息被删除。
/// 返回实际删除的消息总数。
pub async fn cleanup_old_messages(
    &self,
    keep_since: i64,
) -> sqlx::Result<usize>
```

**常量定义（在 `message.rs` 顶部）：**

```rust
/// 消息保留天数。
pub const MESSAGE_RETENTION_DAYS: u64 = 7;
/// 每批次最大删除行数。
const CLEANUP_BATCH_SIZE: usize = 200;
```

### 3.2 算法

```
total_deleted = 0
loop:
    在事务中执行：
        DELETE FROM messages
        WHERE send_time < keep_since
        LIMIT BATCH_SIZE
    获取 affected_rows
    total_deleted += affected_rows
    if affected_rows < BATCH_SIZE:
        break  // 没有更多过期消息
return total_deleted
```

**关键点：**
- 每批独立事务，SQLite 可在批间提交 WAL 并释放锁。
- `LIMIT` 确保单批删除量可控，不会对并发查询造成长时间阻塞。
- 当一批删除行数少于 `BATCH_SIZE` 时，说明已全部清理完毕。

---

## 4. 调用时机

### 4.1 登录成功后立即触发（主路径）

在 [`im-app/src/commands/auth.rs`](im-app/src/commands/auth.rs) 的 `run_complete_account_login` 中，数据库打开成功后（`state.account_db.open(...)` 返回后）异步触发清理任务，不阻塞登录响应。

```rust
// 在 open 成功之后
let db = state.account_db.open(uid, generation).await?;
// ... 登录逻辑 ...
// 在确保登录成功后，启动后台清理任务：
tokio::spawn(async move {
    let cutoff = chrono::Utc::now()
        .timestamp_millis()
        .saturating_sub(MESSAGE_RETENTION_DAYS * 24 * 3600 * 1000);
    match db.messages.cleanup_old_messages(cutoff).await {
        Ok(n) => tracing::info!(deleted = n, "message retention cleanup completed"),
        Err(e) => tracing::warn!(error = %e, "message retention cleanup failed"),
    }
});
```

**不阻塞登录：** 清理在后台 `tokio::spawn` 中执行，登录命令正常返回。清理失败只记 warning，不影响登录。

### 4.2 应用关闭时触发（兜底路径）

在 `im-app/src/main.rs` 的 `RunEvent::ExitRequested` 处理器中，在取消 `shutdown` 令牌之前，串行执行一次清理，确保退出前尽可能清理过期数据。

```rust
.run(|app_handle, event| {
    if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
        let state = app_handle.state::<AppState>();
        // 先尝试在关闭前完成一次清理（最多等待 5 秒）
        let cleanup_fut = async {
            if let Ok(db) = state.account_db.active().await {
                let cutoff = chrono::Utc::now()
                    .timestamp_millis()
                    .saturating_sub(MESSAGE_RETENTION_DAYS * 24 * 3600 * 1000);
                let _ = db.messages.cleanup_old_messages(cutoff).await;
            }
        };
        tokio::spawn(cleanup_fut);
        state.shutdown.cancel();
    }
})
```

---

## 5. 性能保障

| 关注点 | 保障措施 |
|--------|----------|
| 单次删除量 | 每批最多 200 行，事务持有时间极短 |
| 并发查询影响 | 批间提交事务，其他查询可正常并发；SQLite WAL 模式下读写不互斥 |
| WAL 膨胀 | 每批独立事务，SQLite 可在批间 checkpoint WAL |
| 登录阻塞 | 清理在 `tokio::spawn` 中异步执行，不等待结果 |
| 清理失败容忍 | 失败只记 warning，不影响业务逻辑 |
| 应用退出时清理 | 单独 spawn 不阻塞退出流程 |

---

## 6. 测试计划

### 6.1 `im-store/src/tests.rs`

新增测试用例，覆盖以下场景：

1. **空表：** 无消息时返回 0。
2. **全部过期：** 所有消息 `send_time` 均早于阈值，全部删除。
3. **全部保留：** 所有消息 `send_time` 均在阈值之后，无任何删除。
4. **部分过期：** 混合新旧消息，仅删除过期部分，保留部分不受影响。
5. **同 send_time 边界：** `send_time == keep_since` 的消息不被删除（严格小于）。
6. **分批执行：** 插入大量（> 200）过期消息，验证多批后全部清除。

### 6.2 集成测试

在登录流程相关测试中，验证清理任务启动后不会干扰登录状态。

---

## 7. 改动文件清单

| 文件 | 改动内容 |
|------|----------|
| `im-store/src/message.rs` | 新增 `MESSAGE_RETENTION_DAYS`、`CLEANUP_BATCH_SIZE` 常量和 `cleanup_old_messages` 方法 |
| `im-store/src/tests.rs` | 新增清理相关测试用例 |
| `im-app/src/commands/auth.rs` | 登录成功后 spawn 清理任务 |
| `im-app/src/main.rs` | 退出前触发清理 |

---

## 8. 不做什么

- **不做可配置保留天数：** 固定 7 天，符合需求，后续如有需要再引入配置。
- **不做按群过滤：** 全局按时间清理，不区分群组，实现更简单。
- **不做定时周期性清理：** 登录触发 + 退出兜底已足够；若用户长期不登录，旧消息会在下次登录时清理。
