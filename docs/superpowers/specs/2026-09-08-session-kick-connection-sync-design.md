# Session Kick 连接状态同步与权限修复 Spec

> 生成时间：2026-09-08
> 依据：用户报告的三个相关问题
> 执行顺序：按依赖关系串行执行

---

## 总览

| 子项目 | 优先级 | 类型 | 涉及模块 | 依赖 |
|--------|--------|------|----------|------|
| A | P0 | 后端状态同步 | `im-app/src/commands/chat.rs` | 无 |
| B | P0 | 前端状态兜底 | `im-app/ui/src/composables/useMonitor.ts` | A |
| C | P1 | 全局错误/警告自动消失 + 关闭按钮 | `im-app/ui/src/App.vue` | 无 |
| D | P1 | 权限配置修复 | `im-app/src-tauri/capabilities/default.json` | 无 |

---

## 背景与问题分析

### 问题 1：error_code=100 后前端仍显示"已连接"

**现象**：服务端返回 `error_code=100, error_message=登录过期，请重新登录` 时，TCP 连接已断开，但前端状态栏仍显示"已连接"。

**根因**：
- 后端 [chat.rs:1367-1378](../../im-app/src/commands/chat.rs#L1367-L1378) 的 `on_server_error` 回调收到 code=100 时，只 emit `session_kicked` 事件，**没有 emit `connection_status = "disconnected"`**
- 前端 `handleSessionKicked()` 调用 `detachLocalSession()` 只清除了会话状态，但 `connectionStatus` ref 没有更新
- `connectionStatus` 依赖于 `connection_status` 事件更新，而该事件只在正常 TCP 断开时发送

**影响**：用户看到"已连接"状态，但实际无法收发消息，体验混乱。

### 问题 2：global-error / global-warning 不自动消失且 × 按钮无效

**现象**：[App.vue:171](../../im-app/ui/src/App.vue#L171) 的 `global-warning` 和 [App.vue:271](../../im-app/ui/src/App.vue#L271) 的 `global-error` 组件：
- 不会自动消失
- × 按钮没有绑定任何 `@click` 处理器，点击无效

**需求**：
- 错误消息在显示 2 秒后自动隐藏
- × 按钮点击后立即隐藏

### 问题 3：session_kicked 事件权限不足

**现象**：前端报错 `event.listen not allowed. Permissions associated with this command: core:event:allow-listen, core:event:default`

**根因**：
- `capabilities/default.json` 中只有 `core:default` 权限
- `core:default` 是一个集合权限，包含 `core:event:default`
- 但 Tauri 2.x 的权限模型中，某些操作需要显式的 `core:event:allow-listen` 权限
- `session_kicked` 事件在 `useMonitor.ts` 中独立注册（不通过 `allSettled`），触发了权限检查

---

## 解决方案

### 子项目 A：后端同步断连状态（P0）

**文件**：`im-app/src/commands/chat.rs`

**改动位置**：[chat.rs:1370-1378](../../im-app/src/commands/chat.rs#L1370-L1378)

**改动内容**：

在 `on_server_error` 回调中，当 `code == 100` 时，除了 emit `session_kicked` 事件外，还 emit `connection_status = "disconnected"`：

```rust
if code == 100 {
    tracing::warn!("Session kicked offline by other device login");
    // 同步断连状态，确保前端连接状态立即更新
    let _ = app_handle.emit("connection_status", "disconnected");
    match app_handle.emit("session_kicked", ()) {
        Ok(()) => tracing::info!("session_kicked event emitted successfully"),
        Err(e) => tracing::warn!("Failed to emit session_kicked event: {e}"),
    }
}
```

**测试**：
- 启动应用，登录账号
- 在其他设备登录同一账号
- 验证：前端连接状态立即变为"已断开"，同时弹出"被挤下线"对话框

---

### 子项目 B：前端状态兜底（P0）

**文件**：`im-app/ui/src/composables/useMonitor.ts`

**改动位置**：[useMonitor.ts:560-575](../../im-app/ui/src/composables/useMonitor.ts#L560-L575)

**改动内容**：

在 `handleSessionKicked()` 函数开头显式设置 `connectionStatus.value = 'disconnected'`：

```typescript
function handleSessionKicked() {
  // 立即同步断连状态，作为后端事件的兜底
  connectionStatus.value = 'disconnected'
  
  console.debug('[useMonitor] handleSessionKicked called, loggedIn=', loggedIn.value, 'mounted=', true)
  const confirmed = window.confirm(
    '您的账号已在其他设备登录，当前会话已被强制断开。是否重新登录？',
  )
  // ... 其余逻辑不变
}
```

**测试**：
- 手动触发 `handleSessionKicked()`（通过测试或模拟事件）
- 验证：`connectionStatus` 立即变为 `'disconnected'`

---

### 子项目 C：global-error / global-warning 自动消失 + 关闭按钮（P1）

**文件**：`im-app/ui/src/App.vue`

**改动位置**：[App.vue](../../im-app/ui/src/App.vue) 的 `<script setup>` 和 `<template>` 部分

**改动内容**：

1. 添加定时器逻辑和清除函数：

```typescript
import { onMounted, onUnmounted, watch } from 'vue'

// 全局错误/警告自动消失定时器
let globalMessageTimer: ReturnType<typeof setTimeout> | null = null

/** 清除全局消息（错误/警告）的自动消失定时器。 */
function clearGlobalMessageTimer() {
  if (globalMessageTimer) {
    clearTimeout(globalMessageTimer)
    globalMessageTimer = null
  }
}

/** 在 2 秒后自动隐藏错误消息。 */
function scheduleErrorAutoDismiss() {
  clearGlobalMessageTimer()
  globalMessageTimer = setTimeout(() => {
    monitor.error.value = ''
    globalMessageTimer = null
  }, 2000)
}

onMounted(() => {
  // ... 现有逻辑
  
  // 监听 error 变化，2 秒后自动隐藏
  watch(() => monitor.error.value, (newError) => {
    if (newError) scheduleErrorAutoDismiss()
  })
})

onUnmounted(() => {
  // ... 现有逻辑
  clearGlobalMessageTimer()
})
```

2. 为 × 按钮添加点击处理器：

```html
<!-- 警告 -->
<div v-if="monitor.warning.value" class="global-error global-warning" role="status">
  <span>警告</span>
  <p>{{ monitor.warning.value }}</p>
  <button type="button" aria-label="关闭警告" @click="monitor.warning = ''">×</button>
</div>

<!-- 错误 -->
<div v-if="monitor.error.value" class="global-error" role="alert">
  <span>错误</span>
  <p>{{ monitor.error.value }}</p>
  <button type="button" aria-label="关闭错误" @click="monitor.error = ''; clearGlobalMessageTimer()">*</button>
</div>
```

**注意**：
- `monitor.warning` 和 `monitor.error` 是 `ref`，需要 `.value` 才能修改
- 但实际上在 Vue 模板中可以直接赋值（Vue 会自动解包 ref）
- 清除定时器是为了防止自动消失和手动关闭冲突

**测试**：
- 触发一个错误（如断开连接失败）
- 验证：错误消息在 2 秒后自动消失
- 验证：点击 × 按钮立即消失

---

### 子项目 D：权限配置修复（P1）

**文件**：`im-app/src-tauri/capabilities/default.json`

**改动内容**：

显式添加 `core:event:allow-listen` 权限：

```json
{
  "uuid": "generated",
  "name": "IM Monitor",
  "description": "IM group message monitoring client",
  "remote": {
    "urls": []
  },
  "local": {
    "paths": {
      "dataDir": "~/.im-monitor"
    }
  },
  "permissions": [
    {
      "identifier": "core:default",
      "description": "Default core access"
    },
    {
      "identifier": "core:event:allow-listen",
      "description": "Allow listening to custom events (session_kicked, connection_status, etc.)"
    }
  ]
}
```

**测试**：
- 重新构建应用
- 验证：`session_kicked` 事件监听不再报错

---

## 依赖关系

```
A (后端状态同步) → B (前端兜底)
C (错误自动消失) 独立
D (权限修复)     独立
```

- A 和 B 有依赖关系：B 是 A 的兜底，但 B 可以独立测试
- C 和 D 独立，可与 A/B 并行执行

---

## 验收标准

1. **问题 1**：当收到 `error_code=100` 时，前端连接状态立即变为"已断开"，同时弹出"被挤下线"对话框
2. **问题 2**：`global-error` / `global-warning` 消息在显示 2 秒后自动消失，点击 × 按钮立即消失
3. **问题 3**：`session_kicked` 事件监听不再报错，弹窗正常显示

---

## 风险评估

| 风险 | 等级 | 缓解措施 |
|------|------|----------|
| 后端 emit `connection_status` 失败不影响主流程 | 低 | 使用 `let _ =` 忽略错误 |
| 前端 `connectionStatus.value` 被后续事件覆盖 | 低 | 后端事件会先于前端处理，顺序可控 |
| 全局错误自动消失可能误清重要错误 | 中 | 仅影响 `monitor.error`，不影响 `monitor.warning` |
| 权限配置变更需要重新构建 | 低 | 开发模式可验证，生产构建时生效 |

---

## 回滚方案

- 所有改动均为局部修改，可通过 git revert 回滚
- 权限配置变更需要重新生成 Tauri schema（`tauri build`）
