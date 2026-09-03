# 环境配置、运行与打包

## 配置边界

IM Monitor 在构建时读取服务地址、协议版本、传输密钥和 GT4 Captcha ID。生成安装包后，
再修改启动进程的环境变量不会改变安装包中的配置。

环境变量只能避免密钥继续出现在 Git 仓库中。桌面客户端运行协议时必须持有密钥，因此
不能防止安装包使用者提取密钥。

应用安装包版本只修改 workspace 根目录 `Cargo.toml` 的
`workspace.package.version`。协议版本使用 `IM_APP_VER` 和 `IM_PACKAGE_CODE`，两者不是
同一种版本。

## 本地初始化

测试环境：

```bash
cp config/.env.test.example config/.env.test
```

生产环境：

```bash
cp config/.env.production.example config/.env.production
```

实际 `.env.test` 和 `.env.production` 已被 Git 忽略。必须填写模板中的空值，尤其是：

- `IM_VERSION_SECRET_NAME`
- `IM_BODY_AES_KEY`
- `IM_HEADER_AES_KEY`

两个 AES key 必须恰好为 16 字节。脚本不会输出配置值；缺少或无效时只报告变量名和约束。

安装构建依赖：

```bash
cd im-app
npm ci
npm --prefix ui ci
```

## 本地运行

在 `im-app` 目录执行：

```bash
npm run dev:test
npm run dev:production
```

环境文件中的值可以被当前进程的同名环境变量覆盖。该能力主要用于 CI 和临时联调，不应
通过命令历史传递真实密钥。

## 本地打包

在 `im-app` 目录执行：

```bash
npm run build:test
npm run build:production
```

产物位于 workspace 的 `target` 目录。具体安装包格式由当前操作系统和 Tauri bundle
支持决定；一个操作系统不能直接生成所有其他系统的安装包。

## GitHub Environments

在 GitHub 仓库 Settings → Environments 中分别创建：

- `test`
- `production`

每个 Environment 配置以下 Variables：

- `IM_OPENCHAT_USER_URL`
- `IM_BIZ_URL`
- `IM_CHAT_HOST`
- `IM_CHAT_PORT`
- `IM_APP_VER`
- `IM_PACKAGE_CODE`
- `IM_PLAT`
- `IM_LANGUAGE`
- `IM_SYS_MODEL`
- `VITE_GT4_CAPTCHA_ID`

每个 Environment 配置以下 Secrets：

- `IM_VERSION_SECRET_NAME`
- `IM_BODY_AES_KEY`
- `IM_HEADER_AES_KEY`

生产环境可启用 required reviewers，避免未审核的生产安装包构建。

## GitHub 打包

打开 Actions → Build desktop bundles → Run workflow，选择 `test` 或 `production`。
工作流分别生成：

- Windows x86_64
- macOS Intel x86_64
- macOS Apple Silicon arm64
- Linux x86_64

完成后在该 workflow run 的 Artifacts 区域下载。Artifacts 保留 14 天，不会自动创建
GitHub Release。

当前工作流未配置 Apple notarization、macOS 签名或 Windows 代码签名。操作系统可能显示
“未知开发者”或类似安全警告；正式外部分发前应另外配置平台签名。

## 故障排查

- `缺少必需构建变量`：检查所选环境文件或 GitHub Environment 是否完整。
- `必须恰好为 16 字节`：检查 AES-128 key 的 UTF-8 字节长度。
- `VITE_GT4_CAPTCHA_ID`：该变量必须在启动 Vite/Tauri 构建前提供。
- GitHub 提示找不到 bundle：查看对应矩阵 Job 的 Tauri 构建日志；上传步骤不会把空目录
  误报为成功。
- `macOS 已损坏/无法打开`：通常是下载后触发的 Gatekeeper 隔离属性（quarantine）导致。当前
  CI macOS 构建启用了 ad-hoc 重签以降低被拦概率；若仍被拦，可先清除隔离后重试：
  `xattr -cr "/Applications/IM Monitor.app"`。
