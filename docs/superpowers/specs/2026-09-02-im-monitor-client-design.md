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

实际注册的 15 个 command：

- 认证：`login`、`logout`、`send_sms_code`、`send_email_code`、`issue_validation_token`、`verify_validations`、`list_pending_validations`；
- 群组：`fetch_group_list`、`refresh_group_list`、`toggle_monitor`；
- 连接/消息：`connect_chat`、`disconnect_chat`、`get_connection_status`、`get_messages`、`download_message_attachment`。

前端服务 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts` 对所有带 request struct 的认证 command 使用 `{ request }` 包装；无参 command 不包装。`toggle_monitor`、`get_messages` 按各自 command 参数名直接传参。

Rust/protobuf/SQLite 内部 ID 使用 `i64`。跨 Tauri/JavaScript 边界的 `uid`、`group_id`、`host_id`、`msg_id`、`send_uid` 使用十进制字符串，Rust 收到字符串后校验并解析回 `i64`；时间戳和受限计数保持 number。

并发修复以 generation、attempt ID、`CancellationToken` 和连接 slot 的原子安装/移除隔离旧登录、旧连接、旧重连及旧状态事件。登录先推进 generation 并清理旧 session，只有远端认证和群同步都成功且 generation 仍有效时才发布新 session。群刷新、登录后的群快照落库/监控集合恢复和 `toggle_monitor` 共用 `group_ops` 锁，避免群快照与用户开关互相覆盖；远端 HTTP fetch 不占用该锁。

后端 `connection_status` 事件是前端连接状态的唯一权威来源。HTTP 登录和群同步成功后，后端在不阻塞登录结果的后台任务中自动连接 TCP；首次连接失败保持登录状态并按指数退避重试，新登录、退出或应用关闭会使旧任务失效。

实时消息链路使用两层有界队列和端到端字节许可：

- TCP 回调进入容量 64 的 Tokio 帧队列；所有尚未完成 pending、投影排队或活动投影的帧正文共享 32 MiB 许可，单个已解码帧仍限制为 8 MiB；
- 2202 在 Prost 分配对象前预扫描顶层 `group_msg`，单帧最多 10,000 条，超限或 wire 畸形只丢弃该帧；
- 每个 2202 帧只读取一次监控群集合快照，消息按先到的“100 条或首条等待 25ms”条件刷新；
- 监控消息按微批在一个 SQLite 事务中 UPSERT；数据库文件启用 WAL、`synchronous=NORMAL` 和 5 秒 `busy_timeout`，任一行失败回滚整批；
- 提交成功后，在当前微批内按 `group_id` 合并并按协议顺序发送 2102；未监控消息无需落库即可回执，监控消息只有事务成功才进入回执，回执失败会取消连接；
- 已提交且已回执的监控批次进入容量 8 的投影队列；每批最多 8 路并发解密，保序收集后通过一次 `Channel<Vec<MessageDto>>` 跨越 WebView 边界；
- Vue 的 `MessageIndex` 以 `Map<msg_id, MessageDto>` 去重，并按 `(send_time, msg_id)` 稳定升序合并；实时与首屏超限时裁掉较早端，向上翻页超限时裁掉较新端，数组和 Map 双向同步且最多保留 1,000 条；
- `MessagePanel` 使用 TanStack Virtual 的动态高度测量，仅挂载视口附近及 overscan 行。历史查询使用 `(send_time, msg_id)` 复合 keyset 游标，每页上限 200，不使用 `OFFSET`。历史请求带单调递增 token；面板在 `loadingOlder` 从 true 变为 false 后由单一协调 watcher 完成锚点恢复或无新增/失败收尾，再回传 `older-settled(token)`。父级只接受当前范围、当前轮次的握手，握手前不发布期间缓冲的实时消息。

Vue 页面挂载时通过 `register_message_channel` 登记 Channel，热重载或页面重建会用新 Channel 替换旧接收端。实时消息不依赖轮询、选择群组或点击刷新；每个 Channel 批次过滤后只进行一次响应式状态提交。Channel 未登记、发送失败或单条解密失败只产生警告或可见错误，不撤销已经完成的数据库事务及 2102。

消息视图默认调用 `get_messages(group_id=None)`，读取当前可用且仍受监控群组的最近消息；选择单群后传入群 ID，再次点击当前群或点击“全部消息”恢复全量。全量 DTO 携带 `group_name` 和字符串 `group_id`，实时事件在全量模式接受所有受监控群消息，单群模式只合并当前群。

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

### 6.1 群消息与媒体附件解密

`GroupMessage.version != 0` 表示应用层正文已加密。im-chat 1201 的 `user_key_pair` 由 `getAppNewestKeyPair` 映射，只包含公钥和版本，不包含私钥；缺少私钥不能判定 TCP 登录失败。客户端通过 im-biz 旧版 `POST /sys/getKeyPair` 请求 `client_info + KEY_GROUP + group_id`，并从响应 field 2 的通用 `key_pair` 读取群 `public_key` 和包装后的 `msg_key`。

服务端会用当前用户 App 公钥和群私钥产生 Curve25519 共享值，再以 AES-128-ECB/PKCS7 包装群 `msg_key`；因此返回值仍需要对应的本地用户私钥才能还原。客户端在 1201 后比较服务端 App 公钥和版本与 SQLite `user_key_pairs` 的本地最新记录：完全匹配时恢复私钥；缺失或不匹配时生成新的 X25519 密钥对，通过 im-biz `POST /sys/updateUserKeyPair` 只上传公钥，并用响应版本持久化完整本地密钥对。私钥不进入日志、HTTP 请求或前端 DTO。

密钥同步在连接发布为 `connected` 后独立运行，失败不撤销 TCP 在线状态，也不影响原始消息入库或 2102 回执。同步完成后后端发送 `message_keys_ready`，Vue 重新读取当前消息范围，使同步前暂时显示解密提示的历史和实时消息得到再次解密。该流程使用账号级 App 公钥槽，适用于监控专用账号；同账号被多个独立 App 客户端同时轮换密钥会产生版本竞争。

群 `relKey` 解开 `GroupMessage.content` 和 HEX `attachment_key`。正文随后按 `msg_type` 解码为 `TextObj`、`ImageObj`、`AudioObj`、`VideoObj` 或 `FileObj`；原始 `content` 与完整 `raw_proto` 仍保留在 SQLite。附件采用裸 HTTP(S) GET，下载上限 256 MiB，再用解出的 `fileKey` 前 16 字节执行 AES-128-ECB/PKCS7。客户端通过首个 102416 字节密文块的独立填充块识别 PC“102400 字节明文分块”方案，否则按移动端整文件方案解密。

解密后的附件写入 Tauri `$APPCACHE/media`，`assetProtocol` 仅开放该目录；Vue 使用 `convertFileSrc` 展示图片、音频、视频或文件下载。单条正文或附件解密失败只产生可见错误，不删除原始消息。

群派生密钥缓存按 `(group_id, version)` 串联合并同键并发加载：缓存未命中后取得群级锁并二次检查，同群最多发起一次 `/sys/getKeyPair`，不同群仍可并行；失败不缓存并允许后续重试。缓存值与 generation 一同保存，fast path 只返回当前 generation；loader 完成后取得值表写锁，并在该临界区重查 generation 后才写入。会话清理先推进 generation 使旧值立即逻辑失效，再物理清表；因此旧 loader 与 clear 无论交错在检查前还是检查后，新代都不能读取旧值。

Vue 的 `MessageIndex` 最多保留 1000 条：正常实时流保留最新窗口，向上读取历史时保留更早窗口。历史请求进行期间到达的 Channel 批次进入容量同样受限、按消息 ID 去重的暂存区；历史页先发布，只有收到当前 token 的 `older-settled` 握手后才合并实时暂存。索引会返回批内唯一 ID 合并后实际裁剪的数量；只要实时 keep-latest 确实删除了消息，即使合并前窗口为空或此前历史页已报告 `hasMore=false`，前端也会重新设置 `hasOlder=true` 并把下一游标回退到新的可见最老键，使后续 keyset 查询重新覆盖被裁区间。未发生裁剪时保留后端末页状态。切群、退出、卸载和陈旧握手都会丢弃旧范围暂存。

### 6.2 错误、取消与顺序语义

- TCP 帧队列槽或 32 MiB 字节许可耗尽时，接收回调等待并向 socket 读取形成背压，不创建无界任务；单帧超过 8 MiB 时返回错误并断开当前连接。
- 2202 预扫描或 Prost 解码失败只丢弃当前帧。数据库事务失败时，当前微批的监控消息既不回执也不投影；同批未监控消息仍可回执。
- 连接取消优先于尚未开始的持久化、回执和投影。取消发生在事务提交前时不确认未提交消息；提交和回执已经完成后，等待投影或投影中的批次可以被丢弃，但 SQLite 历史仍保留。
- 2102 先按微批、再按群合并；每个群内 ID 保持输入协议顺序。只有该批事务成功的监控消息和无需持久化的未监控消息可以进入回执，回执 ID 不跨批猜测或重排。
- 投影 worker 串行消费批次；批内解密虽然最多 8 路并发，但最终 `Vec<MessageDto>` 恢复输入顺序。前端再以 `(send_time, msg_id)` 建立确定性展示顺序。

## 7. 启动、测试与构建

测试环境桌面开发启动的推荐命令：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app
npm run dev:test
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

负载验证可单独运行：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app message_worker_preserves_ten_thousand_message_burst_without_loss_or_duplicates -- --nocapture
cargo test -p im-store sqlite_batch_upserts_ten_thousand_rows_without_duplicate_primary_keys -- --nocapture

cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm test -- --run src/utils/message.test.ts
npm test -- --run src/composables/useMonitor.test.ts
npm test -- --run src/components/MessagePanel.test.ts
```

Rust 合成负载在单个合法 2202 帧中构造 10,000 条、两个监控群的消息，验证恰好 100 个不超过 100 条的事务批次、每批一次投影、2102 按群合并且只包含已成功提交的监控消息、持久化/投影/回执总 ID 集合与输入完全相等、无重复且协议顺序稳定。独立 SQLite 测试对 10,000 个主键执行批量写入和反序批量 UPSERT，验证最终行数、主键去重和更新结果。前端负载验证 `MessageIndex` 的 10,000 条突发、复合排序、去重、最终 1,000 条双向裁剪及 Map 一致性；Channel 的 10,000 条单批输入只提交一次 Vue 状态；虚拟列表接收索引裁剪后的 1,000 条并断言实际 DOM 行远少于总数，同时保留已有动态高度测试。

负载测试只打印单行 elapsed、峰值微批或已配置投影队列容量等观测信息，避免海量消息日志。elapsed 是诊断数据，不是可跨机器比较的性能承诺；CI 不设置硬耗时阈值。达到 100 条时立即刷新，不通过重复 `sleep(25ms)` 模拟 100 个定时批次；25ms 边界由 Tokio paused time 的独立测试覆盖。

2026-09-03 全套验证结果应以本次命令的最新输出为准，不沿用历史测试计数。自动测试通过不代表真实服务、目标平台打包或端到端性能已完成验证。

## 8. 状态与安全边界

已完成：六 crate 代码、HTTP/TCP framing、认证与 challenge 编排、GT4 前端绑定、群同步与 SQLite、连接 generation/重连、消息微批持久化与批量 Channel、群密钥与五种正文解析、媒体附件解密、复合游标历史分页、动态虚拟消息视图、Tauri IPC、Vue UI 及自动负载测试。这里的“完成”指本地实现和自动测试，不代表真实端到端可用。

仍未完成：

- 真实 openchat-user 的 GT4、短信/邮箱及完整登录联调；
- 真实 im-biz 群列表 wire/字段联调；
- 真实 im-chat 的 1100/1201、心跳、push、断线与重连联调；
- 目标平台打包、签名、安装和冒烟；
- 真实环境群密钥、图片、音频、视频和文件样本联调；
- 消息统计、聚合和导出。

不得宣称端到端完成。文档、日志、bundle 和测试不得泄露 AES/header 密钥、会话 token、手机号、邮箱或真实账号；captchaId 是公开站点标识，可以记录，但 captchaKey 等服务端秘密不得进入前端或文档。
