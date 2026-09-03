# Account Storage Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立系统凭据库、非敏感账号索引、每 UID 独立 SQLite 和旧数据库迁移基础设施。

**Architecture:** `account` 模块分别封装路径、账号索引、系统凭据、活动数据库和旧库迁移。`AppState` 不再在启动时持有固定 SQLite，而是通过 `AccountDatabaseManager` 在认证成功后选择账号数据库；认证与界面流程由后续计划接入这些稳定接口。

**Tech Stack:** Rust 2021、Tauri 2、Tokio、Serde、SQLx SQLite、keyring 3.6.3、Vitest（仅后续计划使用）

**Depends on:** `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/specs/2026-09-03-multi-account-user-experience-design.md`

---

## 文件结构

- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`：账号基础设施模块出口和统一错误。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/paths.rs`：数据根目录及 UID 数据库路径。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/index.rs`：非敏感账号索引的加载、原子保存和更新。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/credentials.rs`：凭据接口、系统 keyring 实现和内存测试替身。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/database.rs`：当前账号 SQLite 的打开、获取和关闭。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/migration.rs`：旧单库的一次性迁移。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/Cargo.toml`：固定兼容 Rust 1.75 的 `keyring`。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/Cargo.toml`：按平台启用 keyring 后端。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs`：初始化账号基础设施，不再提前打开业务库。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/state.rs`：`AppState` 改持有数据库管理器。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`、`groups.rs`、`chat.rs` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/message_content.rs`：从活动账号取得数据库。

### Task 1：固定 keyring 依赖并建立路径模型

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/Cargo.toml`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/Cargo.toml`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/paths.rs`

- [ ] **Step 1：写路径模型失败测试**

在 `paths.rs` 中先写测试，覆盖正 UID、拒绝非正 UID、公共文件和账号数据库位置：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_paths_keep_each_uid_in_its_own_directory() {
        let paths = AppPaths::new(std::path::PathBuf::from("/tmp/im-monitor"));
        assert_eq!(paths.index_file(), std::path::PathBuf::from("/tmp/im-monitor/accounts.json"));
        assert_eq!(paths.legacy_db(), std::path::PathBuf::from("/tmp/im-monitor/im_monitor.db"));
        assert_eq!(
            paths.account_db(42).unwrap(),
            std::path::PathBuf::from("/tmp/im-monitor/accounts/42/im_monitor.db"),
        );
        assert!(paths.account_db(0).is_err());
        assert!(paths.account_db(-1).is_err());
    }
}
```

- [ ] **Step 2：运行测试并确认失败**

运行：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account_paths_keep_each_uid_in_its_own_directory
```

预期：因 `account` 模块或 `AppPaths` 尚不存在而编译失败。

- [ ] **Step 3：实现路径模型并接入模块**

实现以下公开契约；所有公开 API 按项目规则补充详细中文 `///`：

```rust
#[derive(Clone, Debug)]
pub struct AppPaths {
    root: std::path::PathBuf,
}

impl AppPaths {
    pub fn new(root: std::path::PathBuf) -> Self { Self { root } }
    pub fn index_file(&self) -> std::path::PathBuf { self.root.join("accounts.json") }
    pub fn migration_marker(&self) -> std::path::PathBuf { self.root.join("migration.json") }
    pub fn legacy_db(&self) -> std::path::PathBuf { self.root.join("im_monitor.db") }
    pub fn account_db(&self, uid: i64) -> Result<std::path::PathBuf, AccountError> {
        if uid <= 0 {
            return Err(AccountError::InvalidUid(uid));
        }
        Ok(self.root.join("accounts").join(uid.to_string()).join("im_monitor.db"))
    }
}
```

在 `account/mod.rs` 定义跨模块错误，后续任务只扩展该枚举，不再创建字符串错误旁路：

```rust
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("invalid account uid: {0}")]
    InvalidUid(i64),
    #[error("account storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("account index decode failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("credential storage is unavailable: {0}")]
    CredentialUnavailable(String),
    #[error("no active account database")]
    NoActiveDatabase,
    #[error("active database belongs to {active}, not {requested}")]
    ActiveUidMismatch { active: i64, requested: i64 },
    #[error("account database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("pending login state is unavailable")]
    MissingPendingLogin,
    #[error("the login password was already reused")]
    PasswordAlreadyReused,
}
```

在根 `Cargo.toml` 增加 `keyring = { version = "=3.6.3", default-features = false }`。在 `im-app/Cargo.toml` 用 target dependencies 分别启用：

```toml
[target.'cfg(target_os = "macos")'.dependencies]
keyring = { workspace = true, features = ["apple-native"] }

[target.'cfg(target_os = "windows")'.dependencies]
keyring = { workspace = true, features = ["windows-native"] }

[target.'cfg(target_os = "linux")'.dependencies]
keyring = { workspace = true, features = ["sync-secret-service", "crypto-rust", "vendored"] }
```

在 `im-app/Cargo.toml` 增加测试临时目录依赖：

```toml
[dev-dependencies]
tempfile = "3"
```

不要升级到 `keyring 4.2.0`：其 MSRV 是 Rust 1.88，而 workspace 固定 Rust 1.75。

`main.rs` 通过 Tauri 2 的 `app.path().home_dir()?` 取得跨平台用户主目录，再追加 `.im-monitor` 构造 `AppPaths`；删除当前目录回退，防止凭据索引或数据库意外写入安装目录。`main.rs` 同时声明 `mod account;`。

- [ ] **Step 4：运行路径测试与依赖检查**

运行：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account_paths_keep_each_uid_in_its_own_directory
cargo check -p im-app
```

预期：路径测试通过，`cargo check` 无错误且不新增 `missing_docs` 警告。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add Cargo.toml Cargo.lock im-app/Cargo.toml im-app/src/main.rs im-app/src/account/mod.rs im-app/src/account/paths.rs
git commit -m "feat: add account path foundation"
```

### Task 2：实现非敏感账号索引

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/index.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`

- [ ] **Step 1：写索引行为失败测试**

```rust
#[tokio::test]
async fn account_index_tracks_last_account_and_logout_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = AccountIndexStore::new(temp.path().join("accounts.json"));
    store.upsert(AccountRecord::new(42, "a@example.com", 4, 100)).await.unwrap();
    store.upsert(AccountRecord::new(84, "13800138000", 3, 200)).await.unwrap();
    store.mark_logged_out(84).await.unwrap();

    let snapshot = store.load().await.unwrap();
    assert_eq!(snapshot.last_used_uid, Some(84));
    assert!(!snapshot.accounts.iter().find(|item| item.uid == 84).unwrap().has_token);
    assert_eq!(snapshot.accounts.len(), 2);
}
```

另写 `remove_preserves_other_accounts_and_selects_most_recent`，断言删除最后账号后按 `last_used_at` 选择剩余账号。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account_index_
```

预期：因索引类型尚不存在而编译失败。

- [ ] **Step 3：实现索引类型和持久化**

使用以下序列化契约，字段名通过 `camelCase` 固定：

```rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub uid: i64,
    pub display_account: String,
    pub login_type: i32,
    pub last_used_at: i64,
    pub has_saved_password: bool,
    pub has_token: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountIndex {
    pub last_used_uid: Option<i64>,
    pub accounts: Vec<AccountRecord>,
}

pub struct AccountIndexStore {
    path: std::path::PathBuf,
    write_lock: tokio::sync::Mutex<()>,
}
```

`load` 在文件不存在时返回默认值；损坏 JSON 返回结构化错误而不是清空。`save` 先写同目录临时文件，`sync_all` 后替换正式文件。`upsert`、`mark_logged_out`、`set_secret_flags`、`remove` 都持有 `write_lock` 完成读改写，避免并发丢更新。

- [ ] **Step 4：运行索引测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account::index
```

预期：账号 CRUD、最后账号和损坏 JSON 测试全部通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/account/index.rs im-app/src/account/mod.rs
git commit -m "feat: persist non-sensitive account index"
```

### Task 3：封装系统凭据库

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/credentials.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`

- [ ] **Step 1：写内存凭据替身失败测试**

```rust
#[tokio::test]
async fn credential_store_keeps_token_and_password_separate() {
    let store = MemoryCredentialStore::default();
    store.set_token(42, "token-a").await.unwrap();
    store.set_password(42, "secret-a").await.unwrap();
    assert_eq!(store.token(42).await.unwrap().as_deref(), Some("token-a"));
    assert_eq!(store.password(42).await.unwrap().as_deref(), Some("secret-a"));

    store.delete_token(42).await.unwrap();
    assert_eq!(store.token(42).await.unwrap(), None);
    assert_eq!(store.password(42).await.unwrap().as_deref(), Some("secret-a"));
}
```

另写不可用替身测试，断言返回 `CredentialUnavailable`，且不会产生文件。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app credential_store_
```

预期：凭据 trait 和内存替身不存在导致编译失败。

- [ ] **Step 3：实现 trait、内存替身和 keyring 实现**

固定接口：

```rust
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    async fn token(&self, uid: i64) -> Result<Option<String>, AccountError>;
    async fn set_token(&self, uid: i64, value: &str) -> Result<(), AccountError>;
    async fn delete_token(&self, uid: i64) -> Result<(), AccountError>;
    async fn password(&self, uid: i64) -> Result<Option<String>, AccountError>;
    async fn set_password(&self, uid: i64, value: &str) -> Result<(), AccountError>;
    async fn delete_password(&self, uid: i64) -> Result<(), AccountError>;
}
```

`KeyringCredentialStore` 使用 service `im-monitor.token` 和 `im-monitor.password`，username 为 UID 十进制字符串。将同步 `keyring::Entry` 调用放入 `tokio::task::spawn_blocking`。`keyring::Error::NoEntry` 映射为 `Ok(None)`；平台服务、权限和 D-Bus 错误统一映射为不含密钥值的 `CredentialUnavailable`。删除不存在条目视为成功。

- [ ] **Step 4：运行测试和静态检查**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account::credentials
cargo clippy -p im-app --all-targets -- -D warnings
```

预期：内存替身测试通过；Clippy 不显示 Token 或密码值。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/account/credentials.rs im-app/src/account/mod.rs
git commit -m "feat: add system credential storage"
```

### Task 4：实现活动账号数据库管理器

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/database.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`

- [ ] **Step 1：写分库失败测试**

```rust
#[tokio::test]
async fn database_manager_switches_between_uid_scoped_databases() {
    let temp = tempfile::tempdir().unwrap();
    let manager = AccountDatabaseManager::new(AppPaths::new(temp.path().to_path_buf()));
    let first = manager.open(42).await.unwrap();
    first.groups.insert_or_update(&group_row(7, "账号一")).await.unwrap();
    manager.close().await;

    let second = manager.open(84).await.unwrap();
    assert!(second.groups.list_all().await.unwrap().is_empty());
    assert_ne!(manager.database_path(42).unwrap(), manager.database_path(84).unwrap());
}
```

另写 `active_database_requires_authenticated_account`，断言未打开时返回 `NoActiveDatabase`。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app database_manager_
```

预期：数据库管理器不存在导致编译失败。

- [ ] **Step 3：实现数据库管理器**

```rust
#[derive(Clone)]
pub struct ActiveDatabase {
    pub uid: i64,
    pub store: std::sync::Arc<im_store::SqliteStore>,
}

pub struct AccountDatabaseManager {
    paths: AppPaths,
    active: tokio::sync::RwLock<Option<ActiveDatabase>>,
    switch_lock: tokio::sync::Mutex<()>,
}
```

`open(uid)` 创建账号目录、打开 `SqliteStore`，仅在成功后替换 `active`。`require(uid)` 同时检查当前 UID，禁止旧会话取得新账号数据库。`close()` 先取走活动句柄，再调用 `pool.close().await`。所有错误包含路径但不包含凭据。

- [ ] **Step 4：运行数据库测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account::database
```

预期：不同 UID 数据互不相见，未认证访问被拒绝。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/account/database.rs im-app/src/account/mod.rs
git commit -m "feat: isolate account sqlite databases"
```

### Task 5：实现旧单库一次性迁移

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/migration.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`

- [ ] **Step 1：写迁移失败测试**

测试必须覆盖：

```rust
#[tokio::test]
async fn legacy_database_is_migrated_once_and_original_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().to_path_buf());
    seed_legacy_database(&paths.legacy_db()).await;
    let migrator = LegacyDatabaseMigrator::new(paths.clone());

    assert_eq!(migrator.migrate_if_needed(42).await.unwrap(), MigrationOutcome::Migrated);
    assert!(paths.legacy_db().exists());
    assert!(paths.account_db(42).unwrap().exists());
    assert!(paths.migration_marker().exists());
    assert_eq!(migrator.migrate_if_needed(84).await.unwrap(), MigrationOutcome::AlreadyHandled);
}
```

另测目标库已存在时不覆盖、复制或校验失败时不写 marker。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app legacy_database_
```

预期：迁移器不存在导致编译失败。

- [ ] **Step 3：实现一致性迁移**

流程固定为：

1. marker 存在时返回 `AlreadyHandled`；
2. 旧库不存在时写入 `NoLegacyDatabase` marker；
3. 目标库存在时写入 `TargetAlreadyExists` marker，不覆盖；
4. 用 SQLx 打开旧库，通过 `VACUUM INTO ?` 写入同目录临时目标；
5. 用 `SqliteStore::new` 打开临时目标并读取 schema，确认可用；
6. 原子移动为目标 `im_monitor.db`；
7. 保留原 `/Users/<user>/.im-monitor/im_monitor.db` 不删除；
8. 最后写入包含 UID、时间和 outcome 的 `migration.json`。

`MigrationMarker` 使用明确枚举，禁止用自由文本推断状态。

- [ ] **Step 4：运行迁移测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account::migration
```

预期：成功、无旧库、目标已存在、失败不落 marker 和重复启动全部通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/account/migration.rs im-app/src/account/mod.rs
git commit -m "feat: migrate legacy account database"
```

### Task 6：让 AppState 使用活动账号数据库

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/state.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/groups.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/chat.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/message_content.rs`

- [ ] **Step 1：写未登录不打开数据库的失败测试**

在 `state.rs` 测试构造新状态并断言：

```rust
#[tokio::test]
async fn app_state_has_no_business_database_before_login() {
    let state = test_state_with_account_foundation().await;
    let error = state.account_db.active().await.unwrap_err();
    assert!(matches!(error, AccountError::NoActiveDatabase));
    assert!(state.monitoring_groups.read().await.is_empty());
}
```

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app app_state_has_no_business_database_before_login
```

预期：现有 `AppState` 仍固定持有 `db`，测试失败。

- [ ] **Step 3：替换 AppState 数据库字段**

将 `pub db: Arc<SqliteStore>` 替换为：

```rust
pub paths: account::AppPaths,
pub account_index: std::sync::Arc<account::AccountIndexStore>,
pub credentials: std::sync::Arc<dyn account::CredentialStore>,
pub account_db: std::sync::Arc<account::AccountDatabaseManager>,
pub legacy_migrator: std::sync::Arc<account::LegacyDatabaseMigrator>,
```

`main.rs` 只创建数据根目录和上述服务，`monitoring_groups` 初始化为空；不再调用 `SqliteStore::new` 或 `load_monitoring_groups`。

- [ ] **Step 4：逐个改造数据库调用点**

已登录业务命令先从当前 `AuthSession.uid` 取得 UID，再调用：

```rust
let session = authenticated_session(&state).await?;
let db = state.account_db.require(session.uid).await?;
```

必须覆盖：

- `auth.rs` 的登录成功路径是唯一例外：远端返回 UID 后先执行旧库迁移并 `account_db.open(uid)`，再把该句柄传给远端群同步；此时尚不能要求已有 `AuthSession`；
- `groups.rs` 的列表、刷新、toggle；
- `chat.rs` 的 `ConnectionContext`、消息查询、附件下载；
- `message_content.rs` 的用户密钥对访问。

切换或登出先取消连接并等待旧任务停止，再调用 `account_db.close()`。`ConnectionContext` 仍持有当次连接的 `Arc<SqliteStore>`，但 generation 失效后不得继续接收或写入。

- [ ] **Step 5：运行针对性与全量 Rust 测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

预期：所有 Rust 测试通过；搜索生产代码不再出现 `state.db`：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
rg "state\\.db|pub db: Arc<SqliteStore>" im-app/src
```

预期：无匹配。

- [ ] **Step 6：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/main.rs im-app/src/state.rs im-app/src/commands/auth.rs im-app/src/commands/groups.rs im-app/src/commands/chat.rs im-app/src/message_content.rs
git commit -m "refactor: route storage through active account"
```

## 完成标准

- `keyring` 保持 Rust 1.75 兼容；
- 未登录时不创建或打开业务 SQLite；
- 两个 UID 使用不同数据库且数据隔离；
- Token 与密码只有凭据接口能读写；
- 旧单库只迁移一次、从不覆盖已有账号库、原文件保留；
- 当前生产代码不再直接访问固定 `state.db`；
- `cargo test --workspace --all-targets` 和 Clippy 通过。
