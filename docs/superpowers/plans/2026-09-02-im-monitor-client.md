# IM 群消息监控客户端 — 当前实施计划与状态

> 初始日期：2026-09-02
>
> 状态同步：2026-09-03
>
> 工作区：`/Volumes/TRANSCEND/works/objects/rust/broadcast`
>
> 当前结论：代码与自动测试完成，真实端到端联调未完成

## 1. 事实入口

本文记录当前执行状态；详细设计事实见 `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/specs/2026-09-02-im-monitor-client-design.md`。事实来源是当前未提交 working tree、六个 Cargo manifest、Tauri/Vite/npm 配置、Rust/Vue 源码与 2026-09-03 实际验证输出。

技术基线：

- Rust 六 crate workspace：`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common`、`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto`、`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http`、`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat`、`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store`、`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app`；
- Tauri v2、Tokio、reqwest、prost/prost-build、sqlx SQLite、AES/ECB/PKCS7、flate2 gzip；
- Vue 3、TypeScript、Vite、Composition API、Vitest、Vue Test Utils、jsdom；
- 当前没有 Pinia、Vue Router 或 UI 组件库；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/proto/broadcast.proto` 是当前唯一 protobuf 源，构建机需提供 `protoc`。

## 2. 已完成的本地实现

### 2.1 HTTP 与认证

- gateway/im-biz wire 均为 `[2B head][4B big-endian length][payload]`，request marker 分别为 `0xC0`/`0xC1`；
- request 执行 plaintext → AES → optional gzip，response 执行 optional ungzip → decrypt；
- wire payload 8 MiB、明文/解压后 32 MiB，HTTP 响应另有 Content-Length 与 chunk 累计限制和 15 秒总超时；
- X-One 和 X-Ten 已实现；X-Ten 是 ClientInfo JSON 加 `//` 加时间戳后 AES hex，并支持登录后携带 access token；
- openchat-user 使用 X-One、X-Ten 和 `application/octet-stream`；im-biz 使用 X-One 和 `application/json; charset=utf-8`；
- `GroupContactListReq` 直接携带 `ClientInfo`；
- `GroupContactListResp.GroupBase` 字段 1–17 已与 Java protobuf 对齐，字段 8 为 bool `bfJoinFriend`，不再按旧 schema 的 string `desc` 解码；
- im-biz protobuf `CommonResult.errCode` 按 Java 服务端约定以 `200` 为成功；
- 已实现 `sendSmsCaptchaWithGt4`、`sendEmailCaptchaWithGt4`、issued、verify、`listPedingValidate` 和 login；
- LoginType 1–9、ValidateType 16–26 均有严格枚举；
- 主 UI 实现手机验证码、邮箱验证码、手机密码、邮箱密码四种模式；
- 邮箱主验证、二次验证和 login 请求统一显式携带 `countryCode: 0`，手机请求保留实际国家区号；
- 密码只作为 verify 的 `validateValue`，login request 不含 password；Tauri command 层已按 Java `PwdUtil` 规则转换登录密码（ValidateType 20/21）和交易密码（ValidateType 18）；
- 二次验证为邮箱/手机验证码（ValidateType 16/17）时，UI 会再次执行 GT4，再通过未登录验证码接口向原始完整账号发送验证码，并映射为二次 LoginType 2/1；
- `3114179` 返回 challenge，不创建 session；16/17/18/19/20/21/22 分别映射二次 login 2/1/8/9/3/4/7，23–26 只重试原 login，不猜测映射；登录返回 `3114169` 时使用当前 token 查询 `listPedingValidate`，有结果则继续验证，为空或失败则保留原错误；
- 登录成功响应缺少 uid 时，使用 access token 调用 `/user/user/userDetail` 并读取 `userBase.uid` 后再建立会话；开发日志会脱敏 access/refresh token；
- verify 的 `businessProcessing` 已显示在 UI。

### 2.2 GT4

- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useGt4.ts` 加载 `https://static.geetest.com/v4/gt4.js` 并使用 bind product；
- captchaId 来自公开环境变量 `VITE_GT4_CAPTCHA_ID`；当前 `plat=0`、`appVer=680` 的公开默认值为服务端 Android 640+ 配置对应的 `d7b9e5c52c8d9d8b214bc7a4c6db1f4f`，示例位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/.env.example`；没有 captchaKey；
- SDK `onSuccess` 的 snake_case 已转换为 IPC camelCase；
- 主验证和二次验证发送手机/邮箱验证码前都必须先通过滑块；结果一次性消费，成功销毁、失败重置，并受 generation/卸载保护；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json` 的生产/dev CSP 均允许实际 GT4 域名；dev 另开放 1420 的 HTTP/WebSocket。

### 2.3 Tauri、存储与并发

实际注册 13 个 command：

- `login`、`logout`、`send_sms_code`、`send_email_code`、`issue_validation_token`、`verify_validations`、`list_pending_validations`；
- `fetch_group_list`、`refresh_group_list`、`toggle_monitor`；
- `connect_chat`、`disconnect_chat`、`get_messages`。

认证 request struct 通过 `{ request }` IPC wrapper 传入。Rust/protobuf/SQLite 的 ID 保持 `i64`，跨 JavaScript 边界的 uid/group/host/message/sender ID 使用十进制字符串。

认证和连接使用 generation、attempt ID、取消 token 及连接 slot 原子操作，旧登录/旧连接/旧重连不能覆盖新会话。登录群同步、远端群快照和监控开关共用 `group_ops` 锁；远端 fetch 在锁外执行，落库、恢复 `monitoring_groups` 与 toggle 串行，修复了快照/开关竞争。

群同步使用 `available` 完整快照语义，不物理删除历史群；不可用群不进入当前列表或运行时监控集合，重新出现时恢复原监控选择。消息队列容量 8，单条上限 8 MiB，队列满/超限主动失败，不静默丢消息；消息先持久化再发事件。

### 2.4 TCP

- TCP wire：`[2B head][2B message_id][4B content_length][content]`；
- TCP 变换顺序与 Java im-chat 一致：发送时 AES → optional gzip，接收时 optional ungzip → AES decrypt；
- 所有客户端 TCP 请求设置 `encryptedSystemVersion`，payload 使用 `[4B X-One 长度][X-One][加密消息体]`，AES 后达到 128 字节时启用 gzip；
- client 在 message id `1100` 发送 `LoginSessionMessage`，request 中的 `clinet_info` 才携带 ClientInfo/token；
- server message id `1201` 必须按 `PushLoginSuccessMessage` 解码，不含 `clinet_info` 或 token；
- 已删除 1201 需要 `clinet_info`/token 的错误前提；畸形 `PushLoginSuccessMessage` 失败，合法的零 `login_time` 也可成功；
- HTTP 登录和群同步成功后由后端后台自动连接 TCP；首次失败保持登录状态并指数退避重试。心跳、帧增量读取、AES/gzip、8 MiB/32 MiB 限制、自然断线重连和显式取消已有本地实现及测试。

### 2.5 前端与启动

前端位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui`。Tauri hooks 实际是 `npm run dev` 和 `npm run build`，由 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/package.json` 代理到 UI package，兼容 Tauri 以 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app` 为 cwd 的行为。Vite 和 Tauri devUrl 都使用 1420。

开发启动：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app
cargo tauri dev
```

前端单独测试和构建：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm test
npm run typecheck
npm run build
npm audit
```

生产前端输出位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/dist`。

## 3. 未完成工作

### 3.1 真实 openchat-user

- 用可撤销测试账号完成 GT4、短信和邮箱发送联调；
- 核对 issued/verify/`listPedingValidate`/login 的真实返回及 challenge；
- 验证重复登录、退出、超时、业务错误不会留下部分 session。

完成标准：不手工改包即可完成四种主 UI 模式中的适用登录链路，且文档不记录账号、验证码、token 或密钥。

### 3.2 真实 im-biz

- 验证 `GroupContactListReq(ClientInfo)`、X-One、`0xC1` framing、AES response 和实际 Content-Type 兼容性；
- 核对群 ID、host ID、名称、头像、成员数和业务错误；
- 验证失败回滚、大整数 ID 和本地监控选择保持。

### 3.3 真实 im-chat

- 验证 1100 `LoginSessionMessage` 和 1201 `PushLoginSuccessMessage`；
- 验证心跳周期、2202 群消息 push、解密/解压/protobuf decode/持久化；
- 验证服务端断开、网络中断、指数退避、显式取消和退出后不复活。

不得再把 1201 描述为 `LoginSessionMessage`，也不得要求它回显 `clinet_info` 或 token。

### 3.4 桌面交付与后续功能

- 目标平台 Tauri 打包、签名、安装和冒烟未完成；
- 正式 MessageExtractor 未实现，当前只保留原始消息数据；
- 按群/发送人/类型/时间的统计、聚合、面板与导出未实现。

因此不得宣称真实 openchat-user/im-biz/im-chat/GT4 端到端完成。

## 4. 完整验证命令与 2026-09-03 结果

Rust 和工作区检查：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test --workspace --all-targets
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
git status --short
```

前端检查：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm test
npm run typecheck
npm run build
npm audit
```

2026-09-03 实际运行结果：

- `cargo test --workspace --all-targets`：149 passed，0 failed；
- `cargo check --workspace --all-targets`：通过；
- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets -- -D warnings`：通过；
- `npm test`：10 个测试文件、44 passed，0 failed；
- `npm run typecheck`：通过；
- `npm run build`：通过，Vite 生产资源生成成功；
- `npm audit`：0 vulnerabilities；
- `cargo tauri build --no-bundle`：通过，包含根 package 代理的前端构建钩子；
- `git diff --check`：通过。

自动测试通过不覆盖真实服务或桌面签名验证。

## 5. 历史任务与 commit 映射

以下仅是已完成阶段的历史落点，不代表当前未提交 working tree：

- Task 1：`1224049`、`8fc190e`
- Task 2：`c67fa2c`
- Task 3：`3007b41`
- Task 4/5：`33c5620`
- Task 6：`9642b3d`
- Task 7：`b3a4e07`
- Task 8：`7a9433e`
- Task 9：`79d6981`
- Task 10：`4919c2e`
- Task 11：`30bbe41`，其中旧 vanilla 前端已被 Vue 实现替代
- Task 12：`803ecde`
- Task 13：`f25032a`
- Task 14：`ecc5902`
- Task 15：`3e67574`
- Task 16：无 commit，旧任务已被当前 Vue 实现替代
- Task 17：`5472593`

## 6. 工作区与安全约束

- 当前源码、配置、前端和本文档包含未提交修改；历史 commit 不代表当前完整内容；
- 本轮只允许修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/specs/2026-09-02-im-monitor-client-design.md` 与 `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/plans/2026-09-02-im-monitor-client.md`；
- 不执行 add、commit 或 push；
- 不在文档、日志、bundle 或测试中泄露 AES/header 密钥、token、手机号、邮箱、验证码或真实账号；
- captchaId 是公开站点标识，可以记录；captchaKey 等服务端秘密不得进入前端或文档；
- 本轮 markdown 没有专用 lint，使用 `git diff --check` 检查格式。
