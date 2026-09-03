# 双环境配置与跨平台构建实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将服务地址、协议版本和客户端密钥改为测试/生产构建配置，并提供本地命令与 GitHub 三平台安装包构建。

**Architecture:** Rust 通过编译期环境快照构造严格校验的 `AppConfig`，Vue 通过同一构建进程取得 Vite 变量。跨平台 Node 脚本负责加载环境文件、执行无泄漏校验并启动 Tauri；GitHub Actions 使用同名 Environment 注入变量和密钥。

**Tech Stack:** Rust 2021、Tauri 2、Vue 3、Vite 6、Node.js、GitHub Actions。

---

### Task 1: Rust 构建配置

**Files:**
- Modify: `im-common/src/config.rs`
- Modify: `im-common/src/tests.rs`
- Modify: `im-app/src/main.rs`

- [x] 编写变量缺失、非法整数、URL、Host、AES key 和成功构造测试。
- [x] 实现 `AppConfig::from_build_env()`，错误不包含变量值。
- [x] 将 `Default` 改为离线测试配置，桌面入口改用严格配置。
- [x] 运行 `cargo test -p im-common`。

### Task 2: TCP 单密钥解码

**Files:**
- Modify: `im-chat/src/frame.rs`
- Modify: `im-chat/src/client.rs`
- Modify: `im-chat/src/tests.rs`

- [x] 删除会话前固定 key、回退函数和旧测试。
- [x] 所有服务端帧统一使用 `decode_transport_frame`。
- [x] 增加 `9999` 不回退其他 key 的测试。
- [x] 运行 `cargo test -p im-chat`。

### Task 3: GT4 严格配置

**Files:**
- Modify: `im-app/ui/src/composables/useGt4.ts`
- Modify: `im-app/ui/src/composables/useGt4.test.ts`
- Delete: `im-app/ui/.env.example`

- [x] 增加缺失 Captcha ID 时不加载 SDK 的测试。
- [x] 删除源码真实 ID，严格读取注入参数或 Vite 环境变量。
- [x] 运行 GT4 单元测试。

### Task 4: 环境模板与本地脚本

**Files:**
- Create: `config/.env.test.example`
- Create: `config/.env.production.example`
- Create: `scripts/tauri-env.mjs`
- Create: `scripts/tauri-env.test.mjs`
- Modify: `.gitignore`
- Modify: `im-app/package.json`
- Create: `im-app/package-lock.json`

- [x] 测试命令/环境白名单、dotenv 解析、覆盖顺序、校验和脱敏。
- [x] 实现跨平台配置加载与 Tauri CLI 启动。
- [x] 增加模板、忽略规则、四个 npm 命令和锁定 CLI。
- [x] 运行 `npm --prefix im-app run test:scripts`。

### Task 5: GitHub 三平台打包

**Files:**
- Modify: `im-app/tauri.conf.json`
- Create: `.github/workflows/build.yml`

- [x] 删除 Tauri 重复版本。
- [x] 创建 test/production 手动构建入口。
- [x] 增加 Windows x86_64、macOS 双架构和 Linux x86_64 矩阵。
- [x] 从 GitHub Environment Variables/Secrets 注入配置。
- [x] 上传各平台 bundles，不创建 Release。

### Task 6: 文档与验证

**Files:**
- Create: `docs/environment-build.md`
- Create: `docs/superpowers/specs/2026-09-03-environment-build-design.md`

- [x] 编写本地配置、命令、GitHub Environment 和未签名说明。
- [x] 运行 Rust 格式化、Clippy 和 workspace 测试。
- [x] 运行 Vue 测试、类型检查和 Node 脚本测试。
- [x] 执行本机测试环境构建检查。
