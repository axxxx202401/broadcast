# IM 群消息监控客户端 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭建 Tauri v2 + Rust workspace 的核心骨架（Phase 1）：加解密库、proto binding、TCP 连接、存储、UI 框架。

**Architecture:** Cargo workspace 多 crate，im-common 提供 AES/TCP head/X-One，im-proto 提供 prost 编译的 protobuf binding，im-chat 提供 TCP 长连接客户端，im-store 提供 SQLite 持久化，im-app 为 Tauri v2 桌面应用入口。前端用 vanilla HTML/CSS/JS 展示群列表和消息流。

**Tech Stack:**
- Rust 2021 edition, tokio (async runtime)
- `prost` + `prost-build` (protobuf), `aes` + `cipher` + `pkcs7` (AES/ECB/PKCS7Padding)
- `reqwest` (HTTP), `sqlx` + `sqlx-lite` (SQLite)
- `tokio-util` (compat), `flate2` (gzip), `hex`, `md-5`
- Tauri v2, Rust backend + vanilla frontend

**Spec:** `docs/superpowers/specs/2026-09-02-im-monitor-client-design.md`

## Global Constraints

- `body-aes-key = "97b1f52761ffc7f8"` — 16 字符 UTF-8 字符串，直接作为 AES-128 key bytes（NOT hex-decoded）
- `header-aes-key = "f58c15f54e8f7826"` — 同上
- `secretName = "f82956caf0fa90aecf24d5ef9541f624"` — 版本密钥名
- TCP head: `[0xC0, 0x80]` = 加密未压缩, `[0xC0, 0xC0]` = 加密已压缩
- TCP wire: `[2B head][2B messageId(big-endian)][4B contentLength(big-endian)][content]`
- X-One header: `hex(AES_ECBAES_V_L_SALT(secretName+","+timestamp_ms))`，其中 V_L_SALT = `md5("sjlkajsl*Rkfsdsd_tflklsjdf").first16bytes`
- openchat-user 登录流程: `sendSmsCaptchaWithGt4` → `issued` → `verify` → `login`
- 极验参数: captchaId=`0fd8f86d495fa3b8e944c07143e49ced`, captchaKey=`4784ce2e73fa19f7be82ed3cf60d3658`
- im-chat 地址: `35.220.159.225:9500`, openchat-user: `https://test-ochat-user1.68chat.co`, im-biz: `https://test-biz-b.68chat.co`
- proto 源文件: `/Volumes/TRANSCEND/works/objects/java/proto/`

---

## 文件清单

### 新建文件

| 文件 | 职责 |
|------|------|
| `Cargo.toml` | workspace 根 manifest |
| `im-common/Cargo.toml` | 核心库: AES, TCP head, X-One, 配置 |
| `im-common/src/lib.rs` | 模块声明 |
| `im-common/src/aes.rs` | AES/ECB/PKCS7Padding 加解密 |
| `im-common/src/tcp_head.rs` | TCP 帧头解析/构建 |
| `im-common/src/version_key.rs` | X-One 头生成 |
| `im-common/src/config.rs` | 配置结构体 |
| `im-common/src/error.rs` | 错误类型 |
| `im-common/src/tests.rs` | 单元测试 |
| `im-proto/Cargo.toml` | protobuf binding |
| `im-proto/build.rs` | prost-build 编译脚本 |
| `im-proto/src/lib.rs` | 重新导出模块 |
| `proto/common.proto` | 复制自 Java 源码 |
| `proto/im.proto` | 复制自 Java 源码 |
| `proto/group.proto` | 复制自 Java 源码 |
| `proto/login.proto` | 复制自 Java 源码 |
| `proto/friend_message.proto` | 最小化依赖（仅需要的 message） |
| `proto/group_message.proto` | 最小化依赖 |
| `proto/channel_event.proto` | 最小化依赖 |
| `im-chat/Cargo.toml` | TCP 客户端库 |
| `im-chat/src/lib.rs` | 模块声明 |
| `im-chat/src/client.rs` | ChatClient 主结构 |
| `im-chat/src/frame.rs` | TCP 帧编码/解码 |
| `im-chat/src/heartbeat.rs` | 心跳任务 |
| `im-chat/src/reconnect.rs` | 指数退避重连 |
| `im-chat/src/tests.rs` | 帧编码测试 |
| `im-store/Cargo.toml` | SQLite 存储 |
| `im-store/src/lib.rs` | 模块声明 |
| `im-store/src/schema.rs` | 建表 SQL |
| `im-store/src/message.rs` | 消息 CRUD |
| `im-store/src/group.rs` | 群信息 CRUD |
| `im-app/Cargo.toml` | Tauri v2 应用 |
| `im-app/src/main.rs` | Tauri 入口 |
| `im-app/src/state.rs` | 全局状态管理 |
| `im-app/src/commands/auth.rs` | 登录命令 |
| `im-app/src/commands/groups.rs` | 群管理命令 |
| `im-app/src/commands/chat.rs` | 连接控制命令 |
| `im-app/src/monitor.rs` | 后台监控任务 |
| `im-app/src-tauri/tauri.conf.json` | Tauri 配置 |
| `im-app/src-tauri/capabilities/default.json` | Tauri 能力 |
| `im-app/src-tauri/build.rs` | Tauri 构建脚本 |
| `im-app/src-tauri/info.plist` | macOS 元信息（如有需要） |
| `im-app/src-tauri/gen/schemas/*.json` | Tauri 自动生成 |
| `im-app/src-tauri/bundle.json` | 打包配置（可选） |
| `im-app/src-tauri/assets/` | 前端静态文件 |
| `im-app/src-tauri/assets/index.html` | 主页面 |
| `im-app/src-tauri/assets/app.js` | 前端逻辑 |
| `im-app/src-tauri/assets/style.css` | 样式 |

---

## Task 1: Workspace 骨架

**Files:**
- Create: `Cargo.toml`
- Create: `im-common/Cargo.toml`
- Create: `im-proto/Cargo.toml`
- Create: `im-chat/Cargo.toml`
- Create: `im-store/Cargo.toml`
- Create: `im-app/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces: workspace root, 6 个 crate 的 Cargo.toml

- [ ] **Step 1: 创建 workspace Cargo.toml**

```toml
# /Volumes/TRANSCEND/works/objects/rust/broadcast/Cargo.toml
[workspace]
members = [
    "im-common",
    "im-proto",
    "im-chat",
    "im-store",
    "im-app",
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[workspace.dependencies]
# 异步运行时
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["compat"] }
futures = "0.3"

# HTTP
reqwest = { version = "0.12", default-features = false, features = ["json", "gzip", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# 加密
aes = "0.8"
cipher = "0.4"
pkcs7 = "0.2"
md-5 = "0.10"
hex = "0.4"

# 压缩
flate2 = "1"

# Protobuf
prost = "0.13"
prost-build = "0.13"

# SQLite
sqlx = { version = "0.8", default-features = false, features = ["sqlite", "runtime-tokio-rustls"] }

# 工具
chrono = { version = "0.4", default-features = false, features = ["clock"] }
thiserror = "1"
log = "0.4"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Tauri
tauri = { version = "2", features = [] }
tauri-plugin-store = "2"

[profile.release]
opt-level = 2
strip = true
```

- [ ] **Step 2: 创建 im-common Cargo.toml**

```toml
# im-common/Cargo.toml
[package]
name = "im-common"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
aes.workspace = true
cipher.workspace = true
pkcs7.workspace = true
md-5.workspace = true
hex.workspace = true
serde = { workspace = true, features = ["derive"] }
chrono = { workspace = true, features = ["serde"] }
thiserror.workspace = true
log.workspace = true
```

- [ ] **Step 3: 创建 im-proto Cargo.toml**

```toml
# im-proto/Cargo.toml
[package]
name = "im-proto"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
prost = { workspace = true, features = ["derive"] }

[build-dependencies]
prost-build = { workspace = true, features = [] }
```

- [ ] **Step 4: 创建 im-chat Cargo.toml**

```toml
# im-chat/Cargo.toml
[package]
name = "im-chat"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
im-common = { path = "../im-common" }
im-proto = { path = "../im-proto" }
tokio = { workspace = true }
tokio-util = { workspace = true }
prost = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror.workspace = true
log.workspace = true
tracing.workspace = true
flate2.workspace = true
bytes = "1"
```

- [ ] **Step 5: 创建 im-store Cargo.toml**

```toml
# im-store/Cargo.toml
[package]
name = "im-store"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
im-proto = { path = "../im-proto" }
im-common = { path = "../im-common" }
sqlx = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
tracing.workspace = true
chrono = { workspace = true, features = ["serde"] }
```

- [ ] **Step 6: 创建 im-app Cargo.toml**

```toml
# im-app/Cargo.toml
[package]
name = "im-app"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
im-common = { path = "../im-common" }
im-proto = { path = "../im-proto" }
im-chat = { path = "../im-chat" }
im-store = { path = "../im-store" }
im-http = { path = "../im-http" }
tauri = { workspace = true }
tokio = { workspace = true, features = ["sync"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
sqlx = { workspace = true }
tracing.workspace = true
tracing-subscriber = { workspace = true, features = ["env-filter"] }
chrono = { workspace = true, features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
async-trait = "0.1"

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

- [ ] **Step 7: 验证 workspace 可编译**

Run: `cd /Volumes/TRANSCEND/works/objects/rust/broadcast && cargo build -p im-common 2>&1 | tail -5`
Expected: compilation errors about missing source files (expected — we haven't created them yet)

- [ ] **Step 8: Commit**

```bash
git init
git add Cargo.toml im-common/Cargo.toml im-proto/Cargo.toml im-chat/Cargo.toml im-store/Cargo.toml im-app/Cargo.toml
git commit -m "feat: initialize workspace with crate manifests"
```

---

## Task 2: im-common — AES 加解密

**Files:**
- Create: `im-common/src/lib.rs`
- Create: `im-common/src/error.rs`
- Create: `im-common/src/aes.rs`
- Create: `im-common/src/tests.rs`

**Interfaces:**
- Consumes: `aes`, `cipher`, `pkcs7`, `thiserror`
- Produces: `AesCipher::encrypt(plaintext: &[u8]) -> Result<Vec<u8>>`, `AesCipher::decrypt(ciphertext: &[u8]) -> Result<Vec<u8>>`

- [ ] **Step 1: 创建 lib.rs 和 error.rs**

```rust
// im-common/src/lib.rs
pub mod aes;
pub mod tcp_head;
pub mod version_key;
pub mod config;
pub mod error;

#[cfg(test)]
mod tests;

pub use error::AppError;
```

```rust
// im-common/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("AES decryption failed: {0}")]
    AesDecrypt(#[from] cipher::errors::DecryptError),
    #[error("AES encryption failed: {0}")]
    AesEncrypt(String),
    #[error("TCP frame malformed: {0}")]
    TcpFrame(String),
    #[error("Proto parse error: {0}")]
    ProtoParse(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Login failed: {0}")]
    Login(String),
}

pub type AppResult<T> = Result<T, AppError>;
```

- [ ] **Step 2: 编写 AES 测试（应失败）**

```rust
// im-common/src/tests.rs
use super::aes::AesCipher;

#[test]
fn test_aes_encrypt_decrypt() {
    let key = b"97b1f52761ffc7f8";
    let cipher = AesCipher::new(key);
    let plaintext = b"hello world";
    let encrypted = cipher.encrypt(plaintext).unwrap();
    let decrypted = cipher.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, b"hello world");
}

#[test]
fn test_aes_pkcs7_padding() {
    let key = b"97b1f52761ffc7f8";
    let cipher = AesCipher::new(key);
    // 1 byte input → should be padded to 16 bytes
    let encrypted = cipher.encrypt(b"x").unwrap();
    assert_eq!(encrypted.len(), 16);
    let decrypted = cipher.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, b"x");
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p im-common`
Expected: FAIL — module not defined yet

- [ ] **Step 4: 实现 AES 模块**

```rust
// im-common/src/aes.rs
use aes::Aes128;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, Padding};
use pkcs7::Pkcs7;
use crate::error::{AppError, AppResult};

pub struct AesCipher {
    encryptor: pkcs7::Encryptor<Aes128, pkcs7::ecb::Ecb>,
    decryptor: pkcs7::Decryptor<Aes128, pkcs7::ecb::Ecb>,
}

impl AesCipher {
    pub fn new(key: &[u8]) -> Self {
        assert_eq!(key.len(), 16, "AES-128 key must be 16 bytes");
        let key: [u8; 16] = key.try_into().unwrap();
        Self {
            encryptor: pkcs7::Encryptor::with_padding(key.into(), pkcs7::ecb::Ecb, Pkcs7),
            decryptor: pkcs7::Decryptor::with_padding(key.into(), pkcs7::ecb::Ecb, Pkcs7),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        self.encryptor
            .encrypt_vec(plaintext)
            .map_err(|e| AppError::AesEncrypt(e.to_string()))
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> AppResult<Vec<u8>> {
        self.decryptor
            .decrypt_padded_vec::<Pkcs7>(ciphertext)
            .map_err(|e| AppError::AesDecrypt(e.into()))
    }
}
```

> **注意**: 使用 `pkcs7` crate 的简化 API。如果编译有问题，改用 `aes` + `cipher` + `cbc` 的方式：

备选方案（如果 pkcs7 crate 不合适）:
```rust
use aes::Aes128;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, InOutBuf, PadType, UnpadValue};
// 或使用 pkcs5 padding (PKCS5 在 16-byte block 下与 PKCS7 等价)
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p im-common`
Expected: 2 tests pass

- [ ] **Step 6: Commit**

```bash
git add im-common/src/ im-common/Cargo.toml
git commit -m "feat: implement AES/ECB/PKCS7Padding cipher"
```

---

## Task 3: im-common — TCP Head 解析

**Files:**
- Modify: `im-common/src/lib.rs` (add tcp_head module)
- Create: `im-common/src/tcp_head.rs`

**Interfaces:**
- Consumes: 无外部依赖
- Produces: `TcpFrameHeader::parse([u8; 2]) -> Self`, `TcpFrameHeader::build(encrypted, zipped) -> [u8; 2]`

- [ ] **Step 1: 编写测试**

```rust
// im-common/src/tests.rs — 追加
use super::tcp_head::TcpFrameHeader;

#[test]
fn test_parse_encrypted_uncompressed() {
    let head = TcpFrameHeader::parse([0xC0, 0x80]);
    assert!(head.encrypted);
    assert!(!head.zipped);
    assert!(!head.encrypted_system_version);
    assert!(!head.is_report);
}

#[test]
fn test_parse_encrypted_compressed() {
    let head = TcpFrameHeader::parse([0xC0, 0xC0]);
    assert!(head.encrypted);
    assert!(head.zipped);
}

#[test]
fn test_build_encrypted_uncompressed() {
    let result = TcpFrameHeader::build(true, false);
    assert_eq!(result, [0xC0, 0x80]);
}

#[test]
fn test_build_encrypted_compressed() {
    let result = TcpFrameHeader::build(true, true);
    assert_eq!(result, [0xC0, 0xC0]);
}

#[test]
fn test_roundtrip() {
    let original = [0xC0, 0x80];
    let parsed = TcpFrameHeader::parse(original);
    let rebuilt = TcpFrameHeader::build(parsed.encrypted, parsed.zipped);
    assert_eq!(rebuilt, original);
}
```

- [ ] **Step 2: 实现 tcp_head.rs**

```rust
// im-common/src/tcp_head.rs
/// TCP 帧头字节解析。
///
/// byte[0] = 0xC0 固定标志位
/// byte[1] bit7 = encrypted  (0x80), bit6 = zipped  (0x40),
///           bit5 = encryptedSystemVersion (0x20), bit4 = isReport (0x10),
///           bits 3-0 = protocolVersion (0x0F)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpFrameHeader {
    pub encrypted: bool,
    pub zipped: bool,
    pub encrypted_system_version: bool,
    pub is_report: bool,
    pub protocol_version: u8,
}

impl TcpFrameHeader {
    pub fn parse(head: [u8; 2]) -> Self {
        assert_eq!(head[0], 0xC0, "Invalid TCP head byte[0]: expected 0xC0");
        let b1 = head[1];
        Self {
            encrypted: (b1 & 0x80) != 0,
            zipped: (b1 & 0x40) != 0,
            encrypted_system_version: (b1 & 0x20) != 0,
            is_report: (b1 & 0x10) != 0,
            protocol_version: b1 & 0x0F,
        }
    }

    pub fn build(encrypted: bool, zipped: bool) -> [u8; 2] {
        let mut b1 = 0x00u8;
        if encrypted {
            b1 |= 0x80;
        }
        if zipped {
            b1 |= 0x40;
        }
        [0xC0, b1]
    }
}
```

- [ ] **Step 3: 运行测试**

Run: `cargo test -p im-common`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add im-common/src/tcp_head.rs im-common/src/lib.rs
git commit -m "feat: implement TCP frame header parsing"
```

---

## Task 4: im-common — X-One 头生成

**Files:**
- Create: `im-common/src/version_key.rs`
- Modify: `im-common/src/lib.rs`
- Modify: `im-common/src/tests.rs`

**Interfaces:**
- Consumes: `AesCipher`, `md-5`
- Produces: `VersionKeyManager::build_x_one(&self) -> String`

**重要细节**:
- `V_L_SALT` = MD5 hash of `"sjlkajsl*Rkfsdsd_tflklsjdf"`，取前 16 字节（小端或原始 byte 顺序）
- 明文字符串: `"{secretName},{timestamp_millis}"`
- 加密后转 hex 小写字符串

- [ ] **Step 1: 编写测试**

```rust
// im-common/src/tests.rs — 追加
use super::version_key::VersionKeyManager;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_version_key_manager_creation() {
    let manager = VersionKeyManager::new(
        "f82956caf0fa90aecf24d5ef9541f624".to_string(),
        "f58c15f54e8f7826".to_string(),
    );
    let x_one = manager.build_x_one();
    assert!(!x_one.is_empty());
    // 应该是 32 字符 hex (16 bytes)
    assert_eq!(x_one.len(), 32);
}

#[test]
fn test_v_salt_constant() {
    // V_L_SALT = md5("sjlkajsl*Rkfsdsd_tflklsjdf")[0..16]
    let expected = md5::compute("sjlkajsl*Rkfsdsd_tflklsjdf");
    let expected_bytes = &expected.as_slice()[..16];
    assert_eq!(expected_bytes.len(), 16);
}
```

- [ ] **Step 2: 实现 version_key.rs**

```rust
// im-common/src/version_key.rs
use crate::aes::AesCipher;
use crate::error::AppResult;

/// X-One header 生成器。
///
/// 格式: hex(AES_V_L_SALT(secretName + "," + timestamp_ms))
/// 其中 V_L_SALT = md5("sjlkajsl*Rkfsdsd_tflklsjdf").first16Bytes
pub struct VersionKeyManager {
    secret_name: String,
    header_cipher: AesCipher,
}

impl VersionKeyManager {
    pub fn new(secret_name: String, header_key: String) -> Self {
        assert_eq!(
            header_key.len(),
            16,
            "header key must be 16 bytes (UTF-8 string)"
        );
        Self {
            secret_name,
            header_cipher: AesCipher::new(header_key.as_bytes()),
        }
    }

    /// 生成 X-One header 值
    pub fn build_x_one(&self) -> AppResult<String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let plaintext = format!("{},{}", self.secret_name, timestamp);
        let encrypted = self.header_cipher.encrypt(plaintext.as_bytes())?;
        Ok(hex::encode(encrypted))
    }

    pub fn secret_name(&self) -> &str {
        &self.secret_name
    }
}
```

- [ ] **Step 3: 运行测试**

Run: `cargo test -p im-common`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add im-common/src/version_key.rs im-common/src/lib.rs
git commit -m "feat: implement X-One header generation"
```

---

## Task 5: im-common — 配置结构

**Files:**
- Create: `im-common/src/config.rs`
- Modify: `im-common/src/lib.rs`

**Interfaces:**
- Produces: `AppConfig`, `ServerConfig`, `DeviceConfig`, `default_config()`

- [ ] **Step 1: 实现 config.rs**

```rust
// im-common/src/config.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub device: DeviceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub openchat_user_url: String,
    pub im_biz_url: String,
    pub im_chat_host: String,
    pub im_chat_port: u16,
    pub version_secret_name: String,
    pub body_aes_key: String,
    pub header_aes_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub app_ver: i32,
    pub package_code: i32,
    pub plat: i32,
    pub language: i32,
    pub sys_mac: String,
    pub sys_model: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            device: DeviceConfig {
                app_ver: 680,
                package_code: 9803,
                plat: 0,
                language: 2,
                sys_mac: uuid::Uuid::new_v4().to_string(),
                sys_model: "PC-TOOLS".to_string(),
            },
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            openchat_user_url: "https://test-ochat-user1.68chat.co".to_string(),
            im_biz_url: "https://test-biz-b.68chat.co".to_string(),
            im_chat_host: "35.220.159.225".to_string(),
            im_chat_port: 9500,
            version_secret_name: "f82956caf0fa90aecf24d5ef9541f624".to_string(),
            body_aes_key: "97b1f52761ffc7f8".to_string(),
            header_aes_key: "f58c15f54e8f7826".to_string(),
        }
    }
}

impl DeviceConfig {
    pub fn new() -> Self {
        Self::default()
    }
}
```

- [ ] **Step 2: 运行测试**

Run: `cargo test -p im-common`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add im-common/src/config.rs im-common/src/lib.rs
git commit -m "feat: add configuration structs with defaults"
```

---

## Task 6: Proto 文件准备与 im-proto 编译

**Files:**
- Create: `proto/common.proto`（复制自 Java 源码，添加 rust 选项）
- Create: `proto/im.proto`（复制自 Java 源码，添加 rust 选项）
- Create: `proto/group.proto`（复制自 Java 源码，添加 rust 选项）
- Create: `proto/login.proto`（复制自 Java 源码，添加 rust 选项）
- Create: `proto/friend_message.proto`（精简版）
- Create: `proto/group_message.proto`（精简版）
- Create: `proto/channel_event.proto`（精简版）
- Create: `im-proto/build.rs`
- Create: `im-proto/src/lib.rs`

**Interfaces:**
- Consumes: prost-build
- Produces: `im::GroupMessage`, `im::PushGroupMessage`, `im::LoginSessionMessage`, `common::ClientInfo`, `group::GroupBase`, `group::GroupContactListResp`, `login::LoginReq`, `login::LoginResp`, `MessageType`

- [ ] **Step 1: 复制并修改 proto 文件**

从 Java 源码复制 proto 文件到 `proto/` 目录，并添加 Rust 选项。关键修改:

1. `common.proto` — 无需大改，prost 可以处理 Java 选项
2. `im.proto` — 添加所需的 message 定义（确保 `LoginSessionMessage` 包含 `clinetInfo` 拼写）
3. `group.proto` — 提取 `GroupBase`, `GroupContactListReq`, `GroupContactListResp`
4. `login.proto` — 提取 `LoginReq`, `LoginResp`, `UrlInfo`
5. `friend_message.proto` — 只包含 `OneToOneMessage` 及相关依赖（或移除依赖）
6. `group_message.proto` — 只包含 `GroupReqMsgDto` 及相关（或移除）
7. `channel_event.proto` — 最小化

> **关键**: proto 文件中的 `import` 语句需要正确处理。最简单的方案是把所有需要的 message 合并到一个 proto 文件中，或创建独立的 proto 文件只包含需要的内容。

- [ ] **Step 2: 创建合并后的 proto 文件（推荐方案）**

将需要的 message 合并到单个 `broadcast.proto` 中以避免 import 问题:

```protobuf
// proto/broadcast.proto
syntax = "proto3";

option java_package = "com.im.pb";
option java_outer_classname = "IMPB";

// ========== 枚举 ==========

enum MessageType {
    text = 0;
    image = 1;
    audio = 2;
    video = 3;
    location = 4;
    nameCard = 5;
    system = 6;
    file = 7;
    notice = 8;
    dynamicImage = 9;
    redPacket = 10;
    html = 11;
    setImage = 12;
    chatTransfer = 13;
    chatTransferResult = 14;
    redPacketResult = 15;
    html2 = 16;
    mediasCaption = 17;
    animatedGame = 18;
    redPacketCountChange = 19;
    redPacketGameEvent = 20;
    groupChatTrade = 21;
}

enum Platform {
    ANDROID = 0;
    IPHONE = 1;
    UNKOWN = 2;
    MAC = 3;
    WIN = 4;
    HARMONYOS = 5;
}

enum GroupMemberType {
    HOST = 0;
    MANAGE = 1;
    MEMBER = 2;
}

enum AccountType {
    MOBILE = 0;
    EMAIL = 1;
}

enum LoginMode {
    HAND = 0;
    SYS_AUTO = 1;
}

enum LoginType {
    SMS_CODE = 0;
    PASSWORD = 1;
    AUTH_KEY = 2;
    GOOGLE_TOKEN = 3;
    APPLE_TOKEN = 4;
}

enum GetValidateCodeType {
    REG = 0;
    LOGIN = 1;
    FIND_PASSWORD = 2;
    UPDATE_PASSWORD = 3;
    UPDATE_PHONE = 4;
    VALIDATE_PASSWORD = 6;
    FIND_GESTURE_PASSWORD = 7;
    TRADE_PASSWORD = 8;
    UPDATE_TRADE_PASSWORD = 9;
    BIND_PHONE = 10;
    BIND_EMAIL = 11;
    UPDATE_EMAIL = 12;
    VERIFY_GOOGLE_CODE = 19;
    BIND_ACCOUNT = 20;
    VERIFY_LOGIN_PHONE = 21;
    VERIFY_LOGIN_EMAIL = 22;
    VERIFY_LOGIN_PASSWORD = 23;
}

// ========== 消息 ==========

message ClientInfo {
    string session_id = 1;
    int32 app_ver = 2;
    int32 package_code = 3;
    Platform plat = 4;
    int32 language = 5;
    string sys_mac = 6;
    string sys_model = 7;
    string token = 8;
    string version = 9;
}

message CommonResult {
    int32 err_code = 1;
    string err_msg = 2;
    string flag = 3;
    int32 display = 4;
    string title = 5;
}

message CommonResultReq {
    ClientInfo client_info = 1;
}

message UserBase {
    int64 uid = 1;
    string nick_name = 2;
    string icon = 3;
    Gender gender = 4;
    FriendRelation friend_relation = 5;
    UserOnOrOffLine user_on_or_off_line = 6;
    string signature = 7;
    string depict = 8;
    bool bf_cancel = 9;
    bool bf_banned = 10;
    string identify = 11;
    string real_name = 12;
    string id_number = 13;
    int64 create_time = 14;
    int32 user_type = 15;
}

enum Gender {
    SECRECY = 0;
    MALE = 1;
    FEMALE = 2;
}

message FriendRelation {
    bool bf_friend = 1;
    string remark_name = 2;
    int32 status = 3;
}

message UserOnOrOffLine {
    int64 uid = 1;
    bool online = 2;
    int64 create_time = 3;
    bool bf_show = 4;
}

message GroupMemberBase {
    UserBase user = 1;
    int64 group_id = 2;
    GroupMemberType type = 3;
    string group_nick_name = 4;
    int64 score = 5;
    AdminRightBase right = 6;
    bool bf_my_black = 7;
    int32 label_type = 8;
}

message AdminRightBase {
    bool bf_update_data = 1;
    bool bf_join_check = 2;
    bool bf_push_notice = 3;
    bool bf_set_admin = 4;
    bool bf_reset_qrcode = 5;
    bool bf_set_join_notice = 6;
    bool bf_set_live = 7;
}

message GroupBase {
    int64 group_id = 1;
    int64 host_id = 2;
    string name = 3;
    string pic = 4;
    bool bf_join_check = 5;
    int64 create_time = 6;
    int64 member_count = 7;
    string desc = 8;
    int32 group_type = 9;
    int32 status = 10;
    int64 last_msg_time = 11;
    string last_msg = 12;
    int32 mute = 13;
    int64 owner_uid = 14;
    string alias = 15;
    bool bf_quit = 16;
    bool bf_disturb = 17;
    bool bf_star = 18;
    int32 member_limit = 19;
    int32 show_group_name = 20;
    int64 notice_id = 21;
    string notice = 22;
    int64 notice_time = 23;
    string notice_user_id = 24;
    int32 apply_join_type = 25;
    int64 last_msg_id = 26;
    int32 msg_count = 27;
    int32 remark = 28;
    string extra = 29;
    bool bf_hide = 30;
}

message GroupContactListReq {
    CommonResultReq common_result_req = 1;
}

message GroupContactListResp {
    CommonResult common_result = 1;
    int32 group_count = 2;
    repeated GroupBase groups = 3;
}

message LoginSessionMessage {
    ClientInfo clinet_info = 1;
    int64 latest_login_time = 2;
    string install_code = 3;
    int32 push_tag = 4;
}

message GroupMessage {
    int64 send_uid = 1;
    int64 group_id = 2;
    MessageType msg_type = 3;
    bytes content = 4;
    repeated int64 at_uids = 6;
    int64 send_time = 7;
    int64 msg_id = 8;
    GroupMemberBase send_member = 9;
    int32 version = 10;
    string content_md5 = 11;
    string attachment_key = 12;
    string group_name = 13;
    int32 snapchat_time = 14;
    repeated AtUser at_users = 15;
    UploadChannelType channel_type = 16;
    int32 msg_from = 17;
    int32 edit = 18;
    repeated LinkObj links = 19;
    int64 sent_over_time = 20;
    bool is_hide = 21;
}

message AtUser {
    int64 uid = 1;
    string nick_name = 2;
    FriendRelation friend_relation = 5;
}

enum UploadChannelType {
    OSS_DEFAULT = 0;
    OSS_CHAT = 1;
    OSS_LOW_RATE = 2;
}

message LinkObj {
    string link = 1;
    int32 location = 2;
    int32 length = 3;
}

message PushGroupMessage {
    repeated GroupMessage group_msg = 1;
    map<int64, int32> unread_count_map = 2;
}

message RecallMessage {
    int64 msg_id = 1;
    int64 send_uid = 2;
    int64 group_id = 3;
    int64 send_time = 4;
    int32 operator_uid = 5;
    MessageType msg_type = 6;
    string operator_name = 7;
    string content = 8;
    int32 version = 9;
    string content_md5 = 10;
    string attachment_key = 11;
    string group_name = 12;
}

// ========== 登录相关 ==========

message LoginReq {
    ClientInfo client_info = 1;
    string country_code = 2;
    string phone = 3;
    LoginMode login_mode = 4;
    LoginType login_type = 5;
    GetValidateCodeType type = 6;
    string sms_code = 7;
    string password = 8;
    string sys_version = 9;
    string sys_model = 10;
    string sys_mac = 11;
    string token = 12;
    string device_token = 13;
    string device_info = 14;
    AccountType account_type = 15;
    string auth_key = 16;
    string id_token = 17;
    string public_key = 18;
    string official_key = 19;
    string second_mac = 20;
}

message LoginResp {
    CommonResult common_result = 1;
    UserBase user = 2;
    string session_id = 3;
    int32 review_model = 4;
    UrlInfo urls = 5;
    bool login_reg = 6;
    int64 server_time = 7;
    string token = 8;
    string invite_code = 9;
    int32 privacy = 10;
    int64 disable_time = 11;
    string agora_app_id = 12;
    int64 upload_file_size = 13;
    int64 upload_image_size = 14;
    int32 need_reset_password = 15;
    int32 key_version = 16;
    int32 shoot_time_limit = 17;
    bool remind_change_password = 18;
    int64 upload_id_img_size = 19;
    bool is_not_last_device_mac = 20;
}

message UrlInfo {
    string biz = 1;
    string session = 2;
    string friend = 3;
    string group = 4;
    string static_map = 5;
    string download = 6;
    string login = 7;
    string config = 8;
    string wss = 9;
    int32 socket_protocol = 10;
    int32 upload_server = 11;
    string upload_url = 12;
    string wallet_url = 13;
    string news_url = 14;
    string otc_url = 15;
    string red_packet_url = 16;
    string payment_url = 17;
}

message ErrrMessage {
    int32 error_msg_code = 1;
    string error_msg = 2;
    int32 message_protocol_id = 3;
}
```

- [ ] **Step 3: 创建 im-proto/build.rs**

```rust
// im-proto/build.rs
use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let proto_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../proto");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", proto_dir.display());

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_well_known_types()
        .extern_path(".google.protobuf", "::prost::alloc::boxed::Box")
        .compile_protos(&["broadcast.proto"], &[proto_dir])
        .expect("Failed to compile protos");

    // Also copy the generated files to a known location for im-chat to find
    let gen_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("../src/generated");
    std::fs::create_dir_all(&gen_dir).unwrap();
    for entry in walkdir::WalkDir::new(&out_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_name().to_string_lossy().ends_with(".rs") {
            let file_path = entry.path();
            let relative = file_path.strip_prefix(&out_dir).unwrap();
            let dest = gen_dir.join(relative);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(file_path, dest).unwrap();
        }
    }
}
```

> **注意**: 需要在 `im-proto/Cargo.toml` 中添加 `walkdir` 和 `prost` 的 well-known-types。更简洁的方案是不拷贝，直接让 lib.rs 引用 `include!(concat!(env!("OUT_DIR"), "/broadcast.rs"));`

简化版 build.rs（不使用 walkdir）:
```rust
// im-proto/build.rs
fn main() {
    prost_build::Config::new()
        .compile_protos(&["../proto/broadcast.proto"], &["../proto/"])
        .unwrap();
}
```

生成的代码会放在 `$OUT_DIR/broadcast.rs`，由 lib.rs 通过 `include!` 引用。

- [ ] **Step 4: 创建 im-proto/src/lib.rs**

```rust
// im-proto/src/lib.rs
pub mod broadcast {
    include!(concat!(env!("OUT_DIR"), "/broadcast.rs"));
}

// Re-export commonly used types
pub use broadcast::{
    group_message::GroupMessage,
    push_group_message::PushGroupMessage,
    login_session_message::LoginSessionMessage,
    group_contact_list_resp::GroupContactListResp,
    login_req::LoginReq,
    login_resp::LoginResp,
    client_info::ClientInfo,
    common_result::CommonResult,
    common_result_req::CommonResultReq,
    group_base::GroupBase,
    url_info::UrlInfo,
    errr_message::ErrrMessage,
    message_type::MessageType,
    platform::Platform,
    group_member_base::GroupMemberBase,
    user_base::UserBase,
};
```

- [ ] **Step 5: 编译验证**

Run: `cargo build -p im-proto 2>&1`
Expected: SUCCESS — proto 编译通过，生成 Rust binding

- [ ] **Step 6: Commit**

```bash
git add proto/ im-proto/
git commit -m "feat: add protobuf bindings via prost"
```

---

## Task 7: im-chat — TCP 帧编码/解码

**Files:**
- Create: `im-chat/src/lib.rs`
- Create: `im-chat/src/frame.rs`
- Create: `im-chat/src/tests.rs`

**Interfaces:**
- Consumes: `im_common::tcp_head::TcpFrameHeader`, `im_proto`
- Produces: `encode_frame(message_id, content: &[u8], encrypted: bool, zipped: bool) -> Vec<u8>`, `decode_frame(data: &[u8]) -> Result<(u16, Vec<u8>)>`

- [ ] **Step 1: 编写测试**

```rust
// im-chat/src/tests.rs
use super::frame::{decode_frame, encode_frame};
use im_common::tcp_head::TcpFrameHeader;

#[test]
fn test_encode_and_decode_frame() {
    let content = b"test protobuf data";
    let framed = encode_frame(
        1000,                        // messageId
        content,
        false,                       // encrypted
        false,                       // zipped
    );

    // Frame should start with [0xC0, 0x80]
    assert_eq!(&framed[0..2], &[0xC0, 0x80]);

    let (msg_id, body) = decode_frame(&framed).unwrap();
    assert_eq!(msg_id, 1000);
    assert_eq!(body, content);
}

#[test]
fn test_encode_frame_big_endian() {
    let content = b"hello";
    let framed = encode_frame(0x0102, content, false, false);

    // messageId 0x0102 should be big-endian: [0x01, 0x02]
    assert_eq!(&framed[2..4], &[0x01, 0x02]);

    // contentLength 5 should be big-endian: [0x00, 0x00, 0x00, 0x05]
    assert_eq!(&framed[4..8], &[0x00, 0x00, 0x00, 0x05]);
}

#[test]
fn test_decode_invalid_frame() {
    let invalid = vec![0xFF, 0xFF, 0xFF];
    assert!(decode_frame(&invalid).is_err());
}
```

- [ ] **Step 2: 实现 frame.rs**

```rust
// im-chat/src/frame.rs
use crate::error::AppResult;
use im_common::tcp_head::TcpFrameHeader;

/// 编码 TCP 帧
/// wire: [head(2)][messageId(2,BE)][contentLength(4,BE)][content]
pub fn encode_frame(
    message_id: u16,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Vec<u8> {
    let head = TcpFrameHeader::build(encrypted, zipped);
    let content_len = content.len() as u32;

    let mut buf = Vec::with_capacity(2 + 2 + 4 + content.len());
    buf.extend_from_slice(&head);
    buf.extend_from_slice(&message_id.to_be_bytes());
    buf.extend_from_slice(&content_len.to_be_bytes());
    buf.extend_from_slice(content);
    buf
}

/// 解码 TCP 帧
/// 返回 (message_id, content_bytes)
pub fn decode_frame(data: &[u8]) -> AppResult<(u16, Vec<u8>)> {
    if data.len() < 8 {
        return Err(AppError::TcpFrame("data too short for frame header".to_string()));
    }

    let head = [data[0], data[1]];
    let _header = TcpFrameHeader::parse(head);
    let message_id = u16::from_be_bytes([data[2], data[3]]);
    let content_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;

    if data.len() < 8 + content_len {
        return Err(AppError::TcpFrame(format!(
            "truncated frame: need {} bytes, have {}",
            8 + content_len,
            data.len() - 8
        )));
    }

    let content = data[8..8 + content_len].to_vec();
    Ok((message_id, content))
}
```

- [ ] **Step 3: 运行测试**

Run: `cargo test -p im-chat`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add im-chat/src/frame.rs im-chat/src/tests.rs im-chat/src/lib.rs im-chat/Cargo.toml
git commit -m "feat: implement TCP frame encode/decode"
```

---

## Task 8: im-chat — TCP 客户端骨架

**Files:**
- Create: `im-chat/src/client.rs`
- Create: `im-chat/src/heartbeat.rs`
- Create: `im-chat/src/reconnect.rs`
- Modify: `im-chat/src/lib.rs`

**Interfaces:**
- Consumes: `encode_frame`, `decode_frame`, `im_proto`
- Produces: `ChatClient` struct with `connect()`, `login()`, `send()` methods, `on_message` callback

**消息 ID 常量:**
- HeartBeat = 1000
- LoginServer = 1100
- PushLoginSuccess = 1201
- PushGroupMessage = 2202
- PushRecallGroupMessage = 2205

- [ ] **Step 1: 创建 lib.rs**

```rust
// im-chat/src/lib.rs
pub mod client;
pub mod frame;
pub mod heartbeat;
pub mod reconnect;

pub use client::ChatClient;
pub use frame::{decode_frame, encode_frame};
```

- [ ] **Step 2: 创建 client.rs（骨架）**

```rust
// im-chat/src/client.rs
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn, error};

use crate::frame::{decode_frame, encode_frame};
use im_common::config::AppConfig;
use im_proto::{GroupMessage, LoginSessionMessage, MessageType, PushGroupMessage};

pub type MessageHandler = Box<dyn Fn(u16, &[u8]) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ChatClient {
    config: AppConfig,
    stream: Option<Arc<tokio::sync::Mutex<TcpStream>>>,
    handler: Option<Arc<MessageHandler>>,
    sender: Option<mpsc::Sender<Vec<u8>>>,
}

impl ChatClient {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            stream: None,
            handler: None,
            sender: None,
        }
    }

    pub fn on_message<F>(&mut self, handler: F)
    where
        F: Fn(u16, &[u8]) + Send + Sync + 'static,
    {
        self.handler = Some(Arc::new(Box::new(handler)));
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!(
            "{}:{}",
            self.config.server.im_chat_host, self.config.server.im_chat_port
        );
        info!("Connecting to IM chat server: {}", addr);
        let stream = TcpStream::connect(&addr).await?;
        let stream = Arc::new(tokio::sync::Mutex::new(stream));
        self.stream = Some(stream.clone());

        // 启动读取任务
        let reader = ReadTask {
            stream: stream.clone(),
            handler: self.handler.clone(),
        };
        tokio::spawn(reader.run());

        Ok(())
    }

    pub async fn disconnect(&mut self) {
        self.stream = None;
        self.sender = None;
    }

    pub async fn login(&self, token: &str, uid: i64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = self.stream.as_ref().ok_or("Not connected")?;
        let mut conn = stream.lock().await;

        let login_msg = LoginSessionMessage {
            clinet_info: Some(im_proto::ClientInfo {
                session_id: "".to_string(),
                app_ver: self.config.device.app_ver,
                package_code: self.config.device.package_code,
                plat: im_proto::Platform::Android as i32,
                language: self.config.device.language,
                sys_mac: self.config.device.sys_mac.clone(),
                sys_model: self.config.device.sys_model.clone(),
                token: token.to_string(),
                version: format!("{}-{}", self.config.device.app_ver, self.config.device.package_code),
            }),
            latest_login_time: 0,
            install_code: self.config.device.sys_mac.clone(),
            push_tag: 1,
        };

        let body = login_msg.encode_to_vec();
        let frame = encode_frame(1100, &body, true, false); // encrypted=true, zipped=false

        tokio::io::AsyncWriteExt::write_all(&mut *conn, &frame).await?;
        tokio::io::AsyncWriteExt::flush(&mut *conn).await?;

        info!("Login message sent to IM chat server");
        Ok(())
    }

    pub async fn send(&self, message_id: u16, content: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = self.stream.as_ref().ok_or("Not connected")?;
        let mut conn = stream.lock().await;
        let frame = encode_frame(message_id, content, true, false);
        tokio::io::AsyncWriteExt::write_all(&mut *conn, &frame).await?;
        tokio::io::AsyncWriteExt::flush(&mut *conn).await?;
        Ok(())
    }
}

struct ReadTask {
    stream: Arc<tokio::sync::Mutex<TcpStream>>,
    handler: Option<Arc<MessageHandler>>,
}

impl ReadTask {
    async fn run(mut self) {
        let mut buf = Vec::new();
        loop {
            let mut conn = self.stream.lock().await;
            match tokio::io::AsyncReadExt::read_to_end(&mut *conn, &mut buf).await {
                Ok(0) => {
                    warn!("Connection closed by server");
                    break;
                }
                Ok(_) => {
                    // 处理接收到的数据
                    self.handle_data(&buf).await;
                    buf.clear();
                }
                Err(e) => {
                    error!("Read error: {}", e);
                    break;
                }
            }
        }
    }

    async fn handle_data(&self, data: &[u8]) {
        let mut offset = 0;
        while offset + 8 <= data.len() {
            match decode_frame(&data[offset..]) {
                Ok((msg_id, content)) => {
                    if let Some(handler) = &self.handler {
                        handler(msg_id, &content);
                    }
                    offset += 8 + content.len();
                }
                Err(_) => break,
            }
        }
    }
}
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p im-chat 2>&1`
Expected: 修复编译错误（可能有类型不匹配）

- [ ] **Step 4: Commit**

```bash
git add im-chat/src/
git commit -m "feat: implement TCP chat client skeleton"
```

---

## Task 9: im-store — SQLite 存储层

**Files:**
- Create: `im-store/src/lib.rs`
- Create: `im-store/src/schema.rs`
- Create: `im-store/src/message.rs`
- Create: `im-store/src/group.rs`
- Create: `im-store/Cargo.toml`（含 sqlx 依赖）

**Interfaces:**
- Consumes: `im_proto`, `sqlx`
- Produces: `SqliteStore` with `insert_message()`, `get_groups()`, `update_group_monitored()`

- [ ] **Step 1: 编写测试**

```rust
// im-store/src/tests.rs
use super::*;

#[tokio::test]
async fn test_create_tables() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    // 表应该已创建
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups").fetch_one(&store.pool).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_insert_and_fetch_message() {
    let store = SqliteStore::new(":memory:").await.unwrap();
    let msg_id = store.insert_message(MessageRecord {
        msg_id: 1001,
        group_id: 12345,
        send_uid: 779562,
        msg_type: 0, // text
        content: b"Hello, World!".to_vec(),
        send_time: 1725292800000,
        content_md5: "d41d8cd98f00b204e9800998ecf8427e".to_string(),
    })
    .await
    .unwrap();
    assert_eq!(msg_id, 1001);
}
```

- [ ] **Step 2: 实现 schema.rs**

```rust
// im-store/src/schema.rs
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS groups (
    group_id    INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    pic         TEXT DEFAULT '',
    host_id     INTEGER,
    member_count INTEGER DEFAULT 0,
    created_at  INTEGER NOT NULL,
    monitored   INTEGER NOT NULL DEFAULT 1,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    msg_id      INTEGER PRIMARY KEY,
    group_id    INTEGER NOT NULL REFERENCES groups(group_id),
    send_uid    INTEGER NOT NULL,
    msg_type    INTEGER NOT NULL,
    content     BLOB NOT NULL,
    send_time   INTEGER NOT NULL,
    content_md5 TEXT DEFAULT '',
    stored_at   INTEGER NOT NULL,
    raw_proto   BLOB
);

CREATE INDEX IF NOT EXISTS idx_messages_group_time ON messages(group_id, send_time);
CREATE INDEX IF NOT EXISTS idx_groups_monitored ON groups(monitored) WHERE monitored = 1;
"#;
```

- [ ] **Step 3: 实现 message.rs 和 group.rs**

```rust
// im-store/src/message.rs
use sqlx::SqlitePool;
use super::schema::SCHEMA_SQL;

#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub msg_id: i64,
    pub group_id: i64,
    pub send_uid: i64,
    pub msg_type: i32,
    pub content: Vec<u8>,
    pub send_time: i64,
    pub content_md5: String,
}

#[derive(sqlx::FromRow, Debug)]
pub struct MessageRow {
    pub msg_id: i64,
    pub group_id: i64,
    pub send_uid: i64,
    pub msg_type: i32,
    pub content: Vec<u8>,
    pub send_time: i64,
    pub content_md5: String,
    pub stored_at: i64,
    pub raw_proto: Option<Vec<u8>>,
}

pub struct MessageStore {
    pool: SqlitePool,
}

impl MessageStore {
    pub async fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, record: &MessageRecord) -> sqlx::Result<i64> {
        sqlx::query!(
            r#"INSERT OR REPLACE INTO messages
               (msg_id, group_id, send_uid, msg_type, content, send_time, content_md5, stored_at, raw_proto)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            record.msg_id,
            record.group_id,
            record.send_uid,
            record.msg_type,
            record.content,
            record.send_time,
            record.content_md5,
            chrono::Utc::now().timestamp_millis(),
            None::<Vec<u8>>,
        )
        .execute(&self.pool)
        .await?;
        Ok(record.msg_id)
    }

    pub async fn get_by_group(&self, group_id: i64, limit: usize, offset: usize) -> sqlx::Result<Vec<MessageRow>> {
        sqlx::query_as!(
            MessageRow,
            r#"SELECT msg_id, group_id, send_uid, msg_type, content, send_time, content_md5, stored_at, raw_proto
               FROM messages WHERE group_id = ? ORDER BY send_time DESC LIMIT ? OFFSET ?"#,
            group_id,
            limit as i64,
            offset as i64,
        )
        .fetch_all(&self.pool)
        .await
    }
}
```

```rust
// im-store/src/group.rs
use sqlx::SqlitePool;

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct GroupRow {
    pub group_id: i64,
    pub name: String,
    pub pic: String,
    pub host_id: Option<i64>,
    pub member_count: i64,
    pub created_at: i64,
    pub monitored: i32,
    pub updated_at: i64,
}

pub struct GroupStore {
    pool: SqlitePool,
}

impl GroupStore {
    pub async fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert_or_update(&self, group: &GroupRow) -> sqlx::Result<()> {
        sqlx::query!(
            r#"INSERT INTO groups (group_id, name, pic, host_id, member_count, created_at, monitored, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(group_id) DO UPDATE SET
                   name = excluded.name,
                   pic = excluded.pic,
                   member_count = excluded.member_count,
                   updated_at = excluded.updated_at"#,
            group.group_id,
            group.name,
            group.pic,
            group.host_id,
            group.member_count,
            group.created_at,
            group.monitored,
            group.updated_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_monitored(&self) -> sqlx::Result<Vec<GroupRow>> {
        sqlx::query_as!(
            GroupRow,
            "SELECT * FROM groups WHERE monitored = 1 ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn toggle_monitored(&self, group_id: i64, monitored: bool) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE groups SET monitored = ? WHERE group_id = ?",
            monitored as i32,
            group_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

- [ ] **Step 4: 实现 lib.rs**

```rust
// im-store/src/lib.rs
pub mod schema;
pub mod message;
pub mod group;

use sqlx::SqlitePool;
use schema::SCHEMA_SQL;

pub struct SqliteStore {
    pub pool: SqlitePool,
    pub messages: message::MessageStore,
    pub groups: group::GroupStore,
}

impl SqliteStore {
    pub async fn new(dsn: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(dsn).await?;
        sqlx::query(SCHEMA_SQL).execute(&pool).await?;
        Ok(Self {
            pool,
            messages: message::MessageStore::new(pool.clone()).await,
            groups: group::GroupStore::new(pool.clone()).await,
        })
    }
}
```

- [ ] **Step 5: 运行测试**

Run: `cargo test -p im-store`
Expected: tests pass

- [ ] **Step 6: Commit**

```bash
git add im-store/
git commit -m "feat: implement SQLite storage layer"
```

---

## Task 10: im-http — HTTP 客户端（openchat-user）

> **注意**: 此 task 属于 Phase 2 范围，但为了后续任务可运行，先实现骨架。实际 HTTP 请求逻辑留到 Phase 2。

**Files:**
- Create: `im-http/Cargo.toml`
- Create: `im-http/src/lib.rs`
- Create: `im-http/src/openchat_user.rs`
- Create: `im-http/src/im_biz.rs`

**Interfaces:**
- Consumes: `im_common`, `reqwest`
- Produces: `OpenChatUserClient`, `ImBizClient` 骨架

- [ ] **Step 1: 创建 im-http Cargo.toml**

```toml
[package]
name = "im-http"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
im-common = { path = "../im-common" }
im-proto = { path = "../im-proto" }
reqwest = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tokio = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
prost = { workspace = true }
```

- [ ] **Step 2: 创建骨架实现**

```rust
// im-http/src/lib.rs
pub mod openchat_user;
pub mod im_biz;
```

```rust
// im-http/src/openchat_user.rs
use super::*;

pub struct OpenChatUserClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, serde::Deserialize)]
pub struct SendCodeResult {
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ValidateTokenResult {
    pub validate_token: Option<String>,
    pub success: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct LoginResult {
    pub uid: Option<i64>,
    pub token: Option<String>,
    pub is_not_last_device_mac: Option<bool>,
    pub is_login_out: i32,
    pub old_session_id: Option<String>,
}

impl OpenChatUserClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    /// 发送短信验证码（带极验）
    pub async fn send_sms_captcha(
        &self,
        phone: &str,
        country_code: i32,
        gt4_dto: &serde_json::Value,
    ) -> Result<SendCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现加密请求
        let _ = (&self.base_url, phone, country_code, gt4_dto);
        todo!("Phase 2: implement encrypted HTTP request")
    }

    /// 获取 validateToken
    pub async fn issued(&self, validate_scene: i32) -> Result<ValidateTokenResult, Box<dyn std::error::Error + Send + Sync>> {
        todo!("Phase 2: implement encrypted HTTP request")
    }

    /// 验证验证码
    pub async fn verify(
        &self,
        validate_token: &str,
        second_mac: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("Phase 2: implement encrypted HTTP request")
    }

    /// 登录
    pub async fn login(
        &self,
        phone: &str,
        country_code: i32,
        validate_token: &str,
    ) -> Result<LoginResult, Box<dyn std::error::Error + Send + Sync>> {
        todo!("Phase 2: implement encrypted HTTP request")
    }
}
```

```rust
// im-http/src/im_biz.rs
use super::*;

pub struct ImBizClient {
    base_url: String,
    http: reqwest::Client,
    x_one_manager: Option<im_common::version_key::VersionKeyManager>,
}

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub group_id: i64,
    pub name: String,
    pub pic: String,
    pub host_id: Option<i64>,
    pub member_count: i64,
}

impl ImBizClient {
    pub fn new(base_url: String, x_one_manager: im_common::version_key::VersionKeyManager) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
            x_one_manager: Some(x_one_manager),
        }
    }

    /// 获取群列表
    pub async fn fetch_group_list(
        &self,
        client_info: &im_proto::ClientInfo,
    ) -> Result<Vec<GroupInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现 Protobuf + AES 加密请求
        let _ = client_info;
        todo!("Phase 2: implement protobuf HTTP request")
    }
}
```

- [ ] **Step 3: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过（有 todo! 但测试不会运行它们）

- [ ] **Step 4: Commit**

```bash
git add im-http/
git commit -m "feat: add im-http crate skeleton (Phase 2)"
```

---

## Task 11: im-app — Tauri 应用骨架

**Files:**
- Create: `im-app/src/main.rs`
- Create: `im-app/src/state.rs`
- Create: `im-app/src/commands/auth.rs`
- Create: `im-app/src/commands/groups.rs`
- Create: `im-app/src/commands/chat.rs`
- Create: `im-app/src/monitor.rs`
- Create: `im-app/src-tauri/tauri.conf.json`
- Create: `im-app/src-tauri/capabilities/default.json`
- Create: `im-app/src-tauri/build.rs`
- Create: `im-app/src-tauri/assets/index.html`
- Create: `im-app/src-tauri/assets/app.js`
- Create: `im-app/src-tauri/assets/style.css`

**Interfaces:**
- Consumes: `im_common`, `im_chat`, `im_store`
- Produces: Tauri v2 应用入口，群列表 UI，消息流 UI

- [ ] **Step 1: 创建 tauri.conf.json**

```json
{
  "productName": "IM Monitor",
  "version": "0.1.0",
  "identifier": "co.68chat.im-monitor",
  "build": {
    "frontendDist": "../src-tauri/assets",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "",
    "beforeBuildCommand": ""
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "title": "IM Monitor",
        "width": 1200,
        "height": 800,
        "resizable": true,
        "fullscreen": false
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": []
  }
}
```

- [ ] **Step 2: 创建 build.rs**

```rust
// im-app/src-tauri/build.rs
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 3: 创建 state.rs**

```rust
// im-app/src/state.rs
use std::sync::Arc;
use tokio::sync::Mutex;
use im_common::config::AppConfig;
use im_chat::ChatClient;
use im_store::SqliteStore;

#[derive(Default)]
pub struct AppState {
    pub config: Arc<tokio::sync::RwLock<AppConfig>>,
    pub db: Arc<SqliteStore>,
    pub chat_client: Arc<Mutex<Option<ChatClient>>>,
    pub token: Arc<tokio::sync::RwLock<Option<String>>>,
    pub uid: Arc<tokio::sync::RwLock<Option<i64>>>,
    pub monitoring_groups: Arc<tokio::sync::RwLock<std::collections::HashSet<i64>>>,
}
```

- [ ] **Step 4: 创建 Tauri commands（骨架）**

```rust
// im-app/src/commands/auth.rs
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn send_sms_code(
    state: State<'_, AppState>,
    phone: String,
    country_code: i32,
    gt4_dto: serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Phase 2: 实现
    let _ = (state, phone, country_code, gt4_dto);
    todo!("Phase 2")
}

#[tauri::command]
pub async fn login(
    state: State<'_, AppState>,
    phone: String,
    country_code: i32,
    validate_token: String,
) -> Result<serde_json::Value, String> {
    // Phase 2: 实现
    let _ = (state, phone, country_code, validate_token);
    todo!("Phase 2")
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    let mut token = state.token.write().await;
    *token = None;
    let mut uid = state.uid.write().await;
    *uid = None;
    Ok(())
}
```

```rust
// im-app/src/commands/groups.rs
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn fetch_group_list(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    // Phase 2: 实现
    let groups = state.db.groups.list_monitored().await.map_err(|e| e.to_string())?;
    serde_json::to_value(groups).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_monitor(
    state: State<'_, AppState>,
    group_id: i64,
    monitored: bool,
) -> Result<(), String> {
    state.db.groups.toggle_monitored(group_id, monitored).await.map_err(|e| e.to_string())?;
    let mut monitoring = state.monitoring_groups.write().await;
    if monitored {
        monitoring.insert(group_id);
    } else {
        monitoring.remove(&group_id);
    }
    Ok(())
}
```

```rust
// im-app/src/commands/chat.rs
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn connect_chat(state: State<'_, AppState>) -> Result<(), String> {
    let config = state.config.read().await.clone();
    let mut client = state.chat_client.lock().await;

    let mut chat_client = im_chat::ChatClient::new(config);
    let state_clone = state.clone();

    chat_client.on_message(move |msg_id: u16, content: &[u8]| {
        match msg_id {
            2202 => {
                // PushGroupMessage
                if let Ok(push_msg) = im_proto::PushGroupMessage::decode(content) {
                    // 过滤监控群，写入 DB
                    // 发送事件给前端
                }
            }
            1201 => {
                // PushLoginSuccess
            }
            _ => {}
        }
    });

    chat_client.connect().await.map_err(|e| e.to_string())?;

    if let Some(token) = state.token.read().await.clone() {
        if let Some(uid) = state.uid.read().await {
            chat_client.login(&token, *uid).await.map_err(|e| e.to_string())?;
        }
    }

    *client = Some(chat_client);
    Ok(())
}

#[tauri::command]
pub async fn disconnect_chat(state: State<'_, AppState>) {
    let mut client = state.chat_client.lock().await;
    client.take();
}
```

- [ ] **Step 5: 创建 main.rs**

```rust
// im-app/src/main.rs
mod state;
mod commands;
mod monitor;

use tauri::Manager;
use state::AppState;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("im_app=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let config = app_state::AppConfig::default();
            let db = futures::executor::block_on(async {
                im_store::SqliteStore::new("data/im_monitor.db").await
            }).unwrap();
            let state = AppState {
                config: Arc::new(tokio::sync::RwLock::new(config)),
                db: Arc::new(db),
                ..Default::default()
            };
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::auth::logout,
            commands::groups::fetch_group_list,
            commands::groups::toggle_monitor,
            commands::chat::connect_chat,
            commands::chat::disconnect_chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

> **注意**: main.rs 中使用了 `futures::executor::block_on` 在同步上下文中运行 async 代码，这在 Tauri setup 中是常见做法。

- [ ] **Step 6: 创建前端 index.html**

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>IM Monitor</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <div id="app">
        <!-- Login Panel -->
        <div id="login-panel" class="panel">
            <h2>登录</h2>
            <div class="form-group">
                <label>手机号</label>
                <input type="tel" id="phone" placeholder="请输入手机号" maxlength="11">
            </div>
            <div class="form-group">
                <label>验证码</label>
                <div class="code-row">
                    <input type="text" id="code" placeholder="请输入验证码" maxlength="6">
                    <button id="send-code-btn">发送验证码</button>
                </div>
            </div>
            <div class="form-group">
                <button id="login-btn" class="primary">登录</button>
            </div>
        </div>

        <!-- Main Panel -->
        <div id="main-panel" class="panel" style="display:none;">
            <header class="header">
                <h1>IM Monitor</h1>
                <div class="status" id="status">未登录</div>
            </header>

            <div class="layout">
                <aside class="sidebar">
                    <input type="text" id="search-groups" placeholder="搜索群...">
                    <div id="group-list" class="group-list"></div>
                    <button id="connect-btn">连接聊天</button>
                </aside>

                <main class="main">
                    <div id="chat-header">
                        <h3 id="selected-group-name">选择一个群</h3>
                    </div>
                    <div id="message-list" class="message-list"></div>
                    <div class="message-input">
                        <input type="text" placeholder="消息内容（只读监控模式）" disabled>
                    </div>
                </main>
            </div>
        </div>
    </div>

    <script src="app.js"></script>
</body>
</html>
```

- [ ] **Step 7: 创建前端 JS**

```javascript
// app.js
const { invoke } = window.__TAURI__.core;

let selectedGroupId = null;

async function loadGroups() {
    try {
        const groups = await invoke('fetch_group_list');
        renderGroupList(groups);
    } catch (e) {
        console.error('Failed to load groups:', e);
    }
}

function renderGroupList(groups) {
    const list = document.getElementById('group-list');
    list.innerHTML = groups.map(g => `
        <div class="group-item ${g.monitored ? 'monitored' : ''}" data-id="${g.group_id}">
            <span>${g.name}</span>
            <span class="count">${g.member_count}</span>
        </div>
    `).join('');

    list.querySelectorAll('.group-item').forEach(el => {
        el.addEventListener('click', () => {
            selectedGroupId = parseInt(el.dataset.id);
            document.querySelectorAll('.group-item').forEach(e => e.classList.remove('selected'));
            el.classList.add('selected');
            document.getElementById('selected-group-name').textContent = el.querySelector('span').textContent;
            loadMessages(selectedGroupId);
        });
    });
}

async function loadMessages(groupId) {
    // Phase 2: 从数据库加载历史消息
    document.getElementById('message-list').innerHTML = '<p class="loading">加载中...</p>';
}

document.getElementById('send-code-btn').addEventListener('click', async () => {
    const phone = document.getElementById('phone').value;
    // Phase 2: 调用 send_sms_code
    alert('Phase 2: 发送验证码功能');
});

document.getElementById('login-btn').addEventListener('click', async () => {
    const phone = document.getElementById('phone').value;
    const code = document.getElementById('code').value;
    // Phase 2: 调用 login
    alert('Phase 2: 登录功能');
});

document.getElementById('connect-btn').addEventListener('click', async () => {
    try {
        await invoke('connect_chat');
        document.getElementById('status').textContent = '已连接';
    } catch (e) {
        document.getElementById('status').textContent = '连接失败';
    }
});

// 监听消息事件
window.__TAURI__.event.listen('new_message', (event) => {
    if (event.payload.group_id === selectedGroupId) {
        appendMessage(event.payload);
    }
});

function appendMessage(msg) {
    const list = document.getElementById('message-list');
    const time = new Date(msg.send_time).toLocaleTimeString();
    const div = document.createElement('div');
    div.className = 'message';
    div.innerHTML = `<span class="time">${time}</span> <span class="content">${msg.content}</span>`;
    list.appendChild(div);
    list.scrollTop = list.scrollHeight;
}

// 初始化
loadGroups();
```

- [ ] **Step 8: 创建 CSS**

```css
/* style.css */
:root {
    --bg-primary: #1a1a2e;
    --bg-secondary: #16213e;
    --bg-card: #0f3460;
    --text-primary: #eaeaea;
    --text-secondary: #a0a0a0;
    --accent: #e94560;
    --success: #4ade80;
    --border: #2a2a4a;
}

* { margin: 0; padding: 0; box-sizing: border-box; }

body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: var(--bg-primary);
    color: var(--text-primary);
    height: 100vh;
    overflow: hidden;
}

#app { height: 100%; }

.panel {
    display: flex;
    flex-direction: column;
    height: 100%;
}

/* Login */
#login-panel {
    max-width: 400px;
    margin: auto;
    padding: 2rem;
    background: var(--bg-secondary);
    border-radius: 12px;
}

.form-group {
    margin-bottom: 1rem;
}

.form-group label {
    display: block;
    margin-bottom: 0.5rem;
    color: var(--text-secondary);
}

.form-group input {
    width: 100%;
    padding: 0.75rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 1rem;
}

.code-row {
    display: flex;
    gap: 0.5rem;
}

.code-row input { flex: 1; }

button {
    padding: 0.75rem 1.5rem;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    color: white;
    font-size: 1rem;
    cursor: pointer;
    transition: opacity 0.2s;
}

button:hover { opacity: 0.9; }

button.primary { width: 100%; }

/* Header */
.header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
}

.status {
    font-size: 0.875rem;
    padding: 0.25rem 0.75rem;
    border-radius: 12px;
    background: var(--bg-card);
}

/* Layout */
.layout {
    display: flex;
    flex: 1;
    overflow: hidden;
}

.sidebar {
    width: 300px;
    background: var(--bg-secondary);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
}

.sidebar input {
    margin: 1rem;
    padding: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-card);
    color: var(--text-primary);
}

.group-list {
    flex: 1;
    overflow-y: auto;
}

.group-item {
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--border);
    cursor: pointer;
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.group-item:hover, .group-item.selected {
    background: var(--bg-card);
}

.group-item .count {
    font-size: 0.75rem;
    color: var(--text-secondary);
}

.main {
    flex: 1;
    display: flex;
    flex-direction: column;
}

#chat-header {
    padding: 1rem;
    border-bottom: 1px solid var(--border);
}

.message-list {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
}

.message {
    padding: 0.5rem;
    margin-bottom: 0.5rem;
    background: var(--bg-card);
    border-radius: 6px;
}

.message .time {
    font-size: 0.75rem;
    color: var(--text-secondary);
    margin-right: 0.5rem;
}

.message-input {
    padding: 1rem;
    border-top: 1px solid var(--border);
}
```

- [ ] **Step 9: 编译验证**

Run: `cargo build -p im-app 2>&1`
Expected: 修复所有编译错误

- [ ] **Step 10: Commit**

```bash
git add im-app/
git commit -m "feat: add Tauri v2 app skeleton with UI"
```

---

## Task 12: 集成测试 — 端到端验证

- [ ] **Step 1: 编写集成测试**

```rust
// im-chat/src/tests/integration.rs
use im_common::{config::AppConfig, aes::AesCipher, tcp_head::TcpFrameHeader};
use im_chat::frame::{decode_frame, encode_frame};

#[test]
fn test_full_frame_workflow() {
    let config = AppConfig::default();
    let key = AesCipher::new(config.server.body_aes_key.as_bytes());

    // 1. 加密数据
    let plaintext = b"test protobuf content";
    let encrypted = key.encrypt(plaintext).unwrap();

    // 2. 编码帧
    let frame = encode_frame(2202, &encrypted, true, false);

    // 3. 解码帧
    let (msg_id, decrypted) = decode_frame(&frame).unwrap();
    assert_eq!(msg_id, 2202);

    // 4. 解密
    let result = key.decrypt(&decrypted).unwrap();
    assert_eq!(result, plaintext);
}

#[test]
fn test_version_key_generation() {
    use im_common::version_key::VersionKeyManager;
    let manager = VersionKeyManager::new(
        "f82956caf0fa90aecf24d5ef9541f624".to_string(),
        "f58c15f54e8f7826".to_string(),
    );
    let x_one = manager.build_x_one().unwrap();
    assert_eq!(x_one.len(), 32);
    // 验证 hex 解码成功
    hex::decode(&x_one).unwrap();
}
```

- [ ] **Step 2: 运行所有测试**

Run: `cargo test 2>&1`
Expected: 所有测试通过（除 todo! 部分外）

- [ ] **Step 3: 最终编译检查**

Run: `cargo build 2>&1`
Expected: BUILD SUCCESSFUL

- [ ] **Step 4: Commit**

```bash
git add .
git commit -m "test: add integration tests and final validation"
```

---

## 实施顺序总结

| 顺序 | Task | 模块 | 预计时间 |
|------|------|------|----------|
| 1 | Workspace 骨架 | 全部 | 5 min |
| 2 | AES 加解密 | im-common | 10 min |
| 3 | TCP Head 解析 | im-common | 5 min |
| 4 | X-One 头生成 | im-common | 10 min |
| 5 | 配置结构 | im-common | 5 min |
| 6 | Proto 编译 | im-proto | 15 min |
| 7 | TCP 帧编码 | im-chat | 10 min |
| 8 | TCP 客户端 | im-chat | 20 min |
| 9 | SQLite 存储 | im-store | 15 min |
| 10 | HTTP 骨架 | im-http | 10 min |
| 11 | Tauri 应用 | im-app | 30 min |
| 12 | 集成测试 | 全部 | 10 min |

**总计: ~2 小时完成 Phase 1 核心骨架**

Phase 2 将实现实际的 HTTP 加密请求（openchat-user 登录流程 + im-biz 群列表接口）。
