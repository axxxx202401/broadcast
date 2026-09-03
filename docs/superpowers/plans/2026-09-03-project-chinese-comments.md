# 全项目详细中文注释实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为项目自有生产代码与测试代码建立准确、可维护的语义级中文注释基线，并持续约束后续代码同步维护中文注释。

**Architecture:** 按依赖顺序和模块边界分批治理，每批只修改注释或注释约束，不改变运行行为。先落地项目规则和评审清单，再依次处理公共协议、聊天、HTTP、存储、Tauri 后端和 Vue 前端；每个 Rust crate 完成基线后单独启用 `missing_docs = "warn"`。

**Tech Stack:** Rust 2021、Cargo、Rustdoc、Clippy、Protobuf/prost、Tauri v2、Vue 3、TypeScript、Vite、Vitest、CSS。

---

## 0. 执行约束

- 工作目录：`/Volumes/TRANSCEND/works/objects/rust/broadcast`。
- 设计依据：`/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/specs/2026-09-03-project-chinese-comments-design.md`。
- 不修改 `target/`、`node_modules/`、`dist/`、锁文件、prost 生成代码或 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/vendor/gt4.js`。
- 注释使用简体中文；`TCP`、`AES`、`gzip`、`Protobuf`、`X-One`、`X-Ten`、`GT4`、字段名和路径等标识保留英文。
- 不解释自明赋值、普通分支、常规 Vue 标签或单条 CSS 声明。
- 不确定的协议含义不推测；只记录代码、测试和设计文档能够证明的事实。
- 每个任务先运行基线测试，再修改注释，最后运行同一组检查。
- 注释任务不新增行为测试；现有测试是“代码行为未改变”的回归证据。
- 每个任务只暂存任务列出的文件，避免提交当前工作区中无关的未跟踪文件。

### Task 1：建立持久中文注释规则

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/.cursor/rules/chinese-comments.mdc`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/review/chinese-comments-checklist.md`

- [ ] **Step 1：创建 Cursor 项目规则**

写入以下完整内容：

```markdown
---
description: 项目中文注释与文档维护规范
alwaysApply: true
---

# 中文注释规范

依据 `docs/superpowers/specs/2026-09-03-project-chinese-comments-design.md`。

## 范围
- 纳入：Rust、TypeScript、Vue、CSS 自有源码与测试、`proto/broadcast.proto`、构建脚本。
- 排除：`target/`、`node_modules/`、`dist/`、prost 生成代码、`im-app/ui/public/vendor/gt4.js`、锁文件与纯资源。

## 原则
- 使用语义级详细中文注释，说明职责、约束、错误、副作用和协议含义，不复述自明语法。
- 协议名、字段名、HTTP 头和算法标识保留英文，说明性文字使用简体中文。
- 修改实现后同步检查相邻注释；无法确认的业务含义不得编造。

## 格式
- Rust 公开 API 使用 `///`，crate/模块使用 `//!`，局部决策使用 `//`。
- TypeScript/Vue 导出接口、composable 和复杂服务使用 JSDoc。
- Vue/CSS 按区域或非自明交互分段说明，不逐条解释样式或标签。

## 变更要求
- 新增或修改模块、公开 API、复杂私有逻辑和测试场景时，必须同步维护中文注释。
- 评审对照 `docs/review/chinese-comments-checklist.md`；缺少必要注释或注释与实现不一致时不得合并。

## Rust lint
- 某 crate 完成注释基线后，才在其 `Cargo.toml` 启用 `missing_docs = "warn"`。
- prost 生成模块允许局部豁免，初期禁止 workspace 级 `deny(missing_docs)`。
```

- [ ] **Step 2：创建评审清单**

清单必须覆盖：变更范围、模块职责、公开 API、复杂逻辑、测试场景、协议一致性、英文注释翻译、非机械注释、相关测试和静态检查。

```markdown
# 中文注释评审清单

> 完整规范：`docs/superpowers/specs/2026-09-03-project-chinese-comments-design.md`

## 通用
- [ ] 未修改生成物、第三方代码、构建产物或锁文件
- [ ] 新增或修改的模块有中文职责说明
- [ ] 公开 API 的参数、返回值、错误、副作用和调用约束说明完整
- [ ] 加密、帧格式、重连、状态机、持久化等复杂逻辑说明关键不变量
- [ ] 测试说明场景意图，复杂准备过程和关键断言有必要注释
- [ ] 既有英文说明已翻译，协议原始标识得到保留
- [ ] 注释与实现、协议和测试一致，不含机械复述或推测
- [ ] 注释治理未夹带业务行为修改

## 验证
- [ ] `cargo fmt --all --check`
- [ ] 相关 crate 的测试与 Clippy 通过
- [ ] 已启用 `missing_docs` 的 crate 无新增文档警告
- [ ] 前端 `npm run typecheck`、`npm test` 和 `npm run build` 通过
```

- [ ] **Step 3：检查规则文件格式**

Run:

```bash
git diff --check -- /Volumes/TRANSCEND/works/objects/rust/broadcast/.cursor/rules/chinese-comments.mdc /Volumes/TRANSCEND/works/objects/rust/broadcast/docs/review/chinese-comments-checklist.md
```

Expected: exit 0，无空白错误。

- [ ] **Step 4：提交规则与清单**

```bash
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/.cursor/rules/chinese-comments.mdc /Volumes/TRANSCEND/works/objects/rust/broadcast/docs/review/chinese-comments-checklist.md
git commit -m "docs: enforce Chinese code comments"
```

### Task 2：注释 Protobuf 与公共基础类型

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/proto/broadcast.proto`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/build.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/src/lib.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/lib.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/error.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/config.rs`

- [ ] **Step 1：记录基线**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-common
cargo test -p im-proto
cargo doc -p im-common --no-deps
```

Expected: `im-common` 24 tests passed；`im-proto` exit 0；Rustdoc 仅允许记录到的 `tcp_head.rs` 链接告警。

- [ ] **Step 2：补充 Proto 语义说明**

为 `MessageType`、`Platform`、群组、登录和验证相关 enum/message 添加中文说明；为非自明字段补业务含义、单位和兼容约束。`clinet_info` 等服务端既有拼写必须保留并说明兼容原因；无法从调用方证明的 `bf_*`、`review_model`、`display` 等字段只描述数据类型和传输角色，不推测业务含义。

注释风格：

```proto
// 登录会话请求。字段名 clinet_info 沿用服务端协议拼写，不得改名。
message LoginSessionMessage {
  // 客户端及当前授权信息。
  ClientInfo clinet_info = 1;
}
```

- [ ] **Step 3：注释 im-proto 包装层**

将英文 crate 文档翻译为中文；说明 `build.rs` 负责在 Proto 变化时重新生成 Rust 类型；说明 `pb` 为生成代码边界，根级 re-export 仅提供稳定导入路径。

```rust
//! OpenChat Protobuf 类型入口。
//!
//! 类型定义来源于 `proto/broadcast.proto`，构建时由 `prost-build` 生成。

#[allow(missing_docs)] // 生成代码的字段文档由 Proto 源文件维护。
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}
```

- [ ] **Step 4：注释 im-common crate、错误与配置**

补充 crate 职责和帧大小上限；为 `AppError`、`AppResult`、`AppConfig`、`ServerConfig`、`DeviceConfig`、`Default` 和 `DeviceConfig::new` 补中文 Rustdoc。不得翻译 `#[error(...)]` 运行时字符串，因为那会改变用户可见行为。

- [ ] **Step 5：验证并提交**

```bash
cargo fmt --all --check
cargo test -p im-common
cargo test -p im-proto
cargo clippy -p im-common --all-targets -- -D warnings
cargo clippy -p im-proto --all-targets -- -D warnings
cargo doc -p im-proto --no-deps
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/proto/broadcast.proto /Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/build.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/src/lib.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/lib.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/error.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/config.rs
git commit -m "docs: explain protobuf and common types"
```

Expected: 所有命令 exit 0，测试数不变。

### Task 3：注释公共加密、帧头和版本头

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/aes.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/tcp_head.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/version_key.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/src/tests.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/Cargo.toml`

- [ ] **Step 1：补充 AES 和 TCP 帧头说明**

说明 AES-128-ECB-PKCS7、16 字节密钥要求、`new` 的 panic 行为、`try_new` 的错误返回及加解密缓冲区策略。将 TCP 位域说明统一为中文，并用反引号包裹 `byte[0]`、`byte[1]`，消除 Rustdoc 链接告警。

- [ ] **Step 2：翻译并扩充 X-One/X-Ten 文档**

为 `HeaderManager` 及其公开方法说明匿名/认证头差异、时间戳单位、AES 后 hex 编码和空 token/session 的意图；保留 `X-One`、`X-Ten`、`V_L_SALT` 原名。

- [ ] **Step 3：注释测试场景**

文件头划分 AES、TCP 帧头、配置和版本头四类测试；只为非法密钥、位域往返和固定盐等非自明场景添加中文说明，不逐条复述断言。

- [ ] **Step 4：启用 im-common 文档警告**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common/Cargo.toml` 增加：

```toml
[lints.rust]
missing_docs = "warn"
```

- [ ] **Step 5：验证并提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all --check
cargo test -p im-common
cargo clippy -p im-common --all-targets -- -D warnings
cargo doc -p im-common --no-deps
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-common
git commit -m "docs: explain common crypto and framing"
```

Expected: 24 tests passed；Clippy 和 Rustdoc 无文档告警。

### Task 4：注释聊天帧、心跳与重连

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/lib.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/frame.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/heartbeat.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/reconnect.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/tests.rs`

- [ ] **Step 1：记录基线**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-chat
cargo clippy -p im-chat --all-targets -- -D warnings
```

Expected: 28 tests passed，Clippy exit 0。

- [ ] **Step 2：文档化帧协议**

crate 文档说明其边界；`frame.rs` 统一说明：

```text
[head(2)][message_id(2, BE)][content_length(4, BE)][content]
```

说明发送顺序为明文 → AES → 可选 gzip，接收顺序相反；说明 8 MiB wire 限制、32 MiB 解压限制、`Incomplete` 与 `Invalid` 差异、所有服务端密文统一使用 Session 正文密钥，以及 `content.len() <= 1` 的 Java 兼容行为。

- [ ] **Step 3：文档化心跳与重连**

为消息 ID 常量、`heartbeat_message`、`heartbeat_loop`、`ExponentialBackoff` 和 `reconnect_loop` 补中文 Rustdoc。明确心跳首次发送延迟一个完整周期；重连每次尝试前等待、指数翻倍并封顶 30 秒，取消返回 `None`。

- [ ] **Step 4：注释对应测试**

在测试文件头划分帧协议、传输兼容、心跳和重连场景；重点说明 AES→gzip 顺序、空 ACK、半包/坏包、解压膨胀限制和可取消等待。

- [ ] **Step 5：验证并提交**

```bash
cargo fmt --all --check
cargo test -p im-chat
cargo clippy -p im-chat --all-targets -- -D warnings
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/lib.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/frame.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/heartbeat.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/reconnect.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/tests.rs
git commit -m "docs: explain chat framing and retries"
```

### Task 5：注释聊天客户端生命周期并启用 lint

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/client.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/src/tests.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/Cargo.toml`

- [ ] **Step 1：注释公开回调、发送器和客户端**

为 `MessageHandler`、`DisconnectHandler`、`ChatSender`、`ChatClient` 及公开方法说明连接前置条件、超时/取消、副作用与错误。将 `FIX:` 英文历史说明改为中文设计理由。

- [ ] **Step 2：说明连接状态流**

在 `connect`、`disconnect`、`force_abort`、`build_login_frame`、`build_client_frame`、`ReadTask::run` 和 `handle_data` 前说明：

```text
connect → 拆分 TCP 读写半部 → 启动 ReadTask
ReadTask → 增量保留半包 → 解码完整帧 → 分发回调
disconnect → 关闭写半部 → 停止读任务 → 至多通知一次
Drop/超时 → force_abort，不等待断开回调
```

不要推测 `_uid`、`push_tag`、`latest_login_time` 等未被测试证明的业务含义。

- [ ] **Step 3：补充客户端测试说明**

说明重复连接、主动断开、Drop、登录帧、断开回调竞态和发送取消的测试意图。

- [ ] **Step 4：启用文档 lint 并验证**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat/Cargo.toml` 增加：

```toml
[lints.rust]
missing_docs = "warn"
```

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all --check
cargo test -p im-chat
cargo clippy -p im-chat --all-targets -- -D warnings
cargo doc -p im-chat --no-deps
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat
git commit -m "docs: explain chat client lifecycle"
```

Expected: 28 tests passed，无 Rustdoc/Clippy 文档告警。

### Task 6：注释 HTTP 帧与客户端基础设施

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/lib.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/client.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/http_clients.rs`

- [ ] **Step 1：记录基线**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-http
cargo clippy -p im-http --all-targets -- -D warnings
```

Expected: 35 tests passed，1 ignored，Clippy exit 0。

- [ ] **Step 2：翻译 HTTP 帧文档**

说明 Gateway `0xC0` 与 im-biz `0xC1` marker、`[2B head][4B BE length][content]`、AES 后超过 5 KiB 才 gzip 的 Java 兼容阈值、响应逆变换以及解压上限。

- [ ] **Step 3：注释 HTTP 客户端工厂和限流读取**

为 `HTTP_REQUEST_TIMEOUT`、`MAX_HTTP_RESPONSE_SIZE`、`AppHttpClients`、`build_http_client` 和 `read_response_body_limited` 说明配置来源、Content-Length 预检、分块累计限制和 OOM 防护。

- [ ] **Step 4：验证并提交**

```bash
cargo fmt --all --check
cargo test -p im-http
cargo clippy -p im-http --all-targets -- -D warnings
cargo doc -p im-http --no-deps
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/lib.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/client.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/http_clients.rs
git commit -m "docs: explain HTTP framing safeguards"
```

### Task 7：注释 im-biz 群组客户端

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/im_biz.rs`

- [ ] **Step 1：注释公开 API 与 DTO**

为 `ImBizClient`、`GroupInfo`、`new` 和 `fetch_group_list` 说明请求路径、X-One、Protobuf body、二进制帧和业务成功码 `200`。

- [ ] **Step 2：说明兼容映射**

解释 `Content-Type: application/json; charset=utf-8` 虽与二进制 body 不匹配但为 Java 兼容行为；解释 `GroupContactListReq` field 1 直接承载 `ClientInfo`，`host_id == 0` 映射为 `None`。

- [ ] **Step 3：注释六个测试场景并验证**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all --check
cargo test -p im-http im_biz
cargo clippy -p im-http --all-targets -- -D warnings
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/im_biz.rs
git commit -m "docs: explain im-biz group protocol"
```

Expected: 筛选到的 im-biz 测试全部通过。

### Task 8：注释 OpenChat 用户认证客户端并启用 lint

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/src/openchat_user.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/tests/openchat_issued_live.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/Cargo.toml`

- [ ] **Step 1：注释响应 envelope 与错误模型**

为 `ApiResponse<T>`、`ApiBusinessError`、`OpenChatUserError` 和 `parse_api_response` 说明 `{code,msg,data,...}` 结构及 `code == 200` 成功契约。

- [ ] **Step 2：注释枚举、宏和 DTO**

说明 `integer_enum!` 的 i32 wire 契约、未知值拒绝、camelCase 和 `gt4DTO` 等特殊字段名；为 `LoginReq::validate` 说明不同 `LoginType` 的必填字段。

- [ ] **Step 3：注释脱敏与 token 兼容**

说明 `sanitize_debug_json` 递归脱敏范围；说明 access token 可来自 `token`、`access_token` 或 `accessToken`，不要在注释中记录真实凭据。

- [ ] **Step 4：注释七个公开端点和四层 POST 辅助**

逐一记录短信、邮件、issued、verify、`listPedingValidate`、login 和 user detail 的路径、匿名/认证头差异、请求验证和错误返回。服务端路径 `Peding` 的拼写必须保留并说明兼容原因。

- [ ] **Step 5：注释内联与 live 测试**

说明业务错误保真、serde 契约、密码字段跳过、脱敏和 X-Ten 的测试意图；live 测试必须注明默认忽略、依赖真实服务且不得记录账号/token。

- [ ] **Step 6：启用 im-http 文档 lint 并验证**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http/Cargo.toml` 增加：

```toml
[lints.rust]
missing_docs = "warn"
```

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all --check
cargo test -p im-http
cargo clippy -p im-http --all-targets -- -D warnings
cargo doc -p im-http --no-deps
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-http
git commit -m "docs: explain OpenChat authentication APIs"
```

Expected: 35 tests passed，1 ignored；无文档告警。不把 live 测试作为离线验收硬门槛。

### Task 9：注释 SQLite 存储层并启用 lint

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/src/lib.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/src/schema.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/src/message.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/src/group.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/src/tests.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/Cargo.toml`

- [ ] **Step 1：记录基线**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-store
```

Expected: 16 tests passed。

- [ ] **Step 2：翻译 crate 与 schema 文档**

说明 `SqliteStore` 聚合消息和群组仓储；说明旧库 `available` 列迁移、索引补建和 schema 中 `raw_proto`、`monitored`、`available` 的用途。

- [ ] **Step 3：注释消息与群组仓储**

为分页上限、记录 DTO 和公开方法补 Rustdoc；明确 `INSERT OR REPLACE`、倒序分页、远程快照软隐藏、事务回滚、upsert 不覆盖用户 `monitored` 选择及 `rows_affected == 1` 返回语义。

- [ ] **Step 4：注释复杂测试**

重点说明触发器模拟中途失败、并发监控开关、群组消失后历史消息保留、旧 schema 迁移和分页溢出拒绝。

- [ ] **Step 5：启用 lint、验证并提交**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store/Cargo.toml` 增加：

```toml
[lints.rust]
missing_docs = "warn"
```

Run:

```bash
cargo fmt --all --check
cargo test -p im-store
cargo clippy -p im-store --all-targets -- -D warnings
cargo doc -p im-store --no-deps
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-store
git commit -m "docs: explain SQLite storage semantics"
```

Expected: 16 tests passed，无文档告警。

### Task 10：注释 Tauri 入口和连接状态协调器

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/build.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/mod.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/state.rs`

- [ ] **Step 1：记录基线**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app
cargo build -p im-app
```

Expected: 59 tests passed，构建 exit 0。

- [ ] **Step 2：注释入口与 IPC 边界**

说明 Tauri build hook、tracing、数据库目录、`AppState` 初始化顺序、14 个 command 分组和应用退出取消。为 `parse_i64_id` 说明 JavaScript 大整数必须以十进制字符串跨 IPC。

- [ ] **Step 3：注释状态模型**

为 `AuthSession`、`ConnectionPhase`、`ConnectionState`、`ConnectionPermit`、`ConnectionAttemptKey`、`InstalledClient`、`ClientSlot` 和 `AppState` 说明字段职责与锁保护对象。

- [ ] **Step 4：注释 ConnectionCoordinator 不变量**

按方法组说明：

```text
generation：认证切换时递增，使旧连接尝试失效
attempt_id：同一 generation 内标识一次连接尝试
status_publication：串行化 UI 状态事件，防止乱序
install_if_current：只有 generation、attempt 和 phase 均匹配时才能安装客户端
```

- [ ] **Step 5：注释九个并发测试并验证**

```bash
cargo fmt --all --check
cargo test -p im-app
cargo clippy -p im-app --all-targets -- -D warnings
cargo build -p im-app
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/build.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/mod.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/state.rs
git commit -m "docs: explain application connection state"
```

Expected: 59 tests passed，构建和 Clippy exit 0。

### Task 11：注释认证与群组命令

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/groups.rs`

- [ ] **Step 1：注释认证类型和 command**

为错误 DTO、登录结果联合类型和七个认证 command 说明 IPC 参数、业务错误、密码处理和副作用。

- [ ] **Step 2：说明密码哈希与登录状态流**

明确登录密码采用带固定盐的双 MD5，交易密码采用双 MD5；说明：

```text
begin_auth_transition
→ HTTP login
→ challenge 或补齐 uid
→ 拉取并同步群组
→ 原子发布 session/monitoring_groups
→ 后台启动 TCP 自动连接
```

说明 generation 门禁避免旧登录覆盖新会话；不得把业务码 `3114179` 扩写为未被代码证明的产品含义。

- [ ] **Step 3：注释群组同步并发**

说明远程 fetch 不持 `group_ops` 锁，落库和监控集合更新串行；toggle 数据库失败时不修改内存集合；所有跨 IPC 的 ID 使用字符串。

- [ ] **Step 4：注释相关测试并验证**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all --check
cargo test -p im-app
cargo clippy -p im-app --all-targets -- -D warnings
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/groups.rs
git commit -m "docs: explain authentication and group flows"
```

### Task 12：注释聊天命令、消息管道并启用 im-app lint

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/chat.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/Cargo.toml`

- [ ] **Step 1：注释常量、DTO 与 command**

说明连接/登录/断开超时、30 秒心跳、容量为 8 的消息队列、8 MiB 单消息上限、`MessageDto.content_b64` 和四个聊天 command 的错误及副作用。

- [ ] **Step 2：注释连接 Guard 与自动重连**

说明 `ConnectionAttemptGuard` Drop 清理、初次连接阶段、登录成功推送 `1201`、指数退避重连及 stale generation 退出条件。

- [ ] **Step 3：注释消息 worker**

记录 message ID：

```text
1201：TCP 登录成功，释放等待中的 oneshot
2202：群消息；按监控集合决定是否持久化和发 new_message，但始终发送 2102 回执
2205：撤回消息；当前仅识别，尚未实现业务处理
```

说明队列满、超大消息和解码失败采取 fail-closed，取消后关闭 receiver 并丢弃剩余队列。

- [ ] **Step 4：注释断开、广播和 31 个测试场景**

覆盖分页、背压、断开超时、guard Drop、stale generation、Base64 DTO 和消息监控过滤。

- [ ] **Step 5：启用 im-app 文档 lint**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/Cargo.toml` 增加：

```toml
[lints.rust]
missing_docs = "warn"
```

确保所有 `pub async fn` Tauri command 均有中文 Rustdoc，不用模块级 `allow` 掩盖缺口。

- [ ] **Step 6：验证并提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all --check
cargo test -p im-app
cargo clippy -p im-app --all-targets -- -D warnings
cargo build -p im-app
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/Cargo.toml /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/chat.rs
git commit -m "docs: explain chat command orchestration"
```

Expected: 59 tests passed，无文档告警。

### Task 13：注释前端类型、IPC 与纯函数

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/vite.config.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/types/im.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/protocol.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/message.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/protocol.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/message.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/vite-config.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/package-proxy.test.ts`

- [ ] **Step 1：记录前端基线**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm run typecheck
npm test
npm run build
```

Expected: typecheck exit 0；10 个测试文件、53 tests passed；build exit 0。

- [ ] **Step 2：注释类型和 IPC**

为所有导出 DTO、枚举和联合类型补 JSDoc，说明字符串 ID、`content_b64`、`monitored`、challenge 和连接状态；为 `api` 的每个方法说明对应 Tauri command、参数包装和默认分页。

- [ ] **Step 3：注释协议与消息工具**

说明 GT4 四字段归一化、未知连接状态降级、结构化错误格式、Base64 UTF-8/二进制回退、消息去重排序和 1000 条上限、request ID 竞态防护。

- [ ] **Step 4：注释 Vite/CSP 与测试场景**

说明开发 CSP 必须与 Tauri `devCsp` 一致、1420 strict port 和 GT4 域名；测试仅按 describe/复杂场景添加说明。

- [ ] **Step 5：验证并提交**

```bash
npm run typecheck
npm test
npm run build
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/vite.config.ts /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/types /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/vite-config.test.ts /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/package-proxy.test.ts
git commit -m "docs: explain frontend IPC contracts"
```

### Task 14：注释前端 composable

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useGt4.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useMonitor.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useGt4.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useMonitor.test.ts`

- [ ] **Step 1：注释 GT4 生命周期**

说明本地/CDN 回退、失败缓存清理、generation 取消、成功结果单次消费、snake_case 映射和组件卸载销毁。

- [ ] **Step 2：注释认证流程**

为 `useAuth`、method contract、issued→verify→login、业务码 `3114169` 恢复、challenge 映射、pending 合并和 GT4 账号快照补中文说明。`ValidateType >= 23` 明确为“不推测 loginType”。

- [ ] **Step 3：注释监控会话**

说明历史消息 request ID、500 ms 状态轮询版本、Tauri 事件清理、实时消息合并，以及 logout 失败仍清本地状态并产生 warning 的策略。

- [ ] **Step 4：注释复杂测试并验证**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm run typecheck
npm test
npm run build
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables
git commit -m "docs: explain frontend state workflows"
```

Expected: 53 tests passed，类型检查和构建通过。

### Task 15：注释 Vue 组件、入口和样式分区

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/main.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/GroupSidebar.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/StatusBadge.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/GroupSidebar.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/base.css`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.css`

- [ ] **Step 1：注释组件职责和事件契约**

按文件说明：App 组合认证与监控；LoginPanel 支持主登录/challenge；GroupSidebar 区分选中与监控开关；MessagePanel 管理四种展示态和自动滚动；StatusBadge 映射连接状态。

- [ ] **Step 2：按模板区域添加注释**

只标注登录/主界面、全局警告、challenge、群列表、消息四态等区域，不在每个 `<label>`、`v-if` 或事件绑定前添加注释。

- [ ] **Step 3：按 CSS 区域添加注释**

`base.css` 划分设计令牌、全局基元、表单、按钮和工具类；`console.css` 划分登录页、控制台、群组侧栏、消息面板、动画、响应式和减少动画。禁止逐条解释属性。

- [ ] **Step 4：验证并提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm run typecheck
npm test
npm run build
git diff --check
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/main.ts /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles
git commit -m "docs: explain Vue components and styles"
```

### Task 16：启用 im-proto lint 并执行全仓验收

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/Cargo.toml`
- Modify only if omissions are found: all files listed in Tasks 2–15

- [ ] **Step 1：启用 im-proto 文档 lint**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/Cargo.toml` 增加：

```toml
[lints.rust]
missing_docs = "warn"
```

生成模块只允许在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/src/lib.rs` 局部 `#[allow(missing_docs)]`，不得扩大到整个 crate。

- [ ] **Step 2：检查说明性英文注释残留**

```bash
rg '^\s*(//|///|//!|/\*|\*)[^\\n]*[A-Za-z]{4,}' /Volumes/TRANSCEND/works/objects/rust/broadcast \
  --glob '*.rs' --glob '*.ts' --glob '*.vue' --glob '*.css' --glob '*.proto' \
  --glob '!target/**' --glob '!**/node_modules/**' --glob '!**/dist/**' \
  --glob '!im-app/ui/public/vendor/**'
```

Expected: 人工逐条确认只剩协议名、字段名、算法名或无法翻译的代码标识；普通英文说明应翻译为中文。

- [ ] **Step 3：检查公开 Rust API 文档**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo doc --workspace --no-deps
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: exit 0，无 `missing documentation`。

- [ ] **Step 4：执行完整 Rust 回归**

```bash
cargo fmt --all --check
cargo test --workspace --all-targets
cargo check --workspace --all-targets
```

Expected: 全部 exit 0；既有测试全部通过且没有行为变化。

- [ ] **Step 5：执行完整前端回归**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm run typecheck
npm test
npm run build
```

Expected: typecheck 和 build exit 0；10 个测试文件、53 tests passed。

- [ ] **Step 6：复核 diff 边界**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git diff --check
git status --short
git diff --stat
```

Expected: 没有生成物、第三方代码、锁文件或业务逻辑变更混入；用户原有未跟踪文件保持不变。

- [ ] **Step 7：提交最终 lint 与补漏**

```bash
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/Cargo.toml /Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto/src/lib.rs
git commit -m "chore: complete Chinese documentation baseline"
```

如果 Step 2–6 发现其他注释遗漏，只将对应源码一并加入本次提交；不得修改行为代码来“顺便修复”问题。
