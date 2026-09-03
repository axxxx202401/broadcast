# Account Authentication Experience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在账号存储基础设施上实现启动恢复、多账号切换、安全保存密码、简化登录页和友好的二次验证。

**Architecture:** Rust 负责凭据读取、Token 校验、账号数据库切换和最终登录持久化；Vue 只持有账号摘要及“使用已保存密码”标志。启动、切换和手动登录最终进入同一个会话发布流程，现有 generation 门禁阻止旧账号异步结果污染新账号。

**Tech Stack:** Rust 2021、Tauri 2、Tokio、Zeroize、Vue 3 Composition API、TypeScript 5、Vitest

**Depends on:** 完成 `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/plans/2026-09-03-account-storage-foundation.md`

---

## 文件结构

- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/pending_login.rs`：短期保存登录密码及账号上下文，不持久化。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/session.rs`：Token 恢复和统一会话发布。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/accounts.rs`：账号列表、恢复、切换和移除 IPC。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAccounts.ts`：账号摘要、启动恢复和切换状态。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/AccountMenu.vue`：头部账号菜单。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`：应用级验证 DTO、保存密码 sentinel、登录成功持久化和退出语义。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/state.rs`：加入待登录秘密缓存。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/mod.rs`：注册账号命令。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/types/im.ts` 和 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts`：账号 IPC 契约。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.ts`：默认邮箱密码、保存密码、挑战流程和移除 `secondMac`。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.vue`：单栏普通用户登录界面。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`：启动恢复和头部账号菜单编排。

### Task 1：建立短期登录秘密缓存

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/Cargo.toml`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/Cargo.toml`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/pending_login.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/mod.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/state.rs`

- [ ] **Step 1：写缓存生命周期失败测试**

```rust
#[tokio::test]
async fn pending_login_secret_moves_to_challenge_token_and_is_taken_once() {
    let cache = PendingLoginCache::default();
    cache.insert(
        "issued",
        PendingLogin {
            display_account: "a@example.com".into(),
            primary_login_type: 4,
            password: Some(zeroize::Zeroizing::new("secret".into())),
            password_reused: false,
        },
    ).await;
    cache.move_token("issued", "challenge").await.unwrap();
    assert!(cache.get("issued").await.is_none());
    assert_eq!(cache.take("challenge").await.unwrap().display_account, "a@example.com");
    assert!(cache.take("challenge").await.is_none());
}
```

另写测试确认 `reuse_password_once` 第二次返回 `PasswordAlreadyReused`，以及超过 10 分钟的记录会清理。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app pending_login_
```

预期：缓存类型不存在导致编译失败。

- [ ] **Step 3：实现缓存并接入 AppState**

根依赖增加 `zeroize = "1"`，`im-app` 引用 workspace 依赖。缓存契约：

```rust
pub struct PendingLogin {
    pub display_account: String,
    pub primary_login_type: i32,
    pub password: Option<zeroize::Zeroizing<String>>,
    pub password_reused: bool,
}

#[derive(Default)]
pub struct PendingLoginCache {
    entries: tokio::sync::Mutex<std::collections::HashMap<String, TimedPendingLogin>>,
}

impl PendingLoginCache {
    pub async fn insert(&self, token: &str, login: PendingLogin);
    pub async fn move_token(&self, old: &str, new: &str) -> Result<(), AccountError>;
    pub async fn reuse_password_once(&self, token: &str) -> Result<zeroize::Zeroizing<String>, AccountError>;
    pub async fn take(&self, token: &str) -> Option<PendingLogin>;
    pub async fn clear(&self);
}
```

每次操作先删除超过 10 分钟的记录。日志只能记录条目数量，不得记录 token key 或密码。`AppState` 增加 `Arc<PendingLoginCache>`。

- [ ] **Step 4：运行测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account::pending_login
cargo clippy -p im-app --all-targets -- -D warnings
```

预期：缓存、单次复用和过期测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add Cargo.toml Cargo.lock im-app/Cargo.toml im-app/src/account/pending_login.rs im-app/src/account/mod.rs im-app/src/state.rs
git commit -m "feat: add transient login secret cache"
```

### Task 2：让验证命令支持已保存密码和当前登录密码

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/types/im.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.test.ts`

- [ ] **Step 1：写 Rust 失败测试**

覆盖三种秘密来源：

```rust
#[tokio::test]
async fn saved_password_is_resolved_in_rust_and_never_returned() {
    let state = test_state_with_password(42, "saved-secret").await;
    let request = VerifyValidationsDto::saved_password(
        "issued-token", 42, "a@example.com", 21,
    );
    verify_validations_inner(&state, request).await.unwrap();
    let pending = state.pending_login.take("issued-token").await.unwrap();
    assert_eq!(pending.display_account, "a@example.com");
    assert!(pending.password.is_some());
}
```

另测：请求同时携带 `validateValue` 和 `savedPasswordUid` 时拒绝；`reuseLoginPassword` 最多成功一次；非密码类型不得使用保存密码。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app saved_password_is_resolved_in_rust
```

预期：应用级 DTO 尚不存在而失败。

- [ ] **Step 3：实现应用级验证 DTO**

不要直接把新字段加入 `im_http::VerifyReq`。在 `auth.rs` 定义：

```rust
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyValidationsDto {
    pub validate_token: String,
    pub pending_validate_dtos: Vec<PendingValidationInputDto>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingValidationInputDto {
    pub country_code: Option<i32>,
    pub account: Option<String>,
    pub account_type: Option<i32>,
    pub validate_type: im_http::openchat_user::ValidateType,
    pub validate_value: Option<String>,
    pub saved_password_uid: Option<String>,
    #[serde(default)]
    pub reuse_login_password: bool,
}
```

每项必须且只能选择一个秘密来源。`saved_password_uid` 经 `parse_i64_id` 后从 `CredentialStore` 读取；`reuse_login_password` 从 `PendingLoginCache` 读取。转换成 `im_http::VerifyReq` 后才调用现有 `hash_verify_passwords`。首次主验证成功后，为验证码和密码登录都缓存显示账号及原始主登录类型；密码模式额外保存 `Zeroizing<String>`，验证码模式的 `password` 为 `None`。后续 challenge 验证只能更新已有上下文的 token 或复用标志，不能用脱敏账号覆盖最初的完整显示账号。响应和日志不得返回密码。

- [ ] **Step 4：更新 TypeScript IPC 契约测试**

```typescript
it('只发送已保存密码标志，不发送密码明文', async () => {
  await api.verifyValidations({
    validateToken: 'issued',
    pendingValidateDTOS: [{
      account: 'a@example.com',
      validateType: 21,
      savedPasswordUid: '42',
    }],
  })
  expect(invoke).toHaveBeenCalledWith('verify_validations', {
    request: expect.objectContaining({
      pendingValidateDTOS: [expect.not.objectContaining({ validateValue: expect.anything() })],
    }),
  })
})
```

- [ ] **Step 5：运行 Rust 与前端测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app verify_validations
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/services/tauri.test.ts
npm run typecheck
```

预期：秘密来源校验和 IPC 测试通过。

- [ ] **Step 6：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/commands/auth.rs im-app/ui/src/types/im.ts im-app/ui/src/services/tauri.ts im-app/ui/src/services/tauri.test.ts
git commit -m "feat: resolve saved login passwords in rust"
```

### Task 3：登录成功后统一保存账号和凭据

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/migration.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/database.rs`

- [ ] **Step 1：写最终成功前不保存的失败测试**

```rust
#[tokio::test]
async fn credentials_are_persisted_only_after_final_login_success() {
    let state = test_state_with_memory_credentials().await;
    seed_pending_password(&state, "issued", "a@example.com", 4, "secret").await;
    let challenge = finish_remote_login_for_test(&state, "issued", RemoteLogin::challenge("next")).await;
    assert!(challenge.is_ok());
    assert_eq!(state.credentials.password(42).await.unwrap(), None);

    finish_remote_login_for_test(&state, "next", RemoteLogin::success(42, "token")).await.unwrap();
    assert_eq!(state.credentials.password(42).await.unwrap().as_deref(), Some("secret"));
    assert_eq!(state.credentials.token(42).await.unwrap().as_deref(), Some("token"));
}
```

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app credentials_are_persisted_only_after_final_login_success
```

预期：登录成功路径尚未接入账号基础设施。

- [ ] **Step 3：实现统一完成函数**

建立单一 `complete_account_login`，顺序固定：

```rust
struct LoginCompletion {
    uid: i64,
    groups: Vec<GroupDto>,
    account: AccountSummaryDto,
    warnings: Vec<String>,
}

async fn complete_account_login(
    state: &AppState,
    generation: u64,
    uid: i64,
    token: zeroize::Zeroizing<String>,
    request_token: &str,
) -> Result<LoginCompletion, AuthCommandError>
```

函数依次：

1. 执行旧库迁移；
2. 打开 UID 数据库；
3. 同步远端群组并恢复监控选择；
4. 通过 generation 门禁发布 `AuthSession`；
5. `take(request_token)` 取得账号上下文；
6. 保存 Token；存在登录密码时保存密码；
7. 写账号索引并设为最后账号；
8. 启动自动连接。

凭据保存失败不撤销已经成功的远端登录，但在 `LoginResultDto::Success` 增加 `warnings: Vec<String>`，只返回“本次无法安全保存登录信息”等普通文案。挑战响应把旧 token 的缓存移动到新 `validateToken`。

- [ ] **Step 4：运行认证回归**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app commands::auth
```

预期：现有 challenge/generation 测试及新增持久化测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/commands/auth.rs im-app/src/account/migration.rs im-app/src/account/database.rs
git commit -m "feat: persist successful account logins"
```

### Task 4：实现恢复、列表、切换、退出和移除账号命令

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/account/session.rs`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/accounts.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/mod.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/commands/auth.rs`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/src/main.rs`

- [ ] **Step 1：写会话恢复三态失败测试**

```rust
#[tokio::test]
async fn rejected_token_requires_login_and_is_deleted() {
    let state = restore_test_state(UserDetailOutcome::BusinessRejected).await;
    let result = restore_uid(&state, 42).await.unwrap();
    assert!(matches!(result, RestoreSessionDto::NeedsLogin { uid, .. } if uid == "42"));
    assert_eq!(state.credentials.token(42).await.unwrap(), None);
}

#[tokio::test]
async fn transport_failure_keeps_token_for_retry() {
    let state = restore_test_state(UserDetailOutcome::TransportFailure).await;
    let result = restore_uid(&state, 42).await.unwrap();
    assert!(matches!(result, RestoreSessionDto::Retryable { .. }));
    assert!(state.credentials.token(42).await.unwrap().is_some());
}
```

另测有效 Token 发布会话并打开正确数据库。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app restore_
```

预期：恢复服务和 DTO 不存在。

- [ ] **Step 3：实现恢复 DTO 和服务**

```rust
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RestoreSessionDto {
    Success { account: AccountSummaryDto, groups: Vec<GroupDto>, warnings: Vec<String> },
    NeedsLogin { uid: String, display_account: String, login_type: i32, has_saved_password: bool },
    NoAccount,
    Retryable { uid: String, message: String },
}
```

`restore_session` 只尝试索引中的最后账号且 `has_token == true`；已退出账号直接返回 `NeedsLogin`。`switch_account` 先 `begin_auth_transition`、清消息加密状态和待登录缓存、关闭旧库，再恢复目标 UID。`list_accounts` 只返回摘要。

- [ ] **Step 4：实现退出和移除语义**

`logout` 执行顺序：

1. 读取当前 UID；
2. 先把索引 `has_token` 设为 false；
3. 清运行时会话、连接、数据库和待登录缓存；
4. 删除 keyring Token；
5. 删除失败时返回非敏感 warning，但后续启动不得自动使用残留 Token。

`remove_account` 必须删除索引、Token 和密码，但不得调用 `remove_dir_all` 或删除账号 SQLite。

- [ ] **Step 5：注册命令并运行测试**

注册 `restore_session`、`list_accounts`、`switch_account`、`remove_account`。运行：

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo test -p im-app account::session
cargo test -p im-app commands::accounts
cargo test -p im-app commands::auth
```

预期：恢复三态、切换 generation、退出保留密码、移除保留 SQLite 全部通过。

- [ ] **Step 6：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/src/account/session.rs im-app/src/commands/accounts.rs im-app/src/commands/mod.rs im-app/src/commands/auth.rs im-app/src/main.rs
git commit -m "feat: add account session commands"
```

### Task 5：接入前端账号 IPC 和启动恢复

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/types/im.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/services/tauri.test.ts`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAccounts.ts`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAccounts.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.test.ts`

- [ ] **Step 1：写启动不闪登录页失败测试**

```typescript
it('恢复完成前只显示启动状态', async () => {
  const deferred = promiseWithResolvers<RestoreSessionResult>()
  api.restoreSession.mockReturnValueOnce(deferred.promise)
  const wrapper = mount(App)
  expect(wrapper.text()).toContain('正在恢复上次登录')
  expect(wrapper.findComponent(LoginPanel).exists()).toBe(false)
  deferred.resolve({ status: 'noAccount' })
  await flushPromises()
  expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
})
```

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/App.test.ts src/services/tauri.test.ts
```

预期：`restoreSession` 和恢复状态尚不存在。

- [ ] **Step 3：实现 TypeScript 契约与 useAccounts**

```typescript
export interface AccountSummary {
  uid: string
  displayAccount: string
  loginType: PrimaryLoginType
  hasSavedPassword: boolean
  isCurrent: boolean
}

export type RestoreSessionResult =
  | { status: 'success'; account: AccountSummary; groups: GroupDto[]; warnings: string[] }
  | { status: 'needsLogin'; uid: string; displayAccount: string; loginType: PrimaryLoginType; hasSavedPassword: boolean }
  | { status: 'noAccount' }
  | { status: 'retryable'; uid: string; message: string }
```

`useAccounts` 暴露 `phase: 'recovering' | 'ready' | 'needsLogin'`、`accounts`、`selectedAccount`、`restore`、`switchAccount`、`removeAccount`。所有异步动作使用 operation token，忽略旧切换结果。

- [ ] **Step 4：接入 App 启动状态**

`App.vue` 在挂载时调用 `restore`。成功时调用现有 `monitor.acceptLogin` 并设置当前 `AccountSummary`；需要登录时把账号摘要交给 `useAuth.selectSavedAccount`；无账号时设置邮箱密码默认表单。恢复失败提供“重试”和“使用其他账号”，不显示内部错误。同步把 `useAuth` 的成功回调扩展为 `{ account, groups, warnings }`，确保手动登录后头部立即得到邮箱或手机号，而不是再次猜测账号。

- [ ] **Step 5：运行测试与类型检查**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useAccounts.test.ts src/App.test.ts src/services/tauri.test.ts
npm run typecheck
```

预期：恢复期间不闪现登录页，所有联合类型分支完整处理。

- [ ] **Step 6：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/types/im.ts im-app/ui/src/services/tauri.ts im-app/ui/src/services/tauri.test.ts im-app/ui/src/composables/useAccounts.ts im-app/ui/src/composables/useAccounts.test.ts im-app/ui/src/App.vue im-app/ui/src/App.test.ts
git commit -m "feat: restore saved account on startup"
```

### Task 6：简化主登录页并实现密码 sentinel

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/types/im.ts`

- [ ] **Step 1：写默认模式和 sentinel 失败测试**

```typescript
it('默认使用邮箱密码并折叠其他方式', () => {
  const { auth, wrapper } = setupLogin()
  expect(auth.loginMethod.value).toBe(4)
  expect(wrapper.get('input[type="email"]').exists()).toBe(true)
  expect(wrapper.text()).toContain('其他登录方式')
  expect(wrapper.text()).not.toContain('手机号验证码')
})

it('选择保存账号时不把密码明文放入前端', async () => {
  const { auth, backend } = setupAuth()
  auth.selectSavedAccount({ uid: '42', displayAccount: 'a@example.com', loginType: 4, hasSavedPassword: true, isCurrent: false })
  expect(auth.passwordMode.value).toBe('saved')
  await auth.submitLogin()
  expect(backend.verifyValidations).toHaveBeenCalledWith(expect.objectContaining({
    pendingValidateDTOS: [expect.objectContaining({ savedPasswordUid: '42' })],
  }))
})
```

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useAuth.test.ts src/components/LoginPanel.test.ts
```

预期：当前默认手机验证码且页面平铺四种方式。

- [ ] **Step 3：实现认证状态**

`loginMethod` 默认 `4`。增加：

```typescript
const selectedAccountUid = ref<string | null>(null)
const passwordMode = ref<'empty' | 'saved' | 'manual'>('empty')
const otherMethodsOpen = ref(false)

function selectSavedAccount(account: AccountSummary) {
  selectedAccountUid.value = account.uid
  loginMethod.value = account.loginType
  formAccount.value = account.displayAccount
  validateValue.value = ''
  passwordMode.value = account.hasSavedPassword ? 'saved' : 'empty'
}
```

用户输入密码时切换到 `manual`；提交保存模式时发送 `savedPasswordUid`，不构造 `validateValue`。移除 `secondMac` ref、TS 字段和所有请求展开。

- [ ] **Step 4：重写登录面板主状态**

删除双栏介绍、协议步骤、`LOCAL IPC` 和 `secondMac`。主卡片只显示 Logo、账号选择、邮箱、密码状态、登录按钮和“其他登录方式”。其他方式展开后显示其必要字段；GT4 只在发送验证码时出现。

- [ ] **Step 5：运行测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useAuth.test.ts src/components/LoginPanel.test.ts
npm run typecheck
```

预期：默认邮箱密码、折叠、sentinel 和无 `secondMac` 测试通过。

- [ ] **Step 6：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/composables/useAuth.ts im-app/ui/src/composables/useAuth.test.ts im-app/ui/src/components/LoginPanel.vue im-app/ui/src/components/LoginPanel.test.ts im-app/ui/src/types/im.ts
git commit -m "feat: simplify saved account login"
```

### Task 7：优化二次验证交互

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useAuth.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/LoginPanel.test.ts`

- [ ] **Step 1：写普通用户挑战流程失败测试**

测试必须断言：

```typescript
it('二次验证隐藏协议字段并允许返回登录', async () => {
  const { auth, wrapper } = await enterEmailCodeChallenge()
  expect(wrapper.text()).toContain('还差一步，请确认是你本人')
  expect(wrapper.text()).not.toContain('validateToken')
  expect(wrapper.text()).not.toContain('ValidateType')
  await wrapper.get('[data-test="challenge-back"]').trigger('click')
  expect(auth.challengePending.value).toEqual([])
  expect(auth.validateToken.value).toBe('')
})
```

另测单一方式直接展示、多方式选择、发送后 60 秒倒计时、同一登录密码自动复用一次、脱敏目标补全前后缀校验、连续挑战显示“安全验证第 2 步”。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useAuth.test.ts src/components/LoginPanel.test.ts
```

预期：当前页面仍显示 challenge token、ValidateType 和 GT4 状态。

- [ ] **Step 3：实现挑战状态和倒计时**

增加 `challengeStep`、`resendSeconds`、`completedChallengeKeys`、`supplementedTarget`。使用单个 interval，每秒减一，组件卸载时清除。`resetChallenge()` 必须清空 token、pending、临时值、补全目标、倒计时和自动复用标志。

进入 ValidateType 20/21 时调用一次 `reuseLoginPassword: true`；后端返回 `PasswordAlreadyReused` 后显示输入框，不自动重试。ValidateType 16/17 只有用户点击“发送验证码”时才启动 GT4。

- [ ] **Step 4：实现挑战卡片**

只显示：

- “还差一步，请确认是你本人”；
- “安全验证第 N 步”；
- 用户可理解的验证方式；
- 脱敏目标、发送按钮、倒计时和输入框；
- “改用其他验证方式”和“返回登录”。

服务端只给脱敏目标时，要求补充完整邮箱或手机号；在发送前按可见前后缀校验，补充值不写入账号索引。

- [ ] **Step 5：运行测试和虚拟定时器检查**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useAuth.test.ts src/components/LoginPanel.test.ts
npm run typecheck
```

预期：挑战流程测试通过，Vitest 不报告未清理定时器。

- [ ] **Step 6：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/composables/useAuth.ts im-app/ui/src/composables/useAuth.test.ts im-app/ui/src/components/LoginPanel.vue im-app/ui/src/components/LoginPanel.test.ts
git commit -m "feat: streamline secondary verification"
```

### Task 8：增加头部账号菜单

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/AccountMenu.vue`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/AccountMenu.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.css`

- [ ] **Step 1：写菜单失败测试**

```typescript
it('显示当前账号并阻止重复切换', async () => {
  const switchAccount = vi.fn(() => new Promise(() => {}))
  const wrapper = mount(AccountMenu, {
    props: { current: account42, accounts: [account42, account84], switching: false, switchAccount },
  })
  expect(wrapper.text()).toContain('a@example.com')
  await wrapper.get('[data-test="account-84"]').trigger('click')
  expect(switchAccount).toHaveBeenCalledWith('84')
  await wrapper.setProps({ switching: true })
  expect(wrapper.get('[data-test="account-84"]').attributes('disabled')).toBeDefined()
})
```

另测退出仅回登录页、移除当前账号后选择最近账号、移除操作有确认。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/components/AccountMenu.test.ts
```

预期：组件不存在。

- [ ] **Step 3：实现并接线**

菜单主按钮显示邮箱或手机号。展开项包含其他账号、“添加账号”“退出登录”“移除此账号”。UID 只在详情中显示为“用户 ID”。切换时显示“正在切换账号”并禁用所有账号动作。

`App.vue` 删除 `UID / ...`，由 `AccountMenu` 取代；添加账号进入空白邮箱密码登录页，不清除其他账号。

- [ ] **Step 4：运行前端测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/components/AccountMenu.test.ts src/App.test.ts src/composables/useAccounts.test.ts
npm run typecheck
```

预期：菜单和根编排测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/components/AccountMenu.vue im-app/ui/src/components/AccountMenu.test.ts im-app/ui/src/App.vue im-app/ui/src/styles/console.css
git commit -m "feat: add account switcher"
```

### Task 9：执行认证全量回归

**Files:**
- Modify when required by failures: 本计划中已列出的认证、账号和测试文件

- [ ] **Step 1：运行格式化、静态检查和全量测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npm test
npm run typecheck
```

预期：全部通过；若现有 `MessagePanel.test.ts` 锚点测试仍超时，先确认它与本计划改动无关，不得通过删除测试掩盖。

- [ ] **Step 2：检查敏感信息和已删除字段**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
rg "secondMac|second_mac" im-app/ui/src
rg "validateToken|ValidateType|LOCAL IPC|GT4 READY" im-app/ui/src/components
```

预期：第一条无匹配；第二条只允许测试中的“不得显示”断言，不允许生产模板出现。

- [ ] **Step 3：提交回归修正**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app im-http
git commit -m "test: cover multi-account authentication"
```

如果没有需要修正的文件，跳过本次提交，不创建空提交。

## 完成标准

- 启动期间不闪现登录页；
- 有效 Token 自动恢复，业务拒绝清 Token，传输失败保留 Token；
- 前端永远拿不到已保存密码明文；
- 密码只在最终成功后保存，二次验证前不落盘；
- 多账号切换清理旧连接、旧数据库和旧解密状态；
- 退出保留账号和密码，移除账号保留 SQLite；
- 登录页默认邮箱密码，其他方式折叠，无 `secondMac`；
- 二次验证不显示协议字段，并支持倒计时、返回、目标补全及一次密码复用；
- 头部显示当前邮箱或手机号；
- Rust、前端测试和类型检查全部通过。
