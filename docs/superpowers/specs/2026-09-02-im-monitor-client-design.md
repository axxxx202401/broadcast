# IM 群消息监控桌面客户端 — 设计事实

> 初始日期：2026-09-02
>
> 事实同步：2026-09-03
>
> 状态：代码与自动测试已完成；真实服务联调和桌面交付未完成

## 1. 范围与技术栈

项目根目录为 `/Volumes/TRANSCEND/works/objects/rust/broadcast`。这是一个 Tauri v2 桌面客户端，目标链路是 openchat-user 登录、im-biz 群同步、im-chat TCP 收取群消息、SQLite 持久化和 Vue 控制台展示。

Rust workspace 在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/Cargo.toml` 定义六个 crate：

- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-common`：AES/ECB/PKCS7、协议头、X-One/X-Ten、配置和共享错误；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-proto`：通过 `prost`/`prost-build` 编译 `/Volumes/TRANSCEND/works/objects/rust/broadcast/proto/broadcast.proto`；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-http`：基于 `reqwest`、Tokio、`prost`、AES 和 `flate2` gzip 的 openchat-user/im-biz client；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-chat`：基于 Tokio、`tokio-util`、`prost`、AES 和 gzip 的 TCP client、心跳与重连；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-store`：基于 `sqlx` SQLite driver 和 Tokio/Rustls runtime 的群组与消息存储；
- `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app`：Tauri v2 壳、IPC command、认证/群同步/连接编排。

前端位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui`，技术栈是 Vue 3、TypeScript、Vite、Composition API、Vitest、Vue Test Utils 和 jsdom；状态由 composable 管理。当前明确没有 Pinia、Vue Router 或 UI 组件库。

有效桌面配置只有 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/build.rs` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json`。旧的 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src-tauri` 架构已经删除。`prost-build` 当前没有 vendored protoc，构建环境仍需提供 `protoc`。

## 2. HTTP wire 与 headers

openchat-user gateway 和 im-biz 的 HTTP body 都使用：

```text
[2B head][4B payload_length, big-endian][payload]
```

- gateway request 的首字节 marker 是 `0xC0`；
- im-biz request 的首字节 marker 是 `0xC1`；
- head 第二字节用 bit 7 表示 AES、bit 6 表示 gzip；
- request 变换顺序是 plaintext → AES → optional gzip → framing；
- response 变换顺序是 unframe → optional ungzip → AES decrypt；
- gateway 会在 AES 后 payload 超过 5 KiB 时启用 gzip；当前 im-biz 默认 AES、不开 gzip；
- 编码后 wire payload 上限 8 MiB，明文或解压后 body 上限 32 MiB；
- HTTP client 还以 `Content-Length` 预检和 chunk 累计读取限制整体响应为 8 MiB payload 加 6 字节 framing，并使用 15 秒总请求超时。

Headers 的实际实现：

- X-One：`hex(AES(secretName + "," + timestampMs))`；
- X-Ten：`hex(AES(ClientInfo JSON + "//" + timestampMs))`；未认证请求的 session/token 为空，登录后查询用户信息时携带 access token；
- openchat-user 同时发送 X-One、X-Ten，`Content-Type` 是 `application/octet-stream`；
- im-biz 只发送 X-One，`Content-Type` 按现有 Java 兼容事实为 `application/json; charset=utf-8`，即使 body 是 protobuf 加密帧；
- `GroupContactListReq` 字段 1 直接是 `ClientInfo`，不是再包一层 `CommonResultReq`。
- `GroupContactListResp.groups` 的 `GroupBase` 与 Java protobuf 对齐：字段 1–7 为群 ID、群主、名称、图标、入群审核、创建时间、成员数；字段 8–17 为 `bfJoinFriend`、`bfShutup`、`bfGroupReadCancel`、`groupMsgCancelTime`、`bfBanned`、`groupAliasName`、`remark`、`maxMemberCount`、`bfJoinNotice`、`notice`。
- im-biz protobuf 响应中的 `CommonResult.errCode = 200` 表示成功，其他值按业务错误处理。

## 3. 登录协议与 UI

openchat-user 实际链路和路径为：

1. 手机发送：`/user/unauthorized/sendSmsCaptchaWithGt4`；
2. 邮箱发送：`/user/unauthorized/sendEmailCaptchaWithGt4`；
3. issued：`/user/unauthorized/issued`，发送 `validateScene=5` 和 `validateTypes`；
4. verify：`/user/unauthorized/verify`，发送 `validateToken` 和非空 `pendingValidateDTOS`；
5. 待验证列表：`/user/unauthorized/listPedingValidate`，服务端路径拼写就是 `Peding`；
6. login：`/sns/login/login`。

协议枚举完整范围：

- LoginType 1 手机验证码、2 邮箱验证码、3 手机密码、4 邮箱密码、5 注册、6 PC 扫码、7 人脸、8 交易密码、9 Google 验证码；
- ValidateType 16 邮箱验证码、17 手机验证码、18 交易密码、19 Google 验证码、20 手机登录密码、21 邮箱登录密码、22 人脸、23 Messenger 验证码、24 辅助验证、25 iToken 验证码、26 iToken 生物验证。

当前主 UI 只提供四种模式：手机验证码、邮箱验证码、手机密码、邮箱密码，对应 LoginType 1–4 和 ValidateType 17、16、20、21。流程是 issued → verify → login。手机/邮箱验证码发送前必须先完成 GT4 滑块。

邮箱验证码、邮箱密码及邮箱二次验证的 verify/login 请求显式携带 `countryCode: 0`；手机请求使用用户选择的国家区号。

密码模式中的用户输入只写入 verify 的 `pendingValidateDTOS[].validateValue`；login request 没有 `password` 字段，也不会把密码复制进 login。Tauri command 层在发送 verify 前按 Java `PwdUtil` 兼容规则转换：ValidateType 20/21 使用 `MD5(MD5(password) + "!@#b%^&*9")`，ValidateType 18 使用 `MD5(MD5(password))`，验证码等其他类型保持原值。

login 返回业务码 `3114179` 时，后端返回 challenge DTO，不创建 `AuthSession`，也不执行群同步。前端保存服务端 `validateToken`，合并 challenge 数据与 `listPedingValidate` 结果，再次 verify：

- ValidateType 16/17 在提交验证码前再次执行 GT4，并通过未登录验证码接口发送邮箱/手机验证码；优先使用原登录表单中的完整账号，避免服务端脱敏账号不可发送；验证成功后分别切换为 LoginType 2/1 并携带二次 `validateToken` 重试登录；
- ValidateType 18 映射二次 login 的 LoginType 8；
- ValidateType 19 映射 LoginType 9；
- ValidateType 20/21 分别映射 LoginType 3/4；
- ValidateType 22 映射 LoginType 7，并把本次验证值放入 `credentials`；
- ValidateType 23–26 没有已知独立 LoginType，verify 完成后只重试原 login request，不猜测映射。

任一登录请求返回业务码 `3114169`（该场景下验证项缺失）时，前端会使用该次登录请求携带的 `validateToken` 查询 `listPedingValidate`。返回非空则替换当前 challenge 列表并等待用户继续验证；返回为空或查询失败则保留原始业务错误，不自动循环重试。

当前登录成功响应可能只返回 OAuth access token，不包含 uid。Rust 在取得 token 后若发现 uid 缺失，会使用认证 X-Ten 调用 `/user/user/userDetail`，从 `data.userBase.uid` 补齐 uid，再同步群组并发布会话。开发日志必须脱敏 `access_token`、`refresh_token` 及其他凭据。

verify 返回的 `businessProcessing` 会在登录 UI 以业务码和消息显示；它不是会话成功信号。

## 4. GT4 bind 与 CSP

GT4 实现位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useGt4.ts`：

- script URL：`https://static.geetest.com/v4/gt4.js`；
- 初始化使用 `product: "bind"`、`language: "zho"`、`protocol: "https://"`；
- captchaId 从构建环境变量 `VITE_GT4_CAPTCHA_ID` 读取；测试与生产模板分别位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/config/.env.test.example` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/config/.env.production.example`，源码不提供真实 ID 回退值；
- 没有 captchaKey，也不得把服务端密钥放入前端；
- `onSuccess` 从 SDK 的 snake_case `lot_number`、`captcha_output`、`pass_token`、`gen_time` 映射成 IPC DTO 的 camelCase；
- 结果只用于紧接着的一次验证码发送，成功后销毁实例，失败时 reset；账号使用打开滑块时的快照，避免验证期间编辑造成串号；
- generation、一次性消费和卸载销毁限制异步回调及短生命周期。

生产 CSP 与开发 CSP 都实际允许 `https://static.geetest.com` 和 `https://*.geetest.com` 的脚本/连接/图片，并允许 `https://*.geetest.com` frame。开发环境另外允许 `http://127.0.0.1:1420` 与 `ws://127.0.0.1:1420`。两者保留 `object-src 'none'`、`base-uri 'self'`、`frame-ancestors 'none'`；`withGlobalTauri` 为 false。

## 5. Tauri IPC 与并发边界

实际注册的 13 个 command：

- 认证：`login`、`logout`、`send_sms_code`、`send_email_code`、`issue_validation_token`、`verify_validations`、`list_pending_validations`；
- 群组：`fetch_group_list`、`refresh_group_list`、`toggle_monitor`；
- 连接/消息：`connect_chat`、`disconnect_chat`、`get_messages`。

前端服务 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts` 对所有带 request struct 的认证 command 使用 `{ request }` 包装；无参 command 不包装。`toggle_monitor`、`get_messages` 按各自 command 参数名直接传参。

Rust/protobuf/SQLite 内部 ID 使用 `i64`。跨 Tauri/JavaScript 边界的 `uid`、`group_id`、`host_id`、`msg_id`、`send_uid` 使用十进制字符串，Rust 收到字符串后校验并解析回 `i64`；时间戳和受限计数保持 number。

并发修复以 generation、attempt ID、`CancellationToken` 和连接 slot 的原子安装/移除隔离旧登录、旧连接、旧重连及旧状态事件。登录先推进 generation 并清理旧 session，只有远端认证和群同步都成功且 generation 仍有效时才发布新 session。群刷新、登录后的群快照落库/监控集合恢复和 `toggle_monitor` 共用 `group_ops` 锁，避免群快照与用户开关互相覆盖；远端 HTTP fetch 不占用该锁。

后端 `connection_status` 事件是前端连接状态的唯一权威来源。HTTP 登录和群同步成功后，后端在不阻塞登录结果的后台任务中自动连接 TCP；首次连接失败保持登录状态并按指数退避重试，新登录、退出或应用关闭会使旧任务失效。消息进入容量 8 的 Tokio 有界队列，单条上限 8 MiB，队列满或超限时失败并断开，监控群消息先写 SQLite 再发布 `new_message`。

## 6. TCP 协议事实

TCP 帧是：

```text
[2B head][2B message_id, big-endian][4B content_length, big-endian][content]
```

`content_length` 不含前 8 字节。客户端 TCP 请求设置 `encryptedSystemVersion` 标志，并在 payload 前置 `[4B X-One 长度][X-One]`；服务端用 X-One 定位版本 `secretKey`。实际消息体按 Java im-chat 的顺序编码：plaintext → AES → 超过 128 字节时 gzip，再与 X-One 前缀一起 framing；服务端响应解码执行 unframe → optional ungzip → AES decrypt。wire body 上限 8 MiB，解压后上限 32 MiB。

必须区分两个 protobuf：

- client 发送 message id `1100`，payload 是 `LoginSessionMessage`；该 request 才包含协议中拼写为 `clinet_info` 的 `ClientInfo` 和 token；
- server 登录成功 push 是 message id `1201`，payload 必须解码为 `PushLoginSuccessMessage`；它包含 `login_time`、user/web key pair 和 web online 标志，不含 `clinet_info` 或 token。

因此 1201 不做 `clinet_info`/token 校验，也不能按 `LoginSessionMessage`、`LoginResp` 或 `CommonResult` 解码。畸形的 `PushLoginSuccessMessage` 才会使当前登录尝试失败；合法 payload（包括 `login_time=0`）可完成连接。

## 7. 启动、测试与构建

桌面开发启动的唯一推荐命令：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app
cargo tauri dev
```

`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json` 的 Tauri hooks 实际执行 `npm run dev` 和 `npm run build`。因为 hooks 的 cwd 是 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app`，根 package `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/package.json` 再代理到 UI package。Vite 固定监听 `127.0.0.1:1420`，Tauri devUrl 也是 `http://127.0.0.1:1420`。生产产物位于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/dist`。

前端单独验证：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm test
npm run typecheck
npm run build
npm audit
```

Rust 与文档检查：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test --workspace --all-targets
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
git status --short
```

2026-09-03 实际结果：

- Rust：149 项测试通过，0 失败；
- `cargo check --workspace --all-targets`：通过；
- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets -- -D warnings`：通过；
- Vue：10 个测试文件、44 项测试通过，0 失败；
- Vue typecheck、生产 build：通过；
- npm audit：0 vulnerabilities；
- `cargo tauri build --no-bundle`：通过，包含根 package 代理的前端构建钩子；
- `git diff --check`：通过。

## 8. 状态与安全边界

已完成：六 crate 代码、HTTP/TCP framing、认证与 challenge 编排、GT4 前端绑定、群同步与 SQLite、连接 generation/重连、消息持久化、Tauri IPC、Vue UI 及自动测试。这里的“完成”指本地实现和自动测试，不代表真实端到端可用。

仍未完成：

- 真实 openchat-user 的 GT4、短信/邮箱及完整登录联调；
- 真实 im-biz 群列表 wire/字段联调；
- 真实 im-chat 的 1100/1201、心跳、push、断线与重连联调；
- 目标平台打包、签名、安装和冒烟；
- 正式 MessageExtractor；
- 消息统计、聚合和导出。

不得宣称端到端完成。文档、日志、bundle 和测试不得泄露 AES/header 密钥、会话 token、手机号、邮箱或真实账号；captchaId 是公开站点标识，可以记录，但 captchaKey 等服务端秘密不得进入前端或文档。
