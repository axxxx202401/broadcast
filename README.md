# IM Monitor（IM 群消息监控客户端）

一个基于 Rust + Tauri + Vue 3 的桌面应用程序，用于实时接收和展示聊天群组的推送消息。

## 功能概述

- **多账号支持**：支持手机号/邮箱登录、短信验证码、图形验证（Geetest v4）等多因素认证流程
- **群组监控**：查看已关注的群组列表，一键开启/关闭消息监控
- **实时消息**：通过 TCP 长连接实时接收群消息，持久化至本地 SQLite 数据库
- **富媒体支持**：支持文本、图片、音频、视频、文件等多种消息类型的解析与展示
- **离线存储**：所有消息与群组数据保存在本地，退出后仍可查阅历史记录

## 技术栈

| 层次 | 技术 |
|------|------|
| 后端运行时 | Rust 1.75+ / Tokio |
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
├── im-http/       # HTTP 客户端：OpenChat 用户接口、IM 业务接口、私有帧协议
├── im-store/      # SQLite 持久化层：消息、群组、密钥对存储
├── im-app/        # Tauri 桌面应用主入口（Rust 后端 + Vue 前端）
│   ├── src/       # Rust 命令与状态管理
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

- Rust 1.75 或更高版本
- Node.js 18+ 与 pnpm / npm
- macOS：`cargo install --locked cargo-apk`（如需构建 macOS 安装包）
- macOS：系统需安装 [Tauri 所需依赖](https://v2.tauri.app/start/prerequisites/#macos)

### 安装依赖

```bash
# 前端依赖
cd im-app/ui
npm install
cd ../..

# Rust 依赖（在工作区根目录）
cargo build
```

### 开发模式

```bash
# 启动 Tauri 开发服务器（前端 + 后端同时运行）
cd im-app
npm run dev
# 或指定环境
npm run dev:test       # 连接测试服务器
npm run dev:production # 连接生产服务器
```

### 构建

```bash
# 仅构建前端
cd im-app/ui && npm run build

# 构建 Tauri 桌面应用
cd im-app
npm run build:test          # 测试环境构建
npm run build:production    # 生产环境构建

# 构建并直接运行
npm run build-run:production
```

### 脚本测试

```bash
cd im-app
npm run test:scripts   # 运行 tauri-env.mjs 单元测试
```

## 使用方式

1. **登录**：启动应用后，在登录面板输入手机号/邮箱及验证码，完成 Geetest 图形验证
2. **选择群组**：登录后进入主界面，左侧面板显示已关注的群组列表，可搜索并添加新群组
3. **开启监控**：点击群组旁的开关按钮，应用将建立 TCP 长连接并开始接收实时消息
4. **查看消息**：右侧消息面板按时间顺序展示消息内容，支持图片预览、文件下载等操作
5. **断开连接**：关闭监控或切换账号时，应用会自动清理 TCP 连接并保存状态

## 安全说明

- 消息正文采用 AES-256-CBC 加密传输，密钥通过 X25519 密钥交换协商
- 认证令牌与 App 密钥对存储在本地 SQLite，不落盘明文缓存
- CSP 策略限制外部资源加载，仅允许必要的验证服务域名

## 协议说明

`proto/broadcast.proto` 定义了客户端与服务端通信的 Protobuf 消息格式，主要包括：

- **登录流程**：`LoginReq` / `LoginResp` / `LoginSessionMessage`
- **密钥交换**：`GetKeyPairReq` / `GetKeyPairResp` / `UpdateKeyPairReq`
- **群组信息**：`GroupBase` / `GroupContactListReq` / `GroupContactListResp`
- **消息推送**：`PushGroupMessage` / `ReceiveGroupMessage` / `GroupMessage`

## 构建配置

应用通过环境变量区分测试/生产环境，构建脚本 `scripts/tauri-env.mjs` 负责注入对应的服务端地址与密钥。详见 [environment-build-design.md](docs/environment-build.md)。
