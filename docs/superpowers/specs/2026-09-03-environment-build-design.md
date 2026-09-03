# 双环境配置与跨平台构建设计

## 目标

将客户端硬编码的服务地址、协议版本和传输密钥迁移为构建时环境变量，支持 `test` 与
`production` 两种安装包；提供统一的本地运行、打包入口，并通过 GitHub Actions 生成
Windows、macOS 和 Linux 安装包。

工作流只生成 Actions Artifacts，不创建 Release，也不配置平台签名。

## 安全边界

桌面客户端必须持有协议密钥。环境变量和 GitHub Secrets 可以避免密钥继续写入仓库，
但密钥最终仍进入可执行文件，无法防止安装包使用者提取。

构建脚本不得输出密钥值。变量缺失或不满足约束时必须列出变量名并终止，不能静默使用
测试环境或历史默认值。

## 配置模型

Rust 在编译时读取：

- `IM_OPENCHAT_USER_URL`
- `IM_BIZ_URL`
- `IM_CHAT_HOST`
- `IM_CHAT_PORT`
- `IM_VERSION_SECRET_NAME`
- `IM_BODY_AES_KEY`
- `IM_HEADER_AES_KEY`
- `IM_APP_VER`
- `IM_PACKAGE_CODE`
- `IM_PLAT`
- `IM_LANGUAGE`
- `IM_SYS_MODEL`

`sysMac` 继续在构造设备配置时随机生成。URL、端口、整数和 16 字节 AES-128 key 在应用
初始化前校验。`AppConfig::default()` 只提供不访问远端服务的测试占位配置；桌面入口使用
严格的构建配置。

TCP 不再保留会话前固定 AES key 或 `9999` 回退解密。所有加密服务端帧均使用
`IM_BODY_AES_KEY`，失败时终止当前连接。

Vue/Vite 在构建时读取 `VITE_GT4_CAPTCHA_ID`。源码不保留真实 ID 回退值，缺失时 GT4
初始化返回明确配置错误。

安装包版本以 workspace `Cargo.toml` 的 `workspace.package.version` 为唯一来源。
`IM_APP_VER` 和 `IM_PACKAGE_CODE` 是远端协议版本。Rust 工具链、依赖版本及密码摘要固定盐
不按部署环境切换。

## 环境文件与命令

仓库提交：

- `config/.env.test.example`
- `config/.env.production.example`

本地实际使用 `.env.test` 与 `.env.production`，两者均被 Git 忽略。测试模板包含非敏感
测试地址和协议编号；敏感值留空。生产模板由部署者填写。

跨平台 Node 执行器只接受 `dev|build` 和 `test|production`，环境文件值可被调用进程的
同名变量覆盖。它先校验配置，再执行项目锁定版本的 Tauri CLI，并透传退出状态。

公开命令：

- `npm run dev:test`
- `npm run dev:production`
- `npm run build:test`
- `npm run build:production`

## GitHub Actions

工作流通过 `workflow_dispatch` 选择 `test|production`，Job 绑定同名 GitHub
Environment。普通地址、端口和协议编号使用 Environment Variables；两个 AES key 与
Version Secret Name 使用 Environment Secrets。

矩阵包含：

- Windows x86_64
- macOS x86_64
- macOS arm64
- Linux x86_64

每个 Job 安装对应系统依赖、Node、Rust target、根应用依赖和 Vue 依赖，然后调用统一
打包脚本并上传带环境、平台和架构名称的 Artifact。Artifacts 保留 14 天。

## 验证

自动测试覆盖 Rust 缺失/非法构建配置、AES 长度、GT4 缺失配置、Node 环境文件解析、
进程变量覆盖、命令和环境白名单，以及错误不泄漏密钥值。

实施后执行 Rust 格式化、Clippy、workspace 测试、Vue 测试和类型检查、Node 脚本测试及
本机测试环境构建。三种操作系统的最终 bundle 由首次手动 GitHub workflow 验证。

## 非目标

- 不新增自动更新。
- 不创建 GitHub Release。
- 不配置 Apple notarization、macOS 签名或 Windows 代码签名。
- 不提交生产配置。
- 不把客户端密钥描述为不可提取的秘密。
