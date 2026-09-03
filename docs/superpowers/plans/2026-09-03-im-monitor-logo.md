# IM Monitor Logo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将已确认的 B1“柔和全屏雷达”设计制作成项目正式 SVG、Web favicon 和 Tauri 桌面平台图标资源。

**Architecture:** 以一个不依赖字体或外部资源的 SVG 作为唯一设计源，Web 目录保存内容完全相同的静态副本，Tauri CLI 从主 SVG 生成各平台二进制图标。Rust 集成测试锁定 SVG 的品牌路径、雷达参数、Web 副本一致性、关键输出格式和 Tauri 配置，防止后续资源漂移。

**Tech Stack:** SVG 1.1、Tauri CLI 2.11.4、Rust 集成测试、Vite 6

---

## 文件结构

- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg`：B1 Logo 的唯一设计源。
- 创建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg`：Vite 可直接复制到构建目录的 favicon。
- 创建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs`：验证设计参数、Web 副本、生成资源和 Tauri 配置。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json`：显式列出桌面 bundle 图标。
- 保留并提交 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html` 中现有的 `/icon.svg` favicon 引用。
- 由 Tauri CLI 更新 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/` 下的 PNG、ICO、ICNS、Android 和 iOS 图标资源。

### Task 1：建立并实现 SVG 设计契约

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg`

- [ ] **Step 1：记录修改前状态，确认仅覆盖本功能范围内的既有草稿**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git status --short
git diff -- /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.png /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html
```

Expected: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.png`、`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html` 是 Logo 工作产生的未提交改动；其余改动不得被暂存或覆盖。

- [ ] **Step 2：编写会失败的 SVG 契约测试**

创建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs`：

```rust
//! 验证应用图标的设计源、Web 副本和平台构建产物保持一致。

const SOURCE_SVG: &str = include_str!("../icons/icon.svg");
const WEB_SVG: &str = include_str!("../ui/public/icon.svg");

const RIGHT_FIVE_PATH: &str = "M30.25 37.2003H37.8218C37.882 38.4157 38.2135 39.3501 38.8163 40.0035C39.419 40.6568 40.2477 40.9834 41.3025 40.9834C42.7942 40.9834 43.962 40.4289 44.8058 39.3197C45.6497 38.1954 46.0716 36.6457 46.0716 34.6706C46.0716 33.288 45.7175 32.1865 45.0093 31.366C44.3011 30.5456 43.3593 30.1354 42.184 30.1354C41.4155 30.1354 40.7299 30.3025 40.1272 30.6367C39.5245 30.9558 39.0423 31.4116 38.6806 32.0041L31.7644 31.5028L35.4259 14H54.9995L53.7563 19.8343H40.195L38.9971 25.6685C39.9162 25.1975 40.7902 24.8557 41.6189 24.643C42.4477 24.4151 43.299 24.3011 44.173 24.3011C47.0661 24.3011 49.4468 25.2203 51.3153 27.0587C53.1988 28.8971 54.1406 31.2141 54.1406 34.0097C54.1406 37.9751 52.9803 41.1354 50.6598 43.4903C48.3393 45.8301 45.2202 47 41.3025 47C37.867 47 35.2074 46.1644 33.3239 44.4931C31.4404 42.8066 30.4157 40.3757 30.25 37.2003Z";
const LEFT_FIVE_PATH: &str = "M5 37.2003H12.5718C12.632 38.4157 12.9635 39.3501 13.5663 40.0035C14.169 40.6568 14.9977 40.9834 16.0525 40.9834C17.5442 40.9834 18.712 40.4289 19.5558 39.3197C20.3997 38.1954 20.8216 36.6457 20.8216 34.6706C20.8216 33.288 20.4675 32.1865 19.7593 31.366C19.0511 30.5456 18.1093 30.1354 16.934 30.1354C16.1655 30.1354 15.4799 30.3025 14.8772 30.6367C14.2745 30.9558 13.7923 31.4116 13.4306 32.0041L6.51435 31.5028L10.1759 14H29.7495L28.5063 19.8343H14.945L13.7471 25.6685C14.6662 25.1975 15.5402 24.8557 16.3689 24.643C17.1977 24.4151 18.049 24.3011 18.923 24.3011C21.8161 24.3011 24.1968 25.2203 26.0653 27.0587C27.9488 28.8971 28.8906 31.2141 28.8906 34.0097C28.8906 37.9751 27.7303 41.1354 25.4098 43.4903C23.0893 45.8301 19.9702 47 16.0525 47C12.617 47 9.95743 46.1644 8.07391 44.4931C6.19038 42.8066 5.16575 40.3757 5 37.2003Z";

#[test]
fn source_svg_matches_the_approved_b1_design() {
    assert!(SOURCE_SVG.contains(r#"viewBox="0 0 61 61""#));
    assert!(SOURCE_SVG.contains(r#"<rect width="61" height="61" rx="15" fill="#178AFF"/>"#));
    assert!(SOURCE_SVG.contains(
        r#"<path d="M30.5 30.5L55 13A30 30 0 0 1 58 39Z" fill="#71F0D0" opacity="0.22"/>"#
    ));
    assert!(SOURCE_SVG.contains(
        r#"<circle cx="30.5" cy="30.5" r="25" fill="none" stroke="#A7FFEA" stroke-width="1.3" opacity="0.55"/>"#
    ));
    assert!(SOURCE_SVG.contains(
        r#"<circle cx="30.5" cy="30.5" r="17" fill="none" stroke="#A7FFEA" stroke-width="1.1" opacity="0.45"/>"#
    ));
    assert!(SOURCE_SVG.contains(
        r#"<circle cx="30.5" cy="30.5" r="9" fill="none" stroke="#A7FFEA" stroke-width="1" opacity="0.4"/>"#
    ));
    assert!(SOURCE_SVG.contains(
        r#"<path d="M30.5 30.5L53 14" stroke="#8CFFE2" stroke-width="1.5" stroke-linecap="round"/>"#
    ));
    assert!(SOURCE_SVG.contains(
        r#"<circle cx="47" cy="21" r="2" fill="#8CFFE2"/>"#
    ));
    let right_five = format!(
        r#"<path opacity="0.4" d="{RIGHT_FIVE_PATH}" fill="white"/>"#
    );
    let left_five = format!(r#"<path d="{LEFT_FIVE_PATH}" fill="white"/>"#);
    assert!(SOURCE_SVG.contains(right_five.as_str()));
    assert!(SOURCE_SVG.contains(left_five.as_str()));
}

#[test]
fn web_favicon_is_identical_to_the_design_source() {
    assert_eq!(WEB_SVG, SOURCE_SVG);
}
```

- [ ] **Step 3：运行测试并确认先失败**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app --test icon_assets
```

Expected: FAIL，编译器报告无法读取 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg`，证明测试会阻止 favicon 缺失。

- [ ] **Step 4：将 B1 正式 SVG 写入主源和 Web 副本**

将以下相同内容分别写入 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg`：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<svg width="61" height="61" viewBox="0 0 61 61" fill="none" xmlns="http://www.w3.org/2000/svg">
  <rect width="61" height="61" rx="15" fill="#178AFF"/>
  <path d="M30.5 30.5L55 13A30 30 0 0 1 58 39Z" fill="#71F0D0" opacity="0.22"/>
  <circle cx="30.5" cy="30.5" r="25" fill="none" stroke="#A7FFEA" stroke-width="1.3" opacity="0.55"/>
  <circle cx="30.5" cy="30.5" r="17" fill="none" stroke="#A7FFEA" stroke-width="1.1" opacity="0.45"/>
  <circle cx="30.5" cy="30.5" r="9" fill="none" stroke="#A7FFEA" stroke-width="1" opacity="0.4"/>
  <path d="M30.5 30.5L53 14" stroke="#8CFFE2" stroke-width="1.5" stroke-linecap="round"/>
  <circle cx="47" cy="21" r="2" fill="#8CFFE2"/>
  <path opacity="0.4" d="M30.25 37.2003H37.8218C37.882 38.4157 38.2135 39.3501 38.8163 40.0035C39.419 40.6568 40.2477 40.9834 41.3025 40.9834C42.7942 40.9834 43.962 40.4289 44.8058 39.3197C45.6497 38.1954 46.0716 36.6457 46.0716 34.6706C46.0716 33.288 45.7175 32.1865 45.0093 31.366C44.3011 30.5456 43.3593 30.1354 42.184 30.1354C41.4155 30.1354 40.7299 30.3025 40.1272 30.6367C39.5245 30.9558 39.0423 31.4116 38.6806 32.0041L31.7644 31.5028L35.4259 14H54.9995L53.7563 19.8343H40.195L38.9971 25.6685C39.9162 25.1975 40.7902 24.8557 41.6189 24.643C42.4477 24.4151 43.299 24.3011 44.173 24.3011C47.0661 24.3011 49.4468 25.2203 51.3153 27.0587C53.1988 28.8971 54.1406 31.2141 54.1406 34.0097C54.1406 37.9751 52.9803 41.1354 50.6598 43.4903C48.3393 45.8301 45.2202 47 41.3025 47C37.867 47 35.2074 46.1644 33.3239 44.4931C31.4404 42.8066 30.4157 40.3757 30.25 37.2003Z" fill="white"/>
  <path d="M5 37.2003H12.5718C12.632 38.4157 12.9635 39.3501 13.5663 40.0035C14.169 40.6568 14.9977 40.9834 16.0525 40.9834C17.5442 40.9834 18.712 40.4289 19.5558 39.3197C20.3997 38.1954 20.8216 36.6457 20.8216 34.6706C20.8216 33.288 20.4675 32.1865 19.7593 31.366C19.0511 30.5456 18.1093 30.1354 16.934 30.1354C16.1655 30.1354 15.4799 30.3025 14.8772 30.6367C14.2745 30.9558 13.7923 31.4116 13.4306 32.0041L6.51435 31.5028L10.1759 14H29.7495L28.5063 19.8343H14.945L13.7471 25.6685C14.6662 25.1975 15.5402 24.8557 16.3689 24.643C17.1977 24.4151 18.049 24.3011 18.923 24.3011C21.8161 24.3011 24.1968 25.2203 26.0653 27.0587C27.9488 28.8971 28.8906 31.2141 28.8906 34.0097C28.8906 37.9751 27.7303 41.1354 25.4098 43.4903C23.0893 45.8301 19.9702 47 16.0525 47C12.617 47 9.95743 46.1644 8.07391 44.4931C6.19038 42.8066 5.16575 40.3757 5 37.2003Z" fill="white"/>
</svg>
```

- [ ] **Step 5：运行测试并确认通过**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app --test icon_assets
```

Expected: 2 tests passed。

- [ ] **Step 6：提交设计源、Web 副本和契约测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs
git commit -m "feat: add radar monitor logo"
```

### Task 2：生成并配置 Tauri 平台图标

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json`
- Generate: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/`

- [ ] **Step 1：扩展测试，锁定桌面输出格式、尺寸和 bundle 配置**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs` 末尾加入：

```rust
const PNG_32: &[u8] = include_bytes!("../icons/32x32.png");
const PNG_128: &[u8] = include_bytes!("../icons/128x128.png");
const PNG_256: &[u8] = include_bytes!("../icons/128x128@2x.png");
const ICNS: &[u8] = include_bytes!("../icons/icon.icns");
const ICO: &[u8] = include_bytes!("../icons/icon.ico");
const TAURI_CONFIG: &str = include_str!("../tauri.conf.json");

/// 从 PNG 的 IHDR 固定位置读取宽高；调用者必须先保证输入至少包含完整 IHDR。
fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() >= 24);
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[test]
fn generated_desktop_icons_have_expected_formats_and_sizes() {
    assert_eq!(png_dimensions(PNG_32), (32, 32));
    assert_eq!(png_dimensions(PNG_128), (128, 128));
    assert_eq!(png_dimensions(PNG_256), (256, 256));
    assert_eq!(&ICNS[..4], b"icns");
    assert_eq!(&ICO[..4], &[0, 0, 1, 0]);
}

#[test]
fn tauri_bundle_references_all_desktop_icons() {
    let config: serde_json::Value = serde_json::from_str(TAURI_CONFIG).unwrap();
    assert_eq!(
        config["bundle"]["icon"],
        serde_json::json!([
            "icons/32x32.png",
            "icons/128x128.png",
            "icons/128x128@2x.png",
            "icons/icon.icns",
            "icons/icon.ico"
        ])
    );
}
```

- [ ] **Step 2：运行测试并确认平台资源尚未满足契约**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app --test icon_assets
```

Expected: FAIL；缺少生成文件时为 `include_bytes!` 读取失败，已有旧文件时则因尺寸、格式或空的 `bundle.icon` 不符合预期而失败。

- [ ] **Step 3：使用已安装的 Tauri CLI 2.11.4 生成全平台资源**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app
cargo tauri icon /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg --output /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons
```

Expected: 命令成功，并生成桌面 PNG、`icon.ico`、`icon.icns` 以及 Tauri 默认的 Android/iOS 图标集。

- [ ] **Step 4：显式配置桌面 bundle 图标**

将 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json` 的 `bundle.icon` 从空数组改为：

```json
"icon": [
  "icons/32x32.png",
  "icons/128x128.png",
  "icons/128x128@2x.png",
  "icons/icon.icns",
  "icons/icon.ico"
]
```

这些值按 Tauri 配置约定相对于 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json` 解析。

- [ ] **Step 5：运行资源契约测试**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app --test icon_assets
```

Expected: 4 tests passed。

- [ ] **Step 6：提交生成资源和 Tauri 配置**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tests/icon_assets.rs
git commit -m "build: generate application icons"
```

### Task 3：接入并验证 Web favicon

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html`
- Verify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg`

- [ ] **Step 1：确认入口只包含一个正确的 favicon 引用**

`/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html` 的 `<head>` 保持：

```html
<title>IM 实时监控控制台</title>
<link rel="icon" type="image/svg+xml" href="/icon.svg" />
```

Run:

```bash
rg -n 'rel="icon".*href="/icon.svg"' /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html
```

Expected: 恰好 1 个匹配。

- [ ] **Step 2：构建前端并验证 Vite 原样复制 favicon**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm run build
cmp /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/public/icon.svg /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/dist/icon.svg
```

Expected: `npm run build` 成功，`cmp` 退出码为 0。

- [ ] **Step 3：提交 favicon 入口**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/index.html
git commit -m "feat: use monitor logo as favicon"
```

### Task 4：完成视觉与构建验收

**Files:**
- Verify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/`
- Verify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/tauri.conf.json`

- [ ] **Step 1：从主 SVG 生成六档临时验收图并检查真实尺寸**

Run:

```bash
PREVIEW_DIR="$(mktemp -d /tmp/im-monitor-logo-review.XXXXXX)"
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app
cargo tauri icon /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/icons/icon.svg --output "$PREVIEW_DIR" --png 16,32,64,128,256,1024
sips -g pixelWidth -g pixelHeight "$PREVIEW_DIR/16x16.png" "$PREVIEW_DIR/32x32.png" "$PREVIEW_DIR/64x64.png" "$PREVIEW_DIR/128x128.png" "$PREVIEW_DIR/256x256.png" "$PREVIEW_DIR/1024x1024.png"
```

Expected: 六个文件依次为 `16 × 16`、`32 × 32`、`64 × 64`、`128 × 128`、`256 × 256` 和 `1024 × 1024`。

- [ ] **Step 2：逐个打开小、中、大尺寸图标进行视觉复核**

打开并检查当前 `$PREVIEW_DIR` 中的：

- `16x16.png`
- `32x32.png`
- `64x64.png`
- `128x128.png`
- `256x256.png`
- `1024x1024.png`

Expected: 所有尺寸均无裁切；先识别到“55”，随后识别到右上扫描线、目标点和柔和同心圆；背景为 `#178AFF`；右侧“5”为 40% 白色，左侧“5”为纯白。

- [ ] **Step 3：执行针对性测试和工作区构建**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app --test icon_assets
cargo check -p im-app
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm test
npm run build
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app
cargo tauri build --no-bundle
```

Expected: 所有命令退出码为 0；图标测试 4 项通过；Vite 输出包含 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/dist/icon.svg`；Tauri 无 bundle 构建成功。

- [ ] **Step 4：确认没有夹带无关改动**

Run:

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git status --short
git diff --check
git log --oneline -4
```

Expected: Logo 实施相关文件均已提交；`.history/`、`.superpowers/` 等既有无关未跟踪文件没有进入提交；`git diff --check` 无错误。
