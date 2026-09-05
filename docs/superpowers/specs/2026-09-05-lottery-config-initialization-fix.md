# Lottery Config 初始化时序修复

## 问题

应用启动时，`prefetchWithDefault()` 会先以空数组 `[]` 调用 `setLotteryConfig()`，
将 `current_issues = []` 写入 DB。如果此时有实时消息入库，`persist_monitored_batch`
检测到 `current_issues` 为空，跳过匹配逻辑，消息以 `matched=0` 入库。

即使后续 `fetchHistory()` 成功拉取到期号并保存 config，消息已经以 `matched=0` 入库，
重启后前端查不到匹配消息。

**根本原因：启动时序中，`setLotteryConfig([])` 在 `fetchHistory()` 之前执行，
空配置写入了 DB。**

---

## 设计目标

1. 绝对禁止以空数组调用 `setLotteryConfig`（不保存空 config）
2. `current_issues` 始终使用从 API 拉取的实际期号
3. 若 DB 中已有非空 config，不重复保存
4. 删除 `recompute_matched_all`：历史消息与开奖模块无关，config 变更不需要 recompute 历史消息

---

## 变更范围

### 后端：`im-app/src/commands/lottery.rs`

删除 `set_lottery_config` 中的 `recompute_matched_all` 调用：

**变更前：**
```rust
db.lottery_config.upsert(&LotteryConfigRow { ... }).await?;

// 重新计算全部已有消息的 matched 标记。
let recompute_result = db.messages.recompute_matched_all(&issues_for_recompute).await;
match &recompute_result {
    Ok(count) => tracing::info!(...),
    Err(e) => tracing::warn!(...),
}
```

**变更后：**
```rust
db.lottery_config.upsert(&LotteryConfigRow { ... }).await?;
// 保存 config 即可，历史消息的 matched 由入库时确定，无需 recompute。
```

### 后端：`im-store/src/message.rs`

1. 删除 `recompute_matched_all` 方法（全表扫描，消息量大时会很慢，且逻辑上不需要）
2. 删除 `recompute_matched` 方法（同样全群扫描，且无调用方）

### 后端：`im-app/src/commands/chat.rs`

`persist_monitored_batch` 中的匹配逻辑保持不变——有 config 且 `current_issues` 非空时直接写入 `matched=1`，否则 `matched=0`。

**注意**：`insert_batch` 中 `matched` 硬写 `0`，后续通过 `UPDATE messages SET matched = 1` 修正。这是正确的，因为入库时可能还在解密过程中，`content_text` 尚未就绪。

### 前端：`im-app/ui/src/composables/useLottery.ts`

修改 `prefetchWithDefault` 逻辑：

**变更前：**
```ts
async function prefetchWithDefault(current_issues: number[]) {
  const defaultUrl = config.value.api_url
  if (!defaultUrl) { await loadConfig(); void fetchHistory(); return; }
  try {
    await api.setLotteryConfig(defaultUrl, current_issues)  // ← 空数组时也会保存
    await loadConfig()
    await fetchHistory()
    if (drawHistory.value.length > 0) {
      const issues = drawHistory.value.map(item => item.preDrawIssue)
      await api.setLotteryConfig(defaultUrl, issues)  // ← 用实际期号保存
      await loadConfig()
    }
  } catch (_e) {}
}
```

**变更后：**
```ts
async function prefetchWithDefault(_current_issues: number[]) {
  const defaultUrl = config.value.api_url
  if (!defaultUrl) { await loadConfig(); void fetchHistory(); return; }
  try {
    // 已有非空 config 时直接跳过，不重复保存，不触发无意义的 recompute
    if (config.value.current_issues.length > 0) return
    // 先拉取历史，拿到实际期号后再保存，绝不传空数组
    await fetchHistory()
    if (drawHistory.value.length > 0) {
      const issues = drawHistory.value.map(item => item.preDrawIssue)
      await api.setLotteryConfig(defaultUrl, issues)
      await loadConfig()
    }
  } catch (_e) {}
}
```

修改 `runPrefetch`：

**变更前：**
```ts
void prefetchWithDefault(currentIssues.value)
```

**变更后：**
```ts
void prefetchWithDefault([])
```

---

## 变更影响矩阵

| 场景 | 变更前 | 变更后 |
|------|--------|--------|
| 全新安装，无 config | 先用空数组保存 → fetchHistory → 再用实际期号保存 | 先 fetchHistory → 用实际期号保存 |
| DB 有 config，`current_issues` 非空 | 用传入的空数组覆盖 → 再 fetchHistory → 再用实际期号保存 | 直接跳过，不做任何保存 |
| `fetchHistory` 失败（网络等） | 用空数组保存了，DB 变成空 config | 不保存，DB 保持原状 |
| 实时消息在 config 保存前入库 | matched=0（config 为空） | matched=0（config 仍为空，但不会出现此情况，因为 fetchHistory 先于 setLotteryConfig） |
| 实时消息在 config 保存后入库 | matched=1（config 已正确） | matched=1（同上） |

---

## 不需要修改的文件

- `im-app/ui/src/composables/useMonitor.ts` — 消息加载逻辑不变
- `im-app/ui/src/App.vue` — 启动流程不变
- `im-app/ui/src/composables/useLottery.ts` 的其他方法（`saveConfig`、`fetchHistory`、`schedulePoll`）不变
- 前端 UI 组件 — 不涉及

---

## 验证方式

1. 清空 DB 中 `lottery_config` 表（模拟全新安装）
2. 启动应用，查看日志：不应出现 `issue_count = 0` 的 save 记录
3. 等待 `fetchHistory()` 完成后，确认 `lottery_config.current_issues` 为非空数组
4. 发送含"开奖"+ 期号的消息，确认 `matched=1` 入库（日志中出现 `MATCHED lottery message`）
5. 重启应用，确认消息正常显示
6. 确认 `recompute_matched_all` 相关日志不再出现
7. 确认 `set_lottery_config` 命令执行时间显著缩短（无全表扫描）
