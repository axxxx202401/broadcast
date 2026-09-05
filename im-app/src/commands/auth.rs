//! Tauri 认证命令及登录会话切换流程。
//!
//! 本模块负责把验证码、校验令牌、登录和登出等 HTTP 能力暴露给前端，并在登录成功后
//! 同步群组、发布本地认证会话，再启动后台 TCP 自动连接。登录发布路径使用 generation
//! 拒绝过期流程，但远程请求、数据库写入和内存发布并不是一个跨资源的原子事务；独立
//! 群组刷新也不受认证 generation 保护。

use std::{collections::HashSet, future::Future, sync::Arc};

use tauri::State;

use crate::commands::chat::{cancel_auth_and_disconnect, publish_disconnected_status_if_current};
use crate::state::{AppState, AuthSession};

/// 完成登录发布所需的本地状态引用。
struct LoginStateRefs<'a> {
    /// 串行化群组数据库和监控集合的配套更新，不提供认证代际隔离。
    group_ops: &'a tokio::sync::Mutex<()>,
    /// 校验 generation 并协调认证会话切换。
    connection_coordinator: &'a crate::state::ConnectionCoordinator,
    /// 当前已发布、可供连接流程读取的认证会话。
    auth_session: &'a tokio::sync::RwLock<Option<AuthSession>>,
    /// 当前内存中的受监控群组快照。
    monitoring_groups: &'a tokio::sync::RwLock<HashSet<i64>>,
}

/// Tauri 认证命令返回给前端的结构化错误。
#[derive(Debug, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuthCommandError {
    /// 服务端返回的业务错误，保留协议提供的字段供前端展示或分支处理。
    Business {
        /// 服务端业务码；本层不解释其官方业务含义。
        code: i32,
        /// 服务端业务错误消息。
        msg: String,
        /// 可选的业务错误附加数据。
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        /// 可选的服务端展示方式标记。
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<i32>,
        /// 可选的服务端展示标题。
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// 可选的消息模板参数。
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Vec<String>>,
    },
    /// 传输、解析、本地校验或状态切换等非业务错误。
    Other {
        /// 可供前端记录或展示的错误文本。
        message: String,
    },
}

impl From<im_http::openchat_user::OpenChatUserError> for AuthCommandError {
    fn from(error: im_http::openchat_user::OpenChatUserError) -> Self {
        match error {
            im_http::openchat_user::OpenChatUserError::Business(error) => Self::Business {
                code: error.code,
                msg: error.msg,
                data: error.data,
                display: error.display,
                title: error.title,
                params: error.params,
            },
            error => Self::Other {
                message: error.to_string(),
            },
        }
    }
}

impl From<String> for AuthCommandError {
    fn from(message: String) -> Self {
        Self::Other { message }
    }
}

impl From<&str> for AuthCommandError {
    fn from(message: &str) -> Self {
        Self::Other {
            message: message.to_string(),
        }
    }
}

impl From<crate::account::AccountError> for AuthCommandError {
    fn from(error: crate::account::AccountError) -> Self {
        Self::Other {
            message: error.to_string(),
        }
    }
}

/// 前端可见的账号摘要，不含 Token 或密码。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummaryDto {
    /// 十进制字符串形式的用户 ID，避免跨 Tauri 边界时丢失整数精度。
    pub uid: String,
    /// 用户输入的邮箱或手机号，仅用于展示和回填。
    pub display_account: String,
    /// 首次主登录使用的登录方式标识。
    pub login_type: i32,
    /// 系统凭据库是否已保存该账号登录密码；摘要本身不存密码。
    pub has_saved_password: bool,
    /// 该账号是否为当前已发布会话对应的账号。
    pub is_current: bool,
}

/// 登录命令返回给前端的结果。
#[derive(Debug, serde::Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LoginResultDto {
    /// 认证、群组同步及会话发布均完成。
    Success {
        /// 十进制字符串形式的用户 ID，避免跨 Tauri 边界时丢失整数精度。
        uid: String,
        /// 本次远程快照同步后得到的本地群组列表。
        groups: Vec<crate::commands::groups::GroupDto>,
        /// 当前账号摘要，供前端展示而不回传密钥。
        account: AccountSummaryDto,
        /// 非阻塞提示；凭据保存失败时只放普通用户文案。
        warnings: Vec<String>,
    },
    /// 服务端要求继续完成校验，尚未发布本地认证会话。
    Challenge {
        /// 原样保留的服务端业务码。
        code: i32,
        /// 后续校验请求使用的令牌。
        validate_token: String,
        /// 服务端返回的提示消息。
        message: String,
        /// 服务端可选的待完成校验项。
        #[serde(skip_serializing_if = "Option::is_none")]
        pending: Option<Vec<im_http::openchat_user::ValidateModelVo>>,
    },
}

/// 将远程登录响应归一化为可发布会话或待继续校验两种内部结果。
#[derive(Debug, PartialEq)]
enum RemoteLogin {
    /// 远程认证成功；响应缺少 uid 时允许后续通过用户详情补取。
    Success { uid: Option<i64>, token: String },
    /// 远程认证要求完成额外校验。
    Challenge(LoginChallenge),
}

impl RemoteLogin {
    /// 构造已带 uid 的成功结果，供测试注入而不发起真实 HTTP。
    #[cfg(test)]
    fn success(uid: i64, token: impl Into<String>) -> Self {
        Self::Success {
            uid: Some(uid),
            token: token.into(),
        }
    }

    /// 构造仅包含下一挑战令牌的校验结果，供测试注入。
    #[cfg(test)]
    fn challenge(next_token: impl Into<String>) -> Self {
        Self::Challenge(LoginChallenge {
            code: 3114179,
            validate_token: next_token.into(),
            message: "secondary validation required".to_string(),
            pending: None,
        })
    }
}

/// 凭据或账号索引无法安全写入时返回给前端的普通文案。
pub(crate) const CREDENTIAL_SAVE_WARNING: &str = "本次无法安全保存登录信息";

/// 凭据删除失败时返回给前端的普通文案；不得包含 Token 或密码。
pub(crate) const CREDENTIAL_CLEAR_WARNING: &str = "本次无法完全清除登录信息";

/// 退出时索引未能确认已退出，前端不得把本次操作当成干净退出。
pub(crate) const CREDENTIAL_LOGOUT_UNCONFIRMED: &str = "本次无法确认已退出，请重试";

/// 退出登录命令返回给前端的结果。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoutResultDto {
    /// 非阻塞提示；删除 Token 失败时只放普通用户文案。
    pub warnings: Vec<String>,
}

/// 统一登录收尾完成后的内部结果。
struct LoginCompletion {
    uid: i64,
    groups: Vec<crate::commands::groups::GroupDto>,
    account: AccountSummaryDto,
    warnings: Vec<String>,
}

/// 读取请求中的非空 `validateToken`；缺失或空白时返回 `None`。
fn request_validate_token(request: &im_http::openchat_user::LoginReq) -> Option<&str> {
    request
        .validate_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

/// 从登录请求回退显示账号：优先邮箱，其次手机号。
fn display_account_from_request(request: &im_http::openchat_user::LoginReq) -> String {
    request
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            request
                .phone
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// 读取索引或凭据库中是否已有保存密码，供验证码重登或本次写密码失败时保留标志。
async fn existing_saved_password(state: &AppState, uid: i64) -> bool {
    if state
        .account_index
        .has_saved_password(uid)
        .await
        .unwrap_or(false)
    {
        return true;
    }
    matches!(state.credentials.password(uid).await, Ok(Some(_)))
}

/// 确认本次登录或恢复的 generation 仍持有当前会话，否则不得继续落盘或返回 Success。
///
/// 同时检查协调器代际与 `auth_session` 的 uid/generation，避免过期恢复覆盖较新账号的
/// `last_used` 或启动自动连接。过期时只返回错误，不得撤销已经发布的较新会话。
pub(crate) async fn ensure_login_generation_current(
    state: &AppState,
    generation: u64,
    uid: i64,
) -> Result<(), AuthCommandError> {
    if !state
        .connection_coordinator
        .is_generation_current(generation)
        .await
    {
        return Err("Connection generation changed before credential persistence".into());
    }
    let session = state.auth_session.read().await;
    match session.as_ref() {
        Some(session) if session.generation == generation && session.uid == uid => Ok(()),
        _ => Err("Authentication session is no longer current".into()),
    }
}

/// 处理已分类的远程登录结果：挑战只迁移待登录缓存，成功才完成本地收尾。
///
/// `remote_groups` 为 `Some` 时跳过远端群组拉取，供测试注入空快照。
/// 挑战路径不得写入 Token 或密码；请求没有 `validateToken` 时跳过缓存迁移。
/// 成功路径在缺少 uid 时通过用户详情补取，随后调用 [`complete_account_login`]。
async fn handle_remote_login_result(
    state: &AppState,
    generation: u64,
    request: &im_http::openchat_user::LoginReq,
    remote_login: RemoteLogin,
    remote_groups: Option<Vec<im_store::group::GroupRow>>,
) -> Result<LoginResultDto, AuthCommandError> {
    match remote_login {
        RemoteLogin::Challenge(challenge) => {
            if let Some(old_token) = request_validate_token(request) {
                if let Err(error) = state
                    .pending_login
                    .move_token(old_token, &challenge.validate_token)
                    .await
                {
                    tracing::debug!(error = %error, "pending login cache move skipped");
                }
            }
            Ok(LoginResultDto::Challenge {
                code: challenge.code,
                validate_token: challenge.validate_token,
                message: challenge.message,
                pending: challenge.pending,
            })
        }
        RemoteLogin::Success { uid, token } => {
            let token = zeroize::Zeroizing::new(token);
            let uid = match uid {
                Some(uid) => uid,
                None => state
                    .http
                    .openchat_user
                    .user_detail(&token)
                    .await?
                    .user_base
                    .uid
                    .ok_or("User detail response missing userBase.uid")?,
            };
            let remote_groups = match remote_groups {
                Some(groups) => groups,
                None => crate::commands::groups::fetch_remote_groups(state, &token).await?,
            };
            let completion =
                complete_account_login(state, generation, uid, token, request, remote_groups)
                    .await?;
            Ok(LoginResultDto::Success {
                uid: completion.uid.to_string(),
                groups: completion.groups,
                account: completion.account,
                warnings: completion.warnings,
            })
        }
    }
}

/// 在远端登录已成功后，按固定顺序完成本地账号收尾。
///
/// 顺序为：旧库迁移、打开 UID 数据库、同步远端群组并恢复监控、经 generation
/// 门禁发布 `AuthSession`、取出待登录上下文、保存 Token（存在登录密码时再保存密码）、
/// 写入账号索引并更新秘密存在标志、启动自动连接。
///
/// 凭据或账号索引写入失败不会撤销已经成功的远端登录，只向 `warnings` 追加
/// 「本次无法安全保存登录信息」。迁移和打开账号库之前先用协调器代际判断是否已过期，
/// 避免过期登录替换另一 UID 的活动库。发布会话后、写入凭据前以及返回 Success 前都会
/// 复核 generation 与会话归属；代际已变化时不落盘、不自动连接，并返回错误而不是 Success。
/// 本次没有登录密码，或写密码失败但索引/凭据库仍有密码时，保留 `has_saved_password`。
/// 打开数据库之后的失败仅在活动 UID 与 generation 仍属于本次打开时关闭数据库。
/// 测试状态没有 Tauri 句柄时跳过自动连接，避免后台任务访问空句柄。
async fn complete_account_login(
    state: &AppState,
    generation: u64,
    uid: i64,
    token: zeroize::Zeroizing<String>,
    request: &im_http::openchat_user::LoginReq,
    remote_groups: Vec<im_store::group::GroupRow>,
) -> Result<LoginCompletion, AuthCommandError> {
    run_complete_account_login(
        state,
        generation,
        uid,
        token,
        request,
        remote_groups,
        async {},
    )
    .await
}

/// 与 [`complete_account_login`] 相同，但在发布会话后、写入凭据前执行 `after_publish`。
///
/// 仅供测试插入较新会话，验证过期登录不得继续落盘。
#[cfg(test)]
async fn complete_account_login_after_publish(
    state: &AppState,
    generation: u64,
    uid: i64,
    token: zeroize::Zeroizing<String>,
    request: &im_http::openchat_user::LoginReq,
    remote_groups: Vec<im_store::group::GroupRow>,
    after_publish: impl Future<Output = ()>,
) -> Result<LoginCompletion, AuthCommandError> {
    run_complete_account_login(
        state,
        generation,
        uid,
        token,
        request,
        remote_groups,
        after_publish,
    )
    .await
}

/// 实际执行登录收尾；`after_publish` 在会话发布成功后、凭据落盘前运行。
async fn run_complete_account_login(
    state: &AppState,
    generation: u64,
    uid: i64,
    token: zeroize::Zeroizing<String>,
    request: &im_http::openchat_user::LoginReq,
    remote_groups: Vec<im_store::group::GroupRow>,
    after_publish: impl Future<Output = ()>,
) -> Result<LoginCompletion, AuthCommandError> {
    // 打库前只看协调器代际：此时会话常常仍是 None，不能用发布后的会话门禁。
    if !state
        .connection_coordinator
        .is_generation_current(generation)
        .await
    {
        return Err("Connection generation changed before opening account database".into());
    }
    state.legacy_migrator.migrate_if_needed(uid).await?;
    let db = state.account_db.open(uid, generation).await?;
    let groups =
        finish_login_after_opening_account(state, generation, uid, token.to_string(), async {
            crate::commands::groups::apply_remote_groups(&db, &remote_groups).await
        })
        .await?;
    after_publish.await;
    ensure_login_generation_current(state, generation, uid).await?;
    // 后台清理超过保留窗口的旧消息；不阻塞登录响应。
    let cleanup_db = db.clone();
    tokio::spawn(async move {
        let cutoff = chrono::Utc::now()
            .timestamp_millis()
            .saturating_sub(im_store::message::MESSAGE_RETENTION_DAYS as i64 * 24 * 3600 * 1000);
        match cleanup_db.messages.cleanup_old_messages(cutoff).await {
            Ok(n) => tracing::info!(deleted = n, "message retention cleanup completed"),
            Err(e) => tracing::warn!(error = %e, "message retention cleanup failed"),
        }
    });

    let pending = match request_validate_token(request) {
        Some(request_token) => state.pending_login.take(request_token).await,
        None => None,
    };
    let display_account = pending
        .as_ref()
        .map(|pending| pending.display_account.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| display_account_from_request(request));
    let login_type = pending
        .as_ref()
        .map(|pending| pending.primary_login_type)
        .unwrap_or(request.login_type as i32);
    let pending_password = pending.and_then(|pending| pending.password);

    let mut warnings = Vec::new();
    let mut has_token = false;
    match state.credentials.set_token(uid, &token).await {
        Ok(()) => has_token = true,
        Err(error) => {
            tracing::warn!(error = %error, uid, "failed to persist login token");
            warnings.push(CREDENTIAL_SAVE_WARNING.to_string());
        }
    }

    let password_saved_this_attempt = if let Some(password) = pending_password {
        match state.credentials.set_password(uid, &password).await {
            Ok(()) => Some(true),
            Err(error) => {
                tracing::warn!(error = %error, uid, "failed to persist login password");
                if warnings.is_empty() {
                    warnings.push(CREDENTIAL_SAVE_WARNING.to_string());
                }
                Some(false)
            }
        }
    } else {
        None
    };
    let has_saved_password = match password_saved_this_attempt {
        Some(true) => true,
        Some(false) | None => existing_saved_password(state, uid).await,
    };

    let last_used_at = chrono::Utc::now().timestamp_millis();
    if let Err(error) = state
        .account_index
        .upsert(crate::account::index::AccountRecord::new(
            uid,
            display_account.clone(),
            login_type,
            last_used_at,
        ))
        .await
    {
        tracing::warn!(error = %error, uid, "failed to upsert account index");
        if warnings.is_empty() {
            warnings.push(CREDENTIAL_SAVE_WARNING.to_string());
        }
    } else if let Err(error) = state
        .account_index
        .set_secret_flags(uid, has_saved_password, has_token)
        .await
    {
        tracing::warn!(error = %error, uid, "failed to set account secret flags");
        if warnings.is_empty() {
            warnings.push(CREDENTIAL_SAVE_WARNING.to_string());
        }
    }

    ensure_login_generation_current(state, generation, uid).await?;
    if state.app_handle.is_some() {
        crate::commands::chat::start_automatic_connection(state, generation);
    }

    Ok(LoginCompletion {
        uid,
        groups: groups
            .into_iter()
            .map(crate::commands::groups::GroupDto::from)
            .collect(),
        account: AccountSummaryDto {
            uid: uid.to_string(),
            display_account,
            login_type,
            has_saved_password,
            is_current: true,
        },
        warnings,
    })
}

/// 从业务错误中提取的登录校验挑战。
#[derive(Debug, PartialEq)]
struct LoginChallenge {
    code: i32,
    validate_token: String,
    message: String,
    pending: Option<Vec<im_http::openchat_user::ValidateModelVo>>,
}

/// 兼容解析登录挑战附加数据的内部结构。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginChallengeData {
    validate_token: Option<String>,
    /// 优先读取协议字段 `validateModelVOS`，并兼容旧别名 `pending`。
    #[serde(default, rename = "validateModelVOS", alias = "pending")]
    pending: Option<Vec<im_http::openchat_user::ValidateModelVo>>,
}

/// 解释远程登录结果。
///
/// 仅业务码 `3114179` 在本客户端中进入挑战分支；这里不扩写该业务码的官方含义。
/// 成功响应优先读取嵌套 `authorization` 中由 `access_token`、`accessToken` 或
/// `token` 映射的值，缺失时再读取顶层同名兼容字段；缺失或空白 token 均视为错误。
/// 挑战数据同时兼容 `validateModelVOS` 与 `pending`，且要求 `validateToken` 非空；
/// 其他错误保持为 [`AuthCommandError`]。
fn classify_remote_login(
    result: Result<im_http::openchat_user::LoginData, im_http::openchat_user::OpenChatUserError>,
) -> Result<RemoteLogin, AuthCommandError> {
    match result {
        Ok(data) => {
            let token = data
                .access_token()
                .filter(|token| !token.trim().is_empty())
                .ok_or("Login response missing authorization token")?
                .to_string();
            Ok(RemoteLogin::Success {
                uid: data.uid,
                token,
            })
        }
        Err(im_http::openchat_user::OpenChatUserError::Business(error))
            if error.code == 3114179 =>
        {
            let data: LoginChallengeData =
                serde_json::from_value(error.data.ok_or("Login challenge missing response data")?)
                    .map_err(|decode| format!("Invalid login challenge data: {decode}"))?;
            let validate_token = data
                .validate_token
                .filter(|token| !token.trim().is_empty())
                .ok_or("Login challenge missing validateToken")?;
            Ok(RemoteLogin::Challenge(LoginChallenge {
                code: error.code,
                validate_token,
                message: error.msg,
                pending: data.pending,
            }))
        }
        Err(error) => Err(error.into()),
    }
}

/// 请求向手机号发送认证验证码。
///
/// `request` 携带 HTTP 接口所需的账号、区号及场景等参数；成功返回空值，失败返回
/// [`AuthCommandError`]。该命令会触发远程短信发送，不修改本地认证会话；请求发出后的
/// 网络或响应解析错误不证明短信未发送。
#[tauri::command]
pub async fn send_sms_code(
    state: State<'_, AppState>,
    request: im_http::openchat_user::SendSmsCodeReq,
) -> Result<(), AuthCommandError> {
    state
        .http
        .openchat_user
        .send_sms_code(&request)
        .await
        .map_err(AuthCommandError::from)
}

/// 请求向邮箱发送认证验证码。
///
/// `request` 携带 HTTP 接口所需的邮箱及场景等参数；成功返回空值，失败返回
/// [`AuthCommandError`]。该命令会触发远程邮件发送，不修改本地认证会话；请求发出后的
/// 网络或响应解析错误不证明邮件未发送。
#[tauri::command]
pub async fn send_email_code(
    state: State<'_, AppState>,
    request: im_http::openchat_user::SendEmailCodeReq,
) -> Result<(), AuthCommandError> {
    state
        .http
        .openchat_user
        .send_email_code(&request)
        .await
        .map_err(AuthCommandError::from)
}

/// 根据已收集的校验信息向服务端签发校验令牌。
///
/// `request` 原样传给远程签发接口；返回服务端的签发结果，业务错误保留为
/// [`AuthCommandError::Business`]。该命令会创建或推进远程校验流程，不发布本地会话；
/// 请求发出后的网络或响应解析错误不证明服务端未签发令牌。
#[tauri::command]
pub async fn issue_validation_token(
    state: State<'_, AppState>,
    request: im_http::openchat_user::IssuedReq,
) -> Result<im_http::openchat_user::IssuedResp, AuthCommandError> {
    state
        .http
        .openchat_user
        .issued(&request)
        .await
        .map_err(AuthCommandError::from)
}

/// 前端提交给 `verify_validations` 的应用级校验请求。
///
/// 与 [`im_http::openchat_user::VerifyReq`] 分离，以便在进入协议层之前解析已保存密码
/// 或复用本次登录密码。JSON 字段名与既有前端契约一致：`pendingValidateDTOS`。
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyValidationsDto {
    /// 标识本轮校验流程的令牌，同时作为待登录缓存的键。
    pub validate_token: String,
    /// 至少一项待验证材料；JSON 名固定为 `pendingValidateDTOS`。
    #[serde(rename = "pendingValidateDTOS")]
    pub pending_validate_dtos: Vec<PendingValidationInputDto>,
}

/// 单项校验材料，必须且只能选择一种秘密来源。
///
/// 三种来源为：手输 `validateValue`、按 UID 读取的 `savedPasswordUid`、以及从
/// [`crate::account::PendingLoginCache`] 复用一次的 `reuseLoginPassword`。
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingValidationInputDto {
    /// 手机流程携带的可选国家或地区代码。
    pub country_code: Option<i32>,
    /// 与本项关联的账号；首次主验证用它缓存完整显示账号。
    pub account: Option<String>,
    /// 服务端用于区分账号类别的可选整数。
    pub account_type: Option<i32>,
    /// 指明服务端应如何解释解析后的 `validateValue`。
    pub validate_type: im_http::openchat_user::ValidateType,
    /// 用户本次输入的验证码或密码；与另外两种秘密来源互斥。
    pub validate_value: Option<String>,
    /// 从系统凭据库读取已保存登录密码的 UID 十进制字符串。
    pub saved_password_uid: Option<String>,
    /// 为 `true` 时从待登录缓存复用一次本次登录密码。
    #[serde(default)]
    pub reuse_login_password: bool,
}

impl VerifyValidationsDto {
    /// 构造仅携带已保存密码 UID 的测试请求。
    #[cfg(test)]
    pub fn saved_password(
        validate_token: impl Into<String>,
        uid: i64,
        account: impl Into<String>,
        validate_type: i32,
    ) -> Self {
        Self {
            validate_token: validate_token.into(),
            pending_validate_dtos: vec![PendingValidationInputDto {
                country_code: None,
                account: Some(account.into()),
                account_type: None,
                validate_type: test_validate_type(validate_type),
                validate_value: None,
                saved_password_uid: Some(uid.to_string()),
                reuse_login_password: false,
            }],
        }
    }

    /// 构造复用本次已缓存登录密码的测试请求。
    #[cfg(test)]
    pub fn reuse_login_password(
        validate_token: impl Into<String>,
        account: impl Into<String>,
        validate_type: i32,
    ) -> Self {
        Self {
            validate_token: validate_token.into(),
            pending_validate_dtos: vec![PendingValidationInputDto {
                country_code: None,
                account: Some(account.into()),
                account_type: None,
                validate_type: test_validate_type(validate_type),
                validate_value: None,
                saved_password_uid: None,
                reuse_login_password: true,
            }],
        }
    }
}

/// 将测试中的整数 `validateType` 转为协议枚举；未知值直接 panic，避免掩盖错误用例。
#[cfg(test)]
fn test_validate_type(value: i32) -> im_http::openchat_user::ValidateType {
    serde_json::from_value(serde_json::Value::from(value))
        .unwrap_or_else(|error| panic!("测试不支持的 validateType {value}: {error}"))
}

/// 解析秘密后得到的协议请求，以及首次主验证成功后才写入的待登录上下文。
pub(crate) struct ResolvedVerifySecrets {
    /// 已填入明文验证值、尚未做协议摘要的 `VerifyReq`。
    pub request: im_http::openchat_user::VerifyReq,
    /// 仅在该 token 尚无缓存且本次包含主登录校验时存在。
    pub first_primary: Option<crate::account::pending_login::PendingLogin>,
    /// 本次是否通过 `reuseLoginPassword` 取出了缓存密码；真正消费要等服务端见到请求之后。
    pub reuse_requested: bool,
}

/// 提交一组待验证项并返回服务端验证结果。
///
/// 命令先按应用级 DTO 解析手输值、已保存密码或一次复用密码，转换成协议
/// [`im_http::openchat_user::VerifyReq`] 后再按 [`hash_verify_passwords`] 改写密码类验证值，
/// 最后发起远程验证。验证码等非密码验证值保持不变。首次主验证成功后会把显示账号和
/// 可选明文密码写入待登录缓存；响应和日志不得回传密码。错误以 [`AuthCommandError`]
/// 返回，但请求发出后的错误不证明服务端校验状态未改变。
#[tauri::command]
pub async fn verify_validations(
    state: State<'_, AppState>,
    request: VerifyValidationsDto,
) -> Result<im_http::openchat_user::VerifyResp, AuthCommandError> {
    verify_validations_inner(&state, request).await
}

/// 解析秘密、发起远程验证，并在服务端见到请求后再消费一次性复用。
///
/// 单元测试默认视为远程验证成功并返回空响应，可用 [`verify_validations_inner_with_http`]
/// 注入传输失败或业务错误。生产路径调用 `openchat_user.verify`。
/// HTTP 未成功时不写入首次主验证缓存；传输失败不消费 `reuseLoginPassword`。
pub(crate) async fn verify_validations_inner(
    state: &AppState,
    dto: VerifyValidationsDto,
) -> Result<im_http::openchat_user::VerifyResp, AuthCommandError> {
    #[cfg(test)]
    {
        return verify_validations_after_http(
            state,
            dto,
            Ok(im_http::openchat_user::VerifyResp {
                validate_model_vos: Vec::new(),
                business_processing: Vec::new(),
            }),
        )
        .await;
    }
    #[cfg(not(test))]
    {
        let resolved = resolve_verify_secrets(state, dto).await?;
        let token = resolved.request.validate_token.clone();
        let reuse_requested = resolved.reuse_requested;
        let first_primary = resolved.first_primary;
        let mut request = resolved.request;
        hash_verify_passwords(&mut request);
        let http_result = state.http.openchat_user.verify(&request).await;
        finish_verify_after_http(state, &token, reuse_requested, first_primary, http_result).await
    }
}

/// 测试用：用注入的 HTTP 结果走完整解析与复用消费路径，不访问真实网络。
#[cfg(test)]
pub(crate) async fn verify_validations_inner_with_http(
    state: &AppState,
    dto: VerifyValidationsDto,
    http_result: Result<
        im_http::openchat_user::VerifyResp,
        im_http::openchat_user::OpenChatUserError,
    >,
) -> Result<im_http::openchat_user::VerifyResp, AuthCommandError> {
    verify_validations_after_http(state, dto, http_result).await
}

/// 解析秘密并套用注入的 HTTP 结果，供测试覆盖传输失败与成功消费。
#[cfg(test)]
async fn verify_validations_after_http(
    state: &AppState,
    dto: VerifyValidationsDto,
    http_result: Result<
        im_http::openchat_user::VerifyResp,
        im_http::openchat_user::OpenChatUserError,
    >,
) -> Result<im_http::openchat_user::VerifyResp, AuthCommandError> {
    let resolved = resolve_verify_secrets(state, dto).await?;
    let token = resolved.request.validate_token.clone();
    let reuse_requested = resolved.reuse_requested;
    let first_primary = resolved.first_primary;
    let mut request = resolved.request;
    hash_verify_passwords(&mut request);
    let _ = request;
    finish_verify_after_http(state, &token, reuse_requested, first_primary, http_result).await
}

/// 解析每项恰好一种秘密来源，并决定是否准备首次主验证缓存。
///
/// 已保存密码和复用登录密码仅允许 `LoginPassword` / `EmailPassword`。
/// 若该 `validateToken` 已有待登录上下文，则不再用本次账号覆盖最初的完整显示账号。
/// 本函数不发起 HTTP，也不在解析阶段写入缓存。
pub(crate) async fn resolve_verify_secrets(
    state: &AppState,
    dto: VerifyValidationsDto,
) -> Result<ResolvedVerifySecrets, AuthCommandError> {
    use im_http::openchat_user::PendingValidateDto;

    let existing = state.pending_login.get(&dto.validate_token).await;
    let mut first_primary = None;
    let mut reuse_requested = false;
    let mut pending_validate_dtos = Vec::with_capacity(dto.pending_validate_dtos.len());

    for item in &dto.pending_validate_dtos {
        if item.reuse_login_password {
            reuse_requested = true;
        }
        let secret = resolve_one_secret(state, &dto.validate_token, item).await?;
        if existing.is_none()
            && first_primary.is_none()
            && is_primary_login_validate(item.validate_type)
        {
            let password = if is_login_password_validate(item.validate_type) {
                Some(zeroize::Zeroizing::new(secret.clone()))
            } else {
                None
            };
            first_primary = Some(crate::account::pending_login::PendingLogin {
                display_account: item.account.clone().unwrap_or_default(),
                primary_login_type: login_type_from_validate(item.validate_type),
                password,
                password_reused: false,
            });
        }
        pending_validate_dtos.push(PendingValidateDto {
            country_code: item.country_code,
            account: item.account.clone(),
            account_type: item.account_type,
            validate_type: item.validate_type,
            validate_value: secret,
        });
    }

    Ok(ResolvedVerifySecrets {
        request: im_http::openchat_user::VerifyReq {
            validate_token: dto.validate_token,
            pending_validate_dtos,
            second_mac: None,
        },
        first_primary,
        reuse_requested,
    })
}

/// 根据远程验证是否到达服务端决定是否消费一次性复用，并仅在成功时写入首次主验证缓存。
async fn finish_verify_after_http(
    state: &AppState,
    token: &str,
    reuse_requested: bool,
    first_primary: Option<crate::account::pending_login::PendingLogin>,
    http_result: Result<
        im_http::openchat_user::VerifyResp,
        im_http::openchat_user::OpenChatUserError,
    >,
) -> Result<im_http::openchat_user::VerifyResp, AuthCommandError> {
    if reuse_requested && verify_reached_server(&http_result) {
        state.pending_login.reuse_password_once(token).await?;
    }
    match http_result {
        Ok(response) => {
            record_pending_login_after_verify(state, token, first_primary).await;
            Ok(response)
        }
        Err(error) => Err(error.into()),
    }
}

/// 判断远程校验是否已经让服务端见到请求：成功响应、业务错误或可解码失败都算。
fn verify_reached_server(
    result: &Result<im_http::openchat_user::VerifyResp, im_http::openchat_user::OpenChatUserError>,
) -> bool {
    match result {
        Ok(_) => true,
        Err(im_http::openchat_user::OpenChatUserError::Business(_)) => true,
        Err(im_http::openchat_user::OpenChatUserError::Decode(_)) => true,
        Err(im_http::openchat_user::OpenChatUserError::Transport(_)) => false,
        Err(im_http::openchat_user::OpenChatUserError::Validation(_)) => false,
    }
}

/// 远程验证成功后写入首次主验证上下文；后续 challenge 不会传入 `first_primary`。
async fn record_pending_login_after_verify(
    state: &AppState,
    token: &str,
    first_primary: Option<crate::account::pending_login::PendingLogin>,
) {
    if let Some(login) = first_primary {
        state.pending_login.insert(token, login).await;
    }
}

/// 解析单项秘密：手输值、凭据库密码或一次复用密码。
async fn resolve_one_secret(
    state: &AppState,
    token: &str,
    item: &PendingValidationInputDto,
) -> Result<String, AuthCommandError> {
    let typed = item
        .validate_value
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    let saved = item
        .saved_password_uid
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    let reuse = item.reuse_login_password;

    match (typed, saved, reuse) {
        (true, false, false) => Ok(item.validate_value.clone().unwrap_or_default()),
        (false, true, false) => {
            ensure_login_password_type(item.validate_type)?;
            let uid = crate::commands::parse_i64_id(
                item.saved_password_uid.as_deref().unwrap_or_default(),
                "savedPasswordUid",
            )?;
            state
                .credentials
                .password(uid)
                .await?
                .ok_or_else(|| AuthCommandError::from("未找到该账号已保存的登录密码"))
        }
        (false, false, true) => {
            ensure_login_password_type(item.validate_type)?;
            Ok(state.pending_login.peek_password(token).await?.to_string())
        }
        _ => Err(AuthCommandError::from(
            "每个待验证项必须且只能选择 validateValue、savedPasswordUid 或 reuseLoginPassword 之一",
        )),
    }
}

/// 已保存密码和复用登录密码只允许登录密码或邮箱密码校验。
fn ensure_login_password_type(
    validate_type: im_http::openchat_user::ValidateType,
) -> Result<(), AuthCommandError> {
    if is_login_password_validate(validate_type) {
        Ok(())
    } else {
        Err(AuthCommandError::from(
            "已保存密码和复用登录密码仅可用于 LoginPassword 或 EmailPassword",
        ))
    }
}

/// 主登录校验：邮箱/手机验证码以及两类登录密码。
fn is_primary_login_validate(validate_type: im_http::openchat_user::ValidateType) -> bool {
    use im_http::openchat_user::ValidateType;
    matches!(
        validate_type,
        ValidateType::EmailCode
            | ValidateType::PhoneCode
            | ValidateType::LoginPassword
            | ValidateType::EmailPassword
    )
}

/// 可从凭据库或待登录缓存读取的登录密码校验类型。
fn is_login_password_validate(validate_type: im_http::openchat_user::ValidateType) -> bool {
    use im_http::openchat_user::ValidateType;
    matches!(
        validate_type,
        ValidateType::LoginPassword | ValidateType::EmailPassword
    )
}

/// 将主校验类型映射为登录命令使用的 `loginType` 整数。
fn login_type_from_validate(validate_type: im_http::openchat_user::ValidateType) -> i32 {
    use im_http::openchat_user::ValidateType;
    match validate_type {
        ValidateType::PhoneCode => 1,
        ValidateType::EmailCode => 2,
        ValidateType::LoginPassword => 3,
        ValidateType::EmailPassword => 4,
        other => other as i32,
    }
}

/// 按验证类型应用与既有客户端一致的密码摘要格式。
///
/// `TradePassword` 使用不加盐的双 MD5；`LoginPassword` 和 `EmailPassword` 使用
/// [`login_password_md5`]；其余类型保持原值。这里是协议兼容处理，不代表安全密码存储。
fn hash_verify_passwords(request: &mut im_http::openchat_user::VerifyReq) {
    use im_http::openchat_user::ValidateType;

    for item in &mut request.pending_validate_dtos {
        item.validate_value = match item.validate_type {
            ValidateType::TradePassword => double_md5(&item.validate_value),
            ValidateType::LoginPassword | ValidateType::EmailPassword => {
                login_password_md5(&item.validate_value)
            }
            _ => continue,
        };
    }
}

/// 计算登录/邮箱密码兼容摘要：先 MD5，再对“首轮十六进制摘要 +
/// `!@#b%^&*9`”进行第二次 MD5。
fn login_password_md5(value: &str) -> String {
    use md5::{Digest, Md5};

    let first = format!("{:x}", Md5::digest(value.as_bytes()));
    format!("{:x}", Md5::digest(format!("{first}!@#b%^&*9").as_bytes()))
}

/// 计算交易密码兼容摘要：先 MD5，再对首轮十六进制摘要进行第二次 MD5。
fn double_md5(value: &str) -> String {
    use md5::{Digest, Md5};

    let first = format!("{:x}", Md5::digest(value.as_bytes()));
    format!("{:x}", Md5::digest(first.as_bytes()))
}

/// 查询指定校验令牌尚待完成的验证项。
///
/// `request` 标识远程校验流程；返回服务端待验证项列表。该命令仅查询远程状态，不修改
/// 本地认证会话或连接状态，业务错误通过 [`AuthCommandError`] 保留。
#[tauri::command]
pub async fn list_pending_validations(
    state: State<'_, AppState>,
    request: im_http::openchat_user::ListPendingValidateReq,
) -> Result<Vec<im_http::openchat_user::ValidateModelVo>, AuthCommandError> {
    state
        .http
        .openchat_user
        .list_pending_validations(&request)
        .await
        .map_err(AuthCommandError::from)
}

/// 登录并在本地发布可连接的认证会话。
///
/// `request` 先执行本地校验；随后 [`begin_auth_transition`] 取消旧连接并清理旧会话，
/// 再调用远程登录。遇到挑战时先把待登录缓存从旧 `validateToken` 迁到新令牌，返回
/// [`LoginResultDto::Challenge`] 且不保存凭据。成功时由 [`handle_remote_login_result`]
/// 补取 uid、拉取群组并调用 [`complete_account_login`]：迁移旧库、打开 UID 数据库、
/// 同步群组、发布会话、保存凭据与账号索引，再启动自动连接。
///
/// 群组同步失败不会留下半成品会话；过期 generation 不能覆盖较新的登录状态。凭据保存
/// 失败不撤销已经成功的远端登录。HTTP 登录成功不表示 TCP 已经连接。
///
/// 远程登录、用户详情和群组查询都会发起网络请求；错误可能来自请求校验、HTTP、群组
/// 同步、旧连接清理或并发状态切换。远程登录可能创建服务端认证状态，而用户详情和群组
/// 查询在本客户端中作为读取接口使用；远程登录请求发出后，即使最终返回错误，也不能
/// 据此断定服务端没有创建认证状态。
#[tauri::command]
pub async fn login(
    state: State<'_, AppState>,
    request: im_http::openchat_user::LoginReq,
) -> Result<LoginResultDto, AuthCommandError> {
    request.validate()?;
    let generation = begin_auth_transition(
        &state.connection_coordinator,
        &state.chat_client,
        &state.auth_session,
        &state.monitoring_groups,
        &state.connected,
        Some(state.app_handle()),
    )
    .await?;
    // 认证代际切换后立即丢弃上一账号的可选本地解密材料，并关闭旧账号库，
    // 防止远程登录失败后仍留下无会话的活动数据库。
    state.message_crypto.clear().await;
    state.account_db.close().await;

    // 远程认证、群组同步和旧连接清理完成前，不发布新的认证会话。
    let remote_login = classify_remote_login(state.http.openchat_user.login(&request).await)?;
    handle_remote_login_result(&state, generation, &request, remote_login, None).await
}

/// 在群组操作锁内准备群组状态，并在 generation 仍有效时发布登录状态。
///
/// `prepare` 失败时不会发布认证会话和内存监控快照；generation 检查还会阻止过期登录
/// 执行准备步骤或覆盖新状态。此函数串行化本地群组相关更新，但不承诺远程请求与本地
/// 数据库之间具备事务原子性。
async fn finish_login_after_sync<T, F>(
    state: LoginStateRefs<'_>,
    generation: u64,
    uid: i64,
    token: String,
    prepare: F,
) -> Result<T, String>
where
    F: Future<Output = Result<(T, HashSet<i64>), String>>,
{
    let _group_operation = state.group_ops.lock().await;
    let (result, restored_monitoring) = state
        .connection_coordinator
        .prepare_login_if_current(generation, state.auth_session, prepare)
        .await?;
    let session = AuthSession {
        uid,
        token,
        generation,
    };
    state
        .connection_coordinator
        .publish_login_if_current(
            generation,
            state.auth_session,
            state.monitoring_groups,
            session,
            restored_monitoring,
        )
        .await?;
    Ok(result)
}

/// 在已打开的账号库上同步群组并发布会话；失败时按打开代际关闭活动库。
///
/// 打开成功后若群组同步或会话发布失败，调用方不得留下“无会话却占用活动库”的状态。
/// 仅当活动 UID 与 generation 仍属于本次打开时才关闭数据库，避免并发登录、切换
/// 或同账号重入已经打开更新代际后被误关。
pub(crate) async fn finish_login_after_opening_account<T, F>(
    state: &AppState,
    generation: u64,
    uid: i64,
    token: String,
    prepare: F,
) -> Result<T, String>
where
    F: Future<Output = Result<(T, HashSet<i64>), String>>,
{
    let result = finish_login_after_sync(
        LoginStateRefs {
            group_ops: &state.group_ops,
            connection_coordinator: &state.connection_coordinator,
            auth_session: &state.auth_session,
            monitoring_groups: &state.monitoring_groups,
        },
        generation,
        uid,
        token,
        prepare,
    )
    .await;
    if result.is_err() {
        state.account_db.close_if_opened_by(uid, generation).await;
    }
    result
}

/// 登出当前会话。
///
/// 若存在已发布会话，先把对应账号索引标记为已退出（`has_token = false`，保留
/// `has_saved_password` 与 `last_used_uid`），再清理运行时会话、连接、消息密钥、
/// 待登录缓存和活动数据库，最后删除系统凭据库中的 Token。删除 Token 失败只返回
/// 非敏感 warning：索引已经退出，后续 [`crate::account::session::restore_session`]
/// 不得自动使用残留 Token。`mark_logged_out` 失败时仍清理运行时并尝试删除 Token，
/// 删除后再重试标记；若最终仍无法确认索引已退出，返回
/// [`CREDENTIAL_LOGOUT_UNCONFIRMED`]，不得使用保存失败警告，也不得报告干净退出。
/// 没有当前会话时仍清理运行时状态，但不改索引或凭据。
/// 清理发生在本地，不调用远程登出接口。
#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<LogoutResultDto, String> {
    logout_inner(&state).await
}

/// 执行退出语义，供 Tauri 命令与测试复用。
pub(crate) async fn logout_inner(state: &AppState) -> Result<LogoutResultDto, String> {
    let uid = state
        .auth_session
        .read()
        .await
        .as_ref()
        .map(|session| session.uid);
    let mut warnings = Vec::new();
    let mut mark_logged_out_failed = false;
    if let Some(uid) = uid {
        if let Err(error) = state.account_index.mark_logged_out(uid).await {
            tracing::warn!(error = %error, uid, "failed to mark account logged out");
            mark_logged_out_failed = true;
            state.account_index.note_unconfirmed_logout(uid);
        }
    }

    let generation = clear_session_state(
        state.connection_coordinator.as_ref(),
        state.chat_client.as_ref(),
        &state.auth_session,
        &state.monitoring_groups,
        &state.connected,
    )
    .await?;
    state.message_crypto.clear().await;
    state.pending_login.clear().await;
    // 连接取消并等待旧任务后，才关闭活动账号库，避免后台写入落到已关闭连接池。
    state.account_db.close().await;

    if let Some(uid) = uid {
        if let Err(error) = state.credentials.delete_token(uid).await {
            tracing::warn!(error = %error, uid, "failed to delete login token");
            warnings.push(CREDENTIAL_CLEAR_WARNING.to_string());
        }
        if mark_logged_out_failed {
            if let Err(error) = state.account_index.mark_logged_out(uid).await {
                tracing::warn!(error = %error, uid, "failed to retry mark account logged out");
                mark_logged_out_failed = true;
            } else {
                mark_logged_out_failed = false;
            }
        }
    }

    publish_disconnected_status_if_current(
        &state.connection_coordinator,
        generation,
        state.app_handle.as_ref(),
        &state.connected,
    )
    .await;
    if mark_logged_out_failed {
        return Err(CREDENTIAL_LOGOUT_UNCONFIRMED.to_string());
    }
    Ok(LogoutResultDto { warnings })
}

/// 清空会话、监控和连接状态，供登出流程与测试复用。
pub(crate) async fn clear_session_state(
    connection_coordinator: &crate::state::ConnectionCoordinator,
    chat_client: &crate::state::ClientSlot,
    auth_session: &Arc<tokio::sync::RwLock<Option<AuthSession>>>,
    monitoring_groups: &Arc<tokio::sync::RwLock<HashSet<i64>>>,
    connected: &Arc<tokio::sync::RwLock<bool>>,
) -> Result<u64, String> {
    begin_auth_transition(
        connection_coordinator,
        chat_client,
        auth_session,
        monitoring_groups,
        connected,
        None,
    )
    .await
}

/// 开始一次认证状态切换，并返回本次切换的 generation。
///
/// 先取消认证相关连接并清除认证会话，再清空内存监控集合、发布断开状态，最后传播实际
/// 断开结果。generation 用于让晚到的旧登录或旧连接结果失效；函数不承诺这些步骤对
/// 外部观察者表现为单一原子操作。
pub(crate) async fn begin_auth_transition(
    connection_coordinator: &crate::state::ConnectionCoordinator,
    chat_client: &crate::state::ClientSlot,
    auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
    monitoring_groups: &tokio::sync::RwLock<HashSet<i64>>,
    connected: &tokio::sync::RwLock<bool>,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<u64, String> {
    let reset =
        cancel_auth_and_disconnect(connection_coordinator, chat_client, auth_session).await?;
    let mut stored_monitoring = monitoring_groups.write().await;
    stored_monitoring.clear();
    drop(stored_monitoring);
    publish_disconnected_status_if_current(
        connection_coordinator,
        reset.generation,
        app_handle,
        connected,
    )
    .await;
    reset.disconnect_result?;
    Ok(reset.generation)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use im_store::group::GroupRow;

    use crate::commands::groups::toggle_monitor_serialized;
    use crate::{
        commands::chat::authenticated_session_for_connect,
        state::{load_monitoring_groups, AuthSession, ConnectionCoordinator},
    };

    use super::{
        begin_auth_transition, classify_remote_login, clear_session_state, complete_account_login,
        complete_account_login_after_publish, finish_login_after_opening_account,
        finish_login_after_sync, hash_verify_passwords, logout_inner, verify_validations_inner,
        verify_validations_inner_with_http, AccountSummaryDto, AuthCommandError, LoginResultDto,
        LoginStateRefs, PendingValidationInputDto, RemoteLogin, VerifyValidationsDto,
    };

    /// 构造已向内存凭据库写入登录密码的测试状态。
    ///
    /// 调用方必须持有返回的临时目录，直到测试结束，避免账号数据根被提前删除。
    async fn test_state_with_password(
        uid: i64,
        password: &str,
    ) -> (crate::state::AppState, tempfile::TempDir) {
        let (state, temp) = crate::state::test_state_with_account_foundation().await;
        state.credentials.set_password(uid, password).await.unwrap();
        (state, temp)
    }

    /// 构造使用内存凭据库的账号基础设施测试状态。
    ///
    /// 调用方必须持有返回的临时目录，直到测试结束，避免账号数据根被提前删除。
    async fn test_state_with_memory_credentials() -> (crate::state::AppState, tempfile::TempDir) {
        crate::state::test_state_with_account_foundation().await
    }

    /// 向待登录缓存写入一条带密码的上下文，供挑战与最终成功路径复用。
    async fn seed_pending_password(
        state: &crate::state::AppState,
        token: &str,
        display_account: &str,
        login_type: i32,
        password: &str,
    ) {
        state
            .pending_login
            .insert(
                token,
                crate::account::pending_login::PendingLogin {
                    display_account: display_account.into(),
                    primary_login_type: login_type,
                    password: Some(zeroize::Zeroizing::new(password.into())),
                    password_reused: false,
                },
            )
            .await;
    }

    /// 用注入的远程登录结果驱动本地收尾，避免真实 HTTP。
    ///
    /// 挑战路径只迁移待登录缓存；成功路径走 [`super::handle_remote_login_result`]，
    /// 并注入空群组快照以跳过远端群组拉取。
    async fn finish_remote_login_for_test(
        state: &crate::state::AppState,
        request_token: &str,
        remote: RemoteLogin,
    ) -> Result<LoginResultDto, AuthCommandError> {
        let request = im_http::openchat_user::LoginReq {
            login_type: im_http::openchat_user::LoginType::EmailPassword,
            email: Some("a@example.com".into()),
            validate_token: Some(request_token.to_string()),
            ..Default::default()
        };
        let generation = begin_auth_transition(
            &state.connection_coordinator,
            &state.chat_client,
            &state.auth_session,
            &state.monitoring_groups,
            &state.connected,
            None,
        )
        .await?;
        super::handle_remote_login_result(state, generation, &request, remote, Some(Vec::new()))
            .await
    }

    /// 挑战响应不得写入密码或 Token；只有最终登录成功才同时保存二者。
    #[tokio::test]
    async fn credentials_are_persisted_only_after_final_login_success() {
        let (state, _temp) = test_state_with_memory_credentials().await;
        seed_pending_password(&state, "issued", "a@example.com", 4, "secret").await;
        let challenge =
            finish_remote_login_for_test(&state, "issued", RemoteLogin::challenge("next")).await;
        assert!(challenge.is_ok());
        assert_eq!(state.credentials.password(42).await.unwrap(), None);
        assert_eq!(state.credentials.token(42).await.unwrap(), None);
        assert!(state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .is_empty());

        finish_remote_login_for_test(&state, "next", RemoteLogin::success(42, "token"))
            .await
            .unwrap();
        assert_eq!(
            state.credentials.password(42).await.unwrap().as_deref(),
            Some("secret")
        );
        assert_eq!(
            state.credentials.token(42).await.unwrap().as_deref(),
            Some("token")
        );
        let index = state.account_index.load().await.unwrap();
        let record = index
            .accounts
            .iter()
            .find(|item| item.uid == 42)
            .expect("最终成功必须写入账号索引");
        assert!(record.has_saved_password);
        assert!(record.has_token);
        assert_eq!(index.last_used_uid, Some(42));
    }

    /// 验证码登录没有缓存密码时，最终成功仍保存 Token，且不得写入密码。
    #[tokio::test]
    async fn code_login_persists_token_without_password() {
        let (state, _temp) = test_state_with_memory_credentials().await;
        state
            .pending_login
            .insert(
                "issued",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 2,
                    password: None,
                    password_reused: false,
                },
            )
            .await;

        let result =
            finish_remote_login_for_test(&state, "issued", RemoteLogin::success(42, "token"))
                .await
                .unwrap();
        let LoginResultDto::Success {
            account, warnings, ..
        } = result
        else {
            panic!("验证码登录成功应返回 Success");
        };
        assert_eq!(account.display_account, "a@example.com");
        assert_eq!(account.login_type, 2);
        assert!(!account.has_saved_password);
        assert!(account.is_current);
        assert!(warnings.is_empty());
        assert_eq!(
            state.credentials.token(42).await.unwrap().as_deref(),
            Some("token")
        );
        assert_eq!(state.credentials.password(42).await.unwrap(), None);
    }

    /// 先密码登录再验证码重登时，不得清掉已保存密码标志，也不得删除凭据库中的密码。
    #[tokio::test]
    async fn code_login_preserves_existing_saved_password_flag() {
        let (state, _temp) = test_state_with_memory_credentials().await;
        seed_pending_password(&state, "password", "a@example.com", 4, "secret").await;
        finish_remote_login_for_test(&state, "password", RemoteLogin::success(42, "token-1"))
            .await
            .unwrap();
        assert!(
            state
                .account_index
                .load()
                .await
                .unwrap()
                .accounts
                .iter()
                .find(|item| item.uid == 42)
                .unwrap()
                .has_saved_password
        );
        assert_eq!(
            state.credentials.password(42).await.unwrap().as_deref(),
            Some("secret")
        );

        state
            .pending_login
            .insert(
                "code",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 2,
                    password: None,
                    password_reused: false,
                },
            )
            .await;
        let result =
            finish_remote_login_for_test(&state, "code", RemoteLogin::success(42, "token-2"))
                .await
                .unwrap();
        let LoginResultDto::Success { account, .. } = result else {
            panic!("验证码重登成功应返回 Success");
        };
        assert!(account.has_saved_password);
        let record = state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|item| item.uid == 42)
            .unwrap();
        assert!(record.has_saved_password);
        assert_eq!(
            state.credentials.password(42).await.unwrap().as_deref(),
            Some("secret")
        );
        assert_eq!(
            state.credentials.token(42).await.unwrap().as_deref(),
            Some("token-2")
        );
    }

    /// 较新会话发布后，过期登录不得回写 Token 或把 last_used 改成自己。
    #[tokio::test]
    async fn stale_login_does_not_persist_credentials_after_newer_session() {
        let (state, _temp) = test_state_with_memory_credentials().await;
        seed_pending_password(&state, "old", "a@example.com", 4, "secret-42").await;
        let request = im_http::openchat_user::LoginReq {
            login_type: im_http::openchat_user::LoginType::EmailPassword,
            email: Some("a@example.com".into()),
            validate_token: Some("old".to_string()),
            ..Default::default()
        };
        let old_generation = begin_auth_transition(
            &state.connection_coordinator,
            &state.chat_client,
            &state.auth_session,
            &state.monitoring_groups,
            &state.connected,
            None,
        )
        .await
        .unwrap();

        let newer_state = state.clone();
        let result = complete_account_login_after_publish(
            &state,
            old_generation,
            42,
            zeroize::Zeroizing::new("token-42".into()),
            &request,
            Vec::new(),
            async move {
                seed_pending_password(&newer_state, "newer", "b@example.com", 4, "secret-99").await;
                finish_remote_login_for_test(
                    &newer_state,
                    "newer",
                    RemoteLogin::success(99, "token-99"),
                )
                .await
                .unwrap();
            },
        )
        .await;

        assert!(result.is_err(), "过期登录在较新会话发布后不得返回 Success");
        assert_eq!(
            state.credentials.token(99).await.unwrap().as_deref(),
            Some("token-99")
        );
        assert_eq!(state.credentials.token(42).await.unwrap(), None);
        assert_eq!(
            state.account_index.load().await.unwrap().last_used_uid,
            Some(99)
        );
    }

    /// 代际已过期的 complete 不得打开另一 UID 并替换当前活动库。
    #[tokio::test]
    async fn stale_complete_does_not_replace_already_open_account_database() {
        let (state, _temp) = test_state_with_memory_credentials().await;
        state.account_db.open(99, 0).await.unwrap();
        assert!(state.account_db.require(99).await.is_ok());

        let old_generation = 0_u64;
        begin_auth_transition(
            &state.connection_coordinator,
            &state.chat_client,
            &state.auth_session,
            &state.monitoring_groups,
            &state.connected,
            None,
        )
        .await
        .unwrap();
        assert!(
            !state
                .connection_coordinator
                .is_generation_current(old_generation)
                .await
        );

        let request = im_http::openchat_user::LoginReq {
            login_type: im_http::openchat_user::LoginType::EmailPassword,
            email: Some("a@example.com".into()),
            validate_token: Some("stale".to_string()),
            ..Default::default()
        };
        let result = complete_account_login(
            &state,
            old_generation,
            42,
            zeroize::Zeroizing::new("token-42".into()),
            &request,
            Vec::new(),
        )
        .await;
        assert!(result.is_err(), "过期 generation 不得继续打开账号库");
        state
            .account_db
            .require(99)
            .await
            .expect("过期登录不得替换已经打开的 UID 99 数据库");
    }

    /// 系统凭据库不可用时登录仍成功，只返回普通用户可理解的警告。
    #[tokio::test]
    async fn credential_save_failure_does_not_undo_successful_login() {
        let (state, _temp) = crate::state::test_state_with_credentials(std::sync::Arc::new(
            crate::account::credentials::UnavailableCredentialStore,
        ))
        .await;
        seed_pending_password(&state, "issued", "a@example.com", 4, "secret").await;

        let result =
            finish_remote_login_for_test(&state, "issued", RemoteLogin::success(42, "token"))
                .await
                .unwrap();
        let LoginResultDto::Success {
            uid,
            account,
            warnings,
            ..
        } = result
        else {
            panic!("凭据保存失败不得撤销已经成功的远端登录");
        };
        assert_eq!(uid, "42");
        assert_eq!(account.uid, "42");
        assert!(!account.has_saved_password);
        assert_eq!(warnings, vec!["本次无法安全保存登录信息".to_string()]);
        assert!(state.auth_session.read().await.is_some());
    }

    /// 已保存密码必须在 Rust 侧解析并写入待登录缓存，且验证响应不得回传密码。
    #[tokio::test]
    async fn saved_password_is_resolved_in_rust_and_never_returned() {
        let (state, _temp) = test_state_with_password(42, "saved-secret").await;
        let request = VerifyValidationsDto::saved_password("issued-token", 42, "a@example.com", 21);
        let response = verify_validations_inner(&state, request).await.unwrap();
        let pending = state.pending_login.take("issued-token").await.unwrap();
        assert_eq!(pending.display_account, "a@example.com");
        assert!(pending.password.is_some());
        let body = serde_json::to_string(&response).unwrap();
        assert!(!body.contains("saved-secret"), "验证响应不得返回已保存密码");
    }

    /// 同一待验证项不得同时携带手输密码和已保存密码 UID。
    #[tokio::test]
    async fn verify_validations_rejects_typed_value_and_saved_password_together() {
        let (state, _temp) = test_state_with_password(42, "saved-secret").await;
        let request = VerifyValidationsDto {
            validate_token: "issued-token".into(),
            pending_validate_dtos: vec![PendingValidationInputDto {
                country_code: None,
                account: Some("a@example.com".into()),
                account_type: None,
                validate_type: im_http::openchat_user::ValidateType::EmailPassword,
                validate_value: Some("typed-secret".into()),
                saved_password_uid: Some("42".into()),
                reuse_login_password: false,
            }],
        };
        assert!(verify_validations_inner(&state, request).await.is_err());
        assert!(state.pending_login.take("issued-token").await.is_none());
    }

    /// 验证码、交易密码和谷歌验证等非登录密码类型不得读取已保存密码。
    #[tokio::test]
    async fn saved_password_rejected_for_non_password_validate_types() {
        let (state, _temp) = test_state_with_password(42, "saved-secret").await;
        for validate_type in [16, 17, 18, 19] {
            let request = VerifyValidationsDto::saved_password(
                "issued-token",
                42,
                "a@example.com",
                validate_type,
            );
            assert!(
                verify_validations_inner(&state, request).await.is_err(),
                "validateType {validate_type} 不得使用已保存密码"
            );
        }
        assert!(state.pending_login.take("issued-token").await.is_none());
    }

    /// 传输失败不得消费一次性复用；验证成功后第二次复用必须拒绝。
    #[tokio::test]
    async fn verify_validations_reuse_survives_transport_failure_then_consumes() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        state
            .pending_login
            .insert(
                "issued-token",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 4,
                    password: Some(zeroize::Zeroizing::new("cached-secret".into())),
                    password_reused: false,
                },
            )
            .await;
        let request =
            VerifyValidationsDto::reuse_login_password("issued-token", "a***@example.com", 21);
        let transport = Err(im_http::openchat_user::OpenChatUserError::Transport(
            im_common::error::AppError::Http("simulated transport failure".into()),
        ));
        let error = verify_validations_inner_with_http(&state, request.clone(), transport)
            .await
            .unwrap_err();
        assert!(
            matches!(error, AuthCommandError::Other { ref message } if message.contains("HTTP")),
            "传输失败应原样返回，不得伪装成已复用"
        );
        assert!(
            !state
                .pending_login
                .get("issued-token")
                .await
                .unwrap()
                .password_reused
        );

        verify_validations_inner_with_http(
            &state,
            request.clone(),
            Ok(im_http::openchat_user::VerifyResp {
                validate_model_vos: Vec::new(),
                business_processing: Vec::new(),
            }),
        )
        .await
        .unwrap();
        let reused = verify_validations_inner(&state, request).await.unwrap_err();
        assert!(
            matches!(
                reused,
                AuthCommandError::Other { ref message }
                    if message.contains("禁止重复消费")
            ),
            "成功验证后第二次复用必须映射为 PasswordAlreadyReused"
        );
        assert!(
            state
                .pending_login
                .get("issued-token")
                .await
                .unwrap()
                .password_reused
        );
    }

    /// `reuseLoginPassword` 只能成功消费一次，第二次必须返回已复用错误。
    #[tokio::test]
    async fn verify_validations_reuse_login_password_succeeds_once() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        state
            .pending_login
            .insert(
                "issued-token",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 4,
                    password: Some(zeroize::Zeroizing::new("cached-secret".into())),
                    password_reused: false,
                },
            )
            .await;
        let request =
            VerifyValidationsDto::reuse_login_password("issued-token", "a***@example.com", 21);
        verify_validations_inner(&state, request.clone())
            .await
            .unwrap();
        let error = verify_validations_inner(&state, request).await.unwrap_err();
        assert!(
            matches!(
                error,
                AuthCommandError::Other { ref message }
                    if message.contains("禁止重复消费")
            ),
            "第二次复用必须映射为 PasswordAlreadyReused"
        );
        let pending = state.pending_login.take("issued-token").await.unwrap();
        assert_eq!(pending.display_account, "a@example.com");
        assert!(pending.password_reused);
    }

    #[test]
    fn verify_password_values_follow_java_pwd_util_contract() {
        use im_http::openchat_user::{PendingValidateDto, ValidateType, VerifyReq};

        let mut request = VerifyReq {
            validate_token: "token".to_string(),
            pending_validate_dtos: vec![
                PendingValidateDto {
                    country_code: Some(86),
                    account: Some("phone".to_string()),
                    account_type: None,
                    validate_type: ValidateType::LoginPassword,
                    validate_value: "Wo123456".to_string(),
                },
                PendingValidateDto {
                    country_code: None,
                    account: Some("email".to_string()),
                    account_type: None,
                    validate_type: ValidateType::EmailPassword,
                    validate_value: "Aa123456".to_string(),
                },
                PendingValidateDto {
                    country_code: None,
                    account: None,
                    account_type: None,
                    validate_type: ValidateType::TradePassword,
                    validate_value: "Wo123456".to_string(),
                },
                PendingValidateDto {
                    country_code: Some(86),
                    account: Some("phone".to_string()),
                    account_type: None,
                    validate_type: ValidateType::PhoneCode,
                    validate_value: "123456".to_string(),
                },
            ],
            second_mac: None,
        };

        hash_verify_passwords(&mut request);

        assert_eq!(
            request.pending_validate_dtos[0].validate_value,
            "babfa667177f136597a71552b181f54b"
        );
        assert_eq!(
            request.pending_validate_dtos[1].validate_value,
            "8e7d054b2773e11aec1b3e92bc71060d"
        );
        assert_eq!(
            request.pending_validate_dtos[2].validate_value,
            "2fee60254aca3faafa6cacc7b6236a2b"
        );
        assert_eq!(request.pending_validate_dtos[3].validate_value, "123456");
    }

    #[test]
    fn successful_login_serializes_uid_as_decimal_string() {
        let dto = LoginResultDto::Success {
            uid: i64::MAX.to_string(),
            groups: Vec::new(),
            account: AccountSummaryDto {
                uid: i64::MAX.to_string(),
                display_account: "a@example.com".to_string(),
                login_type: 4,
                has_saved_password: true,
                is_current: true,
            },
            warnings: Vec::new(),
        };

        let value = serde_json::to_value(dto).unwrap();
        assert_eq!(value["status"], "success");
        assert_eq!(value["uid"], i64::MAX.to_string());
        assert_eq!(value["warnings"], serde_json::json!([]));
        assert_eq!(value["account"]["uid"], i64::MAX.to_string());
        assert_eq!(value["account"]["displayAccount"], "a@example.com");
        assert_eq!(value["account"]["loginType"], 4);
        assert_eq!(value["account"]["hasSavedPassword"], true);
        assert_eq!(value["account"]["isCurrent"], true);
    }

    #[test]
    fn remote_login_success_requires_token_and_allows_uid_fallback() {
        let login = im_http::openchat_user::LoginData {
            uid: Some(42),
            authorization: Some(im_http::openchat_user::Authorization {
                access_token: Some("session-token".to_string()),
            }),
            ..Default::default()
        };

        assert_eq!(
            classify_remote_login(Ok(login)).unwrap(),
            RemoteLogin::Success {
                uid: Some(42),
                token: "session-token".to_string()
            }
        );
        let without_uid = classify_remote_login(Ok(im_http::openchat_user::LoginData {
            token: Some("detail-token".to_string()),
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(
            without_uid,
            RemoteLogin::Success {
                uid: None,
                token: "detail-token".to_string()
            }
        );
        let error =
            classify_remote_login(Ok(im_http::openchat_user::LoginData::default())).unwrap_err();
        assert!(matches!(
            error,
            AuthCommandError::Other { ref message }
                if message.contains("authorization token")
        ));
    }

    #[test]
    fn tauri_business_error_keeps_code_message_and_optional_data() {
        let error = AuthCommandError::from(im_http::openchat_user::OpenChatUserError::Business(
            im_http::openchat_user::ApiBusinessError {
                code: 3110002,
                msg: "phone code invalid".to_string(),
                data: Some(serde_json::json!({"remaining": 2})),
                display: Some(0),
                title: None,
                params: Some(vec!["2".to_string()]),
            },
        ));
        let value = serde_json::to_value(error).unwrap();

        assert_eq!(value["kind"], "business");
        assert_eq!(value["code"], 3110002);
        assert_eq!(value["msg"], "phone code invalid");
        assert_eq!(value["data"]["remaining"], 2);
    }

    #[test]
    fn business_3114179_becomes_serializable_challenge_with_pending_items() {
        let remote =
            classify_remote_login(Err(im_http::openchat_user::OpenChatUserError::Business(
                im_http::openchat_user::ApiBusinessError {
                    code: 3114179,
                    msg: "secondary validation required".to_string(),
                    data: Some(serde_json::json!({
                        "validateToken": "challenge-token",
                        "validateModelVOS": [{
                            "countryCode": 86,
                            "account": "138****8000",
                            "accountType": 1,
                            "validateType": 17
                        }]
                    })),
                    display: None,
                    title: None,
                    params: None,
                },
            )))
            .unwrap();
        let RemoteLogin::Challenge(challenge) = remote else {
            panic!("expected challenge");
        };
        let value = serde_json::to_value(LoginResultDto::Challenge {
            code: challenge.code,
            validate_token: challenge.validate_token,
            message: challenge.message,
            pending: challenge.pending,
        })
        .unwrap();

        assert_eq!(value["status"], "challenge");
        assert_eq!(value["code"], 3114179);
        assert_eq!(value["validateToken"], "challenge-token");
        assert_eq!(value["pending"][0]["validateType"], 17);
    }

    /// 模拟连接尚在结束时登出，验证旧会话先失效且取消中的连接不能重新获得认证。
    #[tokio::test]
    async fn logout_transition_clears_auth_before_cancelled_connect_can_finish() {
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let operation = coordinator.begin_connect(0).await.unwrap();
        let cancellation = operation.cancellation_token();
        let generation = operation.generation();
        let attempt_id = operation.attempt_id();
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "old-token".to_string(),
            generation,
        })));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let connected = Arc::new(tokio::sync::RwLock::new(true));
        let chat_client = Arc::new(tokio::sync::Mutex::new(None));
        let transition = {
            let coordinator = coordinator.clone();
            let auth_session = auth_session.clone();
            let monitoring_groups = monitoring_groups.clone();
            let connected = connected.clone();
            let chat_client = chat_client.clone();
            tokio::spawn(async move {
                begin_auth_transition(
                    &coordinator,
                    &chat_client,
                    &auth_session,
                    &monitoring_groups,
                    &connected,
                    None,
                )
                .await
            })
        };

        cancellation.cancelled().await;
        assert_eq!(
            authenticated_session_for_connect(&auth_session)
                .await
                .unwrap_err(),
            "Not logged in"
        );
        coordinator.finish_connect(generation, attempt_id).await;
        assert_eq!(transition.await.unwrap().unwrap(), 1);
    }

    /// 阻塞群组准备阶段，验证新会话发布前并发连接始终被拒绝。
    #[tokio::test]
    async fn new_login_keeps_connect_rejected_until_new_session_is_published() {
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 1,
            token: "old-token".to_string(),
            generation: 0,
        })));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let chat_client = Arc::new(tokio::sync::Mutex::new(None));
        let connected = Arc::new(tokio::sync::RwLock::new(true));
        let generation = begin_auth_transition(
            &coordinator,
            &chat_client,
            &auth_session,
            &monitoring_groups,
            &connected,
            None,
        )
        .await
        .unwrap();
        let (prepare_started, prepare_observed) = tokio::sync::oneshot::channel();
        let (release_prepare, prepare_released) = tokio::sync::oneshot::channel();
        let login = {
            let auth_session = auth_session.clone();
            let monitoring_groups = monitoring_groups.clone();
            let group_ops = group_ops.clone();
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                finish_login_after_sync(
                    LoginStateRefs {
                        group_ops: &group_ops,
                        connection_coordinator: &coordinator,
                        auth_session: &auth_session,
                        monitoring_groups: &monitoring_groups,
                    },
                    generation,
                    42,
                    "new-token".to_string(),
                    async {
                        prepare_started.send(()).unwrap();
                        prepare_released.await.unwrap();
                        Ok(((), [7].into_iter().collect()))
                    },
                )
                .await
            })
        };
        prepare_observed.await.unwrap();

        assert_eq!(
            authenticated_session_for_connect(&auth_session)
                .await
                .unwrap_err(),
            "Not logged in"
        );
        release_prepare.send(()).unwrap();
        login.await.unwrap().unwrap();
        assert_eq!(
            authenticated_session_for_connect(&auth_session)
                .await
                .unwrap(),
            AuthSession {
                uid: 42,
                token: "new-token".to_string(),
                generation,
            }
        );
    }

    #[tokio::test]
    async fn failed_group_sync_does_not_leave_partial_login_state() {
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 1,
            token: "old-token".to_string(),
            generation: 0,
        })));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let connection_coordinator = Arc::new(ConnectionCoordinator::new());
        let chat_client = tokio::sync::Mutex::new(None);
        let connected = tokio::sync::RwLock::new(true);
        let generation = begin_auth_transition(
            &connection_coordinator,
            &chat_client,
            &auth_session,
            &monitoring_groups,
            &connected,
            None,
        )
        .await
        .unwrap();

        let result: Result<Vec<im_store::group::GroupRow>, String> = finish_login_after_sync(
            LoginStateRefs {
                group_ops: &group_ops,
                connection_coordinator: &connection_coordinator,
                auth_session: &auth_session,
                monitoring_groups: &monitoring_groups,
            },
            generation,
            42,
            "session-token".to_string(),
            async { Err("group sync failed".to_string()) },
        )
        .await;

        assert_eq!(result.unwrap_err(), "group sync failed");
        assert_eq!(*auth_session.read().await, None);
        assert!(monitoring_groups.read().await.is_empty());
    }

    /// 账号库已打开但群组同步失败时，必须关闭活动库且不得发布会话。
    #[tokio::test]
    async fn failed_group_sync_after_open_closes_account_database() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        state.account_db.open(42, 0).await.unwrap();

        let result: Result<(), String> =
            finish_login_after_opening_account(&state, 0, 42, "session-token".to_string(), async {
                Err("group sync failed".to_string())
            })
            .await;

        assert_eq!(result.unwrap_err(), "group sync failed");
        assert_eq!(*state.auth_session.read().await, None);
        let error = match state.account_db.active().await {
            Err(error) => error,
            Ok(_) => panic!("群组同步失败后不得留下活动账号数据库"),
        };
        assert!(matches!(
            error,
            crate::account::AccountError::NoActiveDatabase
        ));
    }

    /// 打开后同步失败时，不得关闭已经被其他账号占用的活动库。
    #[tokio::test]
    async fn failed_group_sync_after_open_does_not_close_other_account_database() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        state.account_db.open(42, 1).await.unwrap();
        state.account_db.open(84, 2).await.unwrap();

        let result: Result<(), String> =
            finish_login_after_opening_account(&state, 0, 42, "session-token".to_string(), async {
                Err("group sync failed".to_string())
            })
            .await;

        assert_eq!(result.unwrap_err(), "group sync failed");
        assert_eq!(*state.auth_session.read().await, None);
        state
            .account_db
            .require(84)
            .await
            .expect("其他账号的活动库不得被失败的旧登录关闭");
    }

    #[tokio::test]
    async fn successful_relogin_restores_monitored_groups_from_database() {
        let store = im_store::SqliteStore::new(":memory:").await.unwrap();
        for (group_id, monitored) in [(10, 1), (20, 0)] {
            store
                .groups
                .insert_or_update(&GroupRow {
                    group_id,
                    name: group_id.to_string(),
                    pic: String::new(),
                    host_id: None,
                    member_count: 0,
                    created_at: 0,
                    monitored,
                    updated_at: 0,
                })
                .await
                .unwrap();
        }
        let auth_session = Arc::new(tokio::sync::RwLock::new(None));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let connection_coordinator = Arc::new(ConnectionCoordinator::new());
        let chat_client = tokio::sync::Mutex::new(None);
        let connected = tokio::sync::RwLock::new(true);
        let generation = begin_auth_transition(
            &connection_coordinator,
            &chat_client,
            &auth_session,
            &monitoring_groups,
            &connected,
            None,
        )
        .await
        .unwrap();
        let expected = AuthSession {
            uid: 42,
            token: "new-token".to_string(),
            generation,
        };

        finish_login_after_sync(
            LoginStateRefs {
                group_ops: &group_ops,
                connection_coordinator: &connection_coordinator,
                auth_session: &auth_session,
                monitoring_groups: &monitoring_groups,
            },
            generation,
            expected.uid,
            expected.token.clone(),
            async {
                let restored = load_monitoring_groups(&store)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(((), restored))
            },
        )
        .await
        .unwrap();

        assert_eq!(*auth_session.read().await, Some(expected));
        assert_eq!(*monitoring_groups.read().await, [10].into_iter().collect());
        assert!(!*connected.read().await);
    }

    /// 让旧登录晚于新登录到达，验证 generation 阻止其写库并覆盖新群组及会话快照。
    #[tokio::test]
    async fn stale_login_arriving_after_new_login_cannot_overwrite_group_snapshot() {
        let store = Arc::new(im_store::SqliteStore::new(":memory:").await.unwrap());
        let auth_session = Arc::new(tokio::sync::RwLock::new(None));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let group = |group_id, name: &str| GroupRow {
            group_id,
            name: name.to_string(),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: group_id,
        };

        let old_generation = coordinator
            .cancel_and_advance_clearing_auth(&auth_session)
            .await
            .unwrap()
            .0;
        let (release_old, old_released) = tokio::sync::oneshot::channel();
        let old_prepare_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let old_login = {
            let store = store.clone();
            let auth_session = auth_session.clone();
            let monitoring_groups = monitoring_groups.clone();
            let group_ops = group_ops.clone();
            let coordinator = coordinator.clone();
            let old_prepare_ran = old_prepare_ran.clone();
            let old_group = group(1, "old");
            tokio::spawn(async move {
                old_released.await.unwrap();
                finish_login_after_sync(
                    LoginStateRefs {
                        group_ops: &group_ops,
                        connection_coordinator: &coordinator,
                        auth_session: &auth_session,
                        monitoring_groups: &monitoring_groups,
                    },
                    old_generation,
                    1,
                    "old-token".to_string(),
                    async {
                        old_prepare_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                        crate::commands::groups::apply_remote_groups(&store, &[old_group]).await
                    },
                )
                .await
            })
        };

        let new_generation = coordinator
            .cancel_and_advance_clearing_auth(&auth_session)
            .await
            .unwrap()
            .0;
        let new_group = group(2, "new");
        finish_login_after_sync(
            LoginStateRefs {
                group_ops: &group_ops,
                connection_coordinator: &coordinator,
                auth_session: &auth_session,
                monitoring_groups: &monitoring_groups,
            },
            new_generation,
            2,
            "new-token".to_string(),
            async { crate::commands::groups::apply_remote_groups(&store, &[new_group]).await },
        )
        .await
        .unwrap();

        release_old.send(()).unwrap();
        assert!(old_login.await.unwrap().unwrap_err().contains("generation"));
        assert!(!old_prepare_ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            store
                .groups
                .list_all()
                .await
                .unwrap()
                .into_iter()
                .map(|group| group.group_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(*monitoring_groups.read().await, [2].into_iter().collect());
        assert_eq!(
            *auth_session.read().await,
            Some(AuthSession {
                uid: 2,
                token: "new-token".to_string(),
                generation: new_generation,
            })
        );
    }

    /// 控制重登录恢复与切换监控的到达顺序，验证二者共享群组操作锁且最终状态一致。
    #[tokio::test]
    async fn relogin_restore_and_toggle_share_group_operation_order() {
        let store = Arc::new(im_store::SqliteStore::new(":memory:").await.unwrap());
        store
            .groups
            .insert_or_update(&GroupRow {
                group_id: 7,
                name: "Group".to_string(),
                pic: String::new(),
                host_id: None,
                member_count: 0,
                created_at: 0,
                monitored: 1,
                updated_at: 0,
            })
            .await
            .unwrap();
        let auth_session = Arc::new(tokio::sync::RwLock::new(None));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let connection_coordinator = Arc::new(ConnectionCoordinator::new());
        let chat_client = Arc::new(tokio::sync::Mutex::new(None));
        let connected = Arc::new(tokio::sync::RwLock::new(true));
        let generation = begin_auth_transition(
            &connection_coordinator,
            &chat_client,
            &auth_session,
            &monitoring_groups,
            &connected,
            None,
        )
        .await
        .unwrap();
        let gate = group_ops.lock().await;

        let relogin = {
            let store = store.clone();
            let auth_session = auth_session.clone();
            let monitoring_groups = monitoring_groups.clone();
            let group_ops = group_ops.clone();
            let connection_coordinator = connection_coordinator.clone();
            tokio::spawn(async move {
                finish_login_after_sync(
                    LoginStateRefs {
                        group_ops: &group_ops,
                        connection_coordinator: &connection_coordinator,
                        auth_session: &auth_session,
                        monitoring_groups: &monitoring_groups,
                    },
                    generation,
                    42,
                    "token".to_string(),
                    async {
                        let restored = load_monitoring_groups(&store)
                            .await
                            .map_err(|error| error.to_string())?;
                        Ok(((), restored))
                    },
                )
                .await
            })
        };
        tokio::task::yield_now().await;
        let toggle = {
            let store = store.clone();
            let monitoring_groups = monitoring_groups.clone();
            let group_ops = group_ops.clone();
            tokio::spawn(async move {
                toggle_monitor_serialized(&group_ops, &store, &monitoring_groups, 7, false).await
            })
        };
        tokio::task::yield_now().await;
        drop(gate);

        relogin.await.unwrap().unwrap();
        toggle.await.unwrap().unwrap();

        assert!(auth_session.read().await.is_some());
        assert!(monitoring_groups.read().await.is_empty());
        assert_eq!(store.groups.list_all().await.unwrap()[0].monitored, 0);
    }

    /// 模拟阻塞中的连接任务，验证登出辅助函数及时取消并清空会话、监控和连接状态。
    #[tokio::test]
    async fn logout_helper_clears_session_monitoring_and_connection_state() {
        let connection_coordinator = Arc::new(ConnectionCoordinator::new());
        let operation = connection_coordinator.begin_connect(0).await.unwrap();
        let cancellation = operation.cancellation_token();
        let generation = operation.generation();
        let attempt_id = operation.attempt_id();
        let worker_coordinator = connection_coordinator.clone();
        let worker = tokio::spawn(async move {
            cancellation.cancelled().await;
            worker_coordinator
                .finish_connect(generation, attempt_id)
                .await;
        });
        let chat_client = tokio::sync::Mutex::new(None);
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        })));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(
            [10, 20].into_iter().collect::<HashSet<_>>(),
        ));
        let connected = Arc::new(tokio::sync::RwLock::new(true));

        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            clear_session_state(
                &connection_coordinator,
                &chat_client,
                &auth_session,
                &monitoring_groups,
                &connected,
            ),
        )
        .await
        .expect("logout must cancel blocked connect promptly")
        .unwrap();
        worker.await.unwrap();

        assert_eq!(*auth_session.read().await, None);
        assert!(monitoring_groups.read().await.is_empty());
        assert!(!*connected.read().await);
    }

    /// 退出先把索引标为已退出并保留密码，再删除 Token、清空待登录缓存。
    #[tokio::test]
    async fn logout_keeps_password_marks_logged_out_and_clears_pending_login() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        state
            .account_index
            .upsert(crate::account::index::AccountRecord::new(
                42,
                "a@example.com",
                4,
                100,
            ))
            .await
            .unwrap();
        state
            .account_index
            .set_secret_flags(42, true, true)
            .await
            .unwrap();
        state.credentials.set_token(42, "token-42").await.unwrap();
        state
            .credentials
            .set_password(42, "saved-secret")
            .await
            .unwrap();
        *state.auth_session.write().await = Some(AuthSession {
            uid: 42,
            token: "token-42".into(),
            generation: 0,
        });
        seed_pending_password(&state, "issued", "a@example.com", 4, "secret").await;

        let result = logout_inner(&state).await.unwrap();
        assert!(result.warnings.is_empty());
        assert_eq!(
            state.credentials.password(42).await.unwrap().as_deref(),
            Some("saved-secret")
        );
        assert_eq!(state.credentials.token(42).await.unwrap(), None);
        let index = state.account_index.load().await.unwrap();
        assert_eq!(index.last_used_uid, Some(42));
        let record = index.accounts.iter().find(|item| item.uid == 42).unwrap();
        assert!(!record.has_token);
        assert!(record.has_saved_password);
        assert!(state.pending_login.take("issued").await.is_none());
        assert!(state.auth_session.read().await.is_none());
    }
}
