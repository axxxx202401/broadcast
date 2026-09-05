# IM Monitor（IM 群消息监控客户端）

一个基于 Rust + Tauri + Vue 3 的桌面应用程序，用于实时接收、展示和筛选聊天群组的推送消息，并为监控消息匹配开奖结果。

## 功能概述

- **多账号支持**：支持手机号/邮箱登录、短信验证码、图形验证（Geetest v4）等多因素认证流程，可在多个账号间自由切换
- **群组监控**：查看已关注的群组列表，一键开启/关闭消息监控；侧栏支持按名称或 ID 搜索
- **实时消息**：通过 TCP 长连接实时接收群消息，持久化至本地 SQLite 数据库（默认保留 7 天）
- **消息过滤**：默认只显示「匹配开奖规则」的消息；可切换为显示全部消息，或按单群筛选
- **富媒体支持**：支持文本、图片、音频、视频、文件等多种消息类型的解析与展示
- **离线存储**：所有消息与群组数据保存在本地，退出后仍可查阅历史记录
- **开奖匹配**：可配置第三方开奖 API；每条入站消息会与该期号的期号/哈希进行匹配，并在消息面板标注 `matched` 标记
- **消息清理**：退出应用时自动清理超过保留期（默认 7 天）的历史消息，避免本地存储无限增长

## 技术栈

| 层次 | 技术 |
|------|------|
| 后端运行时 | Rust 1.76+ / Tokio |
| 跨平台 GUI | Tauri v2 |
| 前端框架 | Vue 3 + TypeScript + Vite |
| 数据库 | SQLite (sqlx) |
| 协议序列化 | Protobuf (prost) |
| 加密传输 | AES-256-CBC / X25519 密钥交换 |

## 项目结构

```
broadcast/
├── im-common/     # 共享基础类型：AES 加解密、TCP 帧头解析、配置与错误定义
├── im-proto/      # Protobuf 协议定义与生成代码（broadcast.proto）
├── im-chat/       # TCP 长连接客户端：帧编解码、心跳、指数退避重连
├── im-http/       # HTTP 客户端：OpenChat 用户接口、IM 业务接口、私有帧协议、开奖 API
├── im-store/      # SQLite 持久化层：消息、群组、密钥对、开奖配置与保留策略
├── im-app/        # Tauri 桌面应用主入口（Rust 后端 + Vue 前端）
│   ├── src/       # Rust 命令与状态管理（账号/认证/群组/聊天/开奖 IPC）
│   └── ui/        # Vue 3 前端工程
├── proto/         # .proto 源文件目录
├── scripts/       # 构建辅助脚本（tauri-env.mjs）
└── docs/          # 设计文档与规格说明
```

### Crate 依赖关系

```
im-app ──► im-chat ──► im-common
      ──► im-http ──► im-common
      ──► im-store ──► im-common
      ──► im-proto
```

## 快速开始

### 环境要求

- Rust 1.76 或更高版本
- Node.js 18+ 与 pnpm / npm
- macOS：系统需安装 [Tauri 所需依赖](https://v2.tauri.app/start/prerequisites/#macos)
- macOS（可选）：`cargo install --locked cargo-apk`（如需构建 macOS 安装包）

### 配置环境变量

```bash
# 测试环境
cp config/.env.test.example config/.env.test
# 生产环境
cp config/.env.production.example config/.env.production
```

必须填写模板中的空值，尤其是：

- `IM_VERSION_SECRET_NAME`
- `IM_BODY_AES_KEY`
- `IM_HEADER_AES_KEY`

两个 AES key 必须恰好为 16 字节。详见 [docs/environment-build.md](docs/environment-build.md)。

### 安装依赖

```bash
# 前端依赖
cd im-app/ui
npm install
cd ../..
```

Rust 依赖在首次 `cargo build` 时自动拉取。

### 开发模式

```bash
cd im-app

# 前端 + Rust 后端同时运行
npm run dev            # 使用 .env.test
npm run dev:test       # 显式使用测试环境
npm run dev:production # 显式使用生产环境
```

环境变量可被当前进程的同名环境变量覆盖，主要用于 CI 和临时联调。

### 构建与打包

```bash
cd im-app

# 仅构建前端
npm run build

# 构建 Tauri 桌面应用
npm run build:test          # 测试环境构建
npm run build:production    # 生产环境构建

# 构建并直接运行
npm run build-run:production

# 运行构建脚本单元测试
npm run test:scripts
```

## 使用方式

### 1. 登录

启动应用后，在登录面板输入手机号/邮箱，完成以下流程：

- 请求发送验证码（短信或邮件）
- 填写 Geetest v4 图形验证结果
- 输入验证码并登录；如服务端触发二次校验，按提示补充密码/人脸/交易密码等

### 2. 切换/管理账号

顶部的账号菜单提供：

- **切换账号**：从已保存的账号列表中选择另一个会话
- **添加账号**：以空白表单进入登录页，不清除其他账号
- **退出登录**：断开 TCP 连接，清除本地会话视图（Token 保留在凭据库）
- **移除账号**：同时删除凭据和该账号下的本地数据库

### 3. 选择与监控群组

左侧面板展示已关注群组列表：

- **搜索**：可按群组名称或群组 ID 过滤
- **监控开关**：点击单群的开关，应用会建立/断开 TCP 长连接并持久化该群消息
- **刷新**：从远端同步最新群组快照，恢复被删群组与监控状态
- **监控摘要**：在「全部群消息」标题下展示当前正在监控的群 ID（默认显示 5 个，超出可展开）

### 4. 查看消息

右侧消息面板按时间顺序展示消息内容：

- **全部 / 单群**：侧栏点击群名进入单群视图；再次点击或点击顶部「全部」回到跨群视图
- **匹配过滤**：默认只显示 `matched=1` 的消息（与当前开奖规则匹配）；在顶部开关可切换为全部
- **历史分页**：滚动到顶部时自动加载更早一页；加载失败保留现有游标，可重试
- **富媒体**：图片、音视频、文件均支持预览与下载；附件保存在本地缓存后解密
- **开奖条**：面板顶部内嵌开奖信息条，显示本期/上期期号；点击可编辑开奖 API URL 与关注期号

### 5. 连接与断开

顶栏状态徽章显示当前连接状态（`disconnected` / `connecting` / `connected`）。点击「连接聊天」建立 TCP 长连接；点击「断开」主动断开。连接期间消息会持续推入并写入本地数据库。

### 6. 消息清理

每次退出应用时，会自动清理 7 天前（`MESSAGE_RETENTION_DAYS = 7`）的过期消息。登录完成后也会触发一次清理。清理在后台异步执行，不阻塞界面。

## 安全说明

- 消息正文采用 AES-256-CBC 加密传输，密钥通过 X25519 密钥交换协商
- 认证令牌与 App 密钥对存储在本地 SQLite，不落盘明文缓存
- CSP 策略限制外部资源加载，仅允许必要的验证服务域名
- 安装包的协议密钥在构建时注入，运行期间无法通过环境变量覆盖

## 协议说明

`proto/broadcast.proto` 定义了客户端与服务端通信的 Protobuf 消息格式，主要包括：

- **登录流程**：`LoginReq` / `LoginResp` / `LoginSessionMessage`
- **密钥交换**：`GetKeyPairReq` / `GetKeyPairResp` / `UpdateKeyPairReq`
- **群组信息**：`GroupBase` / `GroupContactListReq` / `GroupContactListResp`
- **消息推送**：`PushGroupMessage` / `ReceiveGroupMessage` / `GroupMessage`

## 构建配置

应用通过环境变量区分测试/生产环境，构建脚本 `scripts/tauri-env.mjs` 负责注入对应的服务端地址与密钥。详见 [environment-build-design.md](docs/environment-build.md)。

## 文档

- [环境配置、运行与打包](docs/environment-build.md)
- [消息保留与清理设计](docs/superpowers/specs/2026-09-05-message-retention-design.md)
- [消息摄入到展示链路](docs/superpowers/specs/2026-09-05-message-ingest-to-display.md)
- [中文注释规范与评审清单](docs/review/chinese-comments-checklist.md)
