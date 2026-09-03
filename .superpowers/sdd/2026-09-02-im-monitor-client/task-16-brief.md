## Phase 2 Task 16: Frontend wiring — login, groups, messages

**Project:** IM Monitor Client  
**Status:** `superseded`  
**Commit:** 空（当前工作区未提交）  
**Updated:** 2026-09-02

## 结论

原任务以 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src-tauri/assets/` 下的 vanilla `index.html`、`app.js`、`style.css` 为目标。该目录和重复的 `src-tauri` 配置现已删除，前端迁移到 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/`，因此原执行步骤不再适用。

等价功能已由 Vue 3 + TypeScript + Vite 实现：

- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts`：认证、群组、连接和消息 IPC；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.ts`：认证状态与命令编排；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useMonitor.ts`：群列表、历史消息、实时事件和连接状态；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/`：登录、群侧栏、消息面板和状态组件；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/chat.rs`：`get_messages`、连接状态事件和实时消息事件；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json`：Vue/Vite 开发、生产构建和 CSP。

最终审查修复后的关键语义：

- 后端 `connection_status` 是唯一权威连接状态：初始失败发布 `disconnected`，自动重连退避发布 `connecting`；前端 connect IPC 成功或失败均不自行覆盖，并在 `connecting` 时禁用重复连接；
- logout IPC 即使因后端断开超时而 reject，也会在 `finally` 清空本地 session、群、消息和连接状态，并显示 warning；
- 1201 的真实 protobuf 类型是 `LoginSessionMessage`，不是 `LoginResp`/`CommonResult`；空、畸形或缺少 `clinet_info` 的响应不会进入 Connected；
- 群同步按远端完整快照维护 `available` 标记，不物理删除有历史消息的群；刷新和登录在同一 `group_ops` 临界序列中从 `list_monitored` 重建内存集合，使消息 worker 立即停止处理不可用群，群重新出现时恢复原 `monitored`。

## 当前验证

- `cargo test --workspace --all-targets`：132 项通过，0 失败；
- `cargo check --workspace --all-targets` 与 `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets -- -D warnings`：通过；
- `npm test`：5 个测试文件、20 项通过，0 失败；
- `npm run typecheck`：通过；
- `npm run build`：`vue-tsc --noEmit && vite build` 通过；
- `npm audit`：0 vulnerabilities；
- `git diff --check`：通过。

## 真实联调缺口

- GT4 challenge 和短信发送字段尚未在真实 openchat-user 服务验证；
- SMS code、validate token、device verification 的真实关系尚未确认；
- 登录、群列表和 TCP 聊天服务尚未完成端到端联调；尤其需确认真实 1201 `LoginSessionMessage.clinet_info` 的字段填充及 token 是否回显；
- 心跳、推送、断线和重连只完成本地实现及自动测试；
- MessageExtractor 和统计功能未实现。

不新增 `task-16-report.md`：本 brief 已完整记录 superseded 原因、替代实现、验证状态和联调缺口，新增报告只会重复信息。
