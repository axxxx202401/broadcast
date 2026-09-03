//! Token 恢复与统一会话发布。
//!
//! 启动恢复只尝试索引中的最后账号，且该记录必须仍标记 `has_token`。
//! Token 由用户详情接口校验：业务拒绝或本地校验失败会要求重新登录，传输失败则保留 Token 供重试。
//! 成功路径复用登录收尾的 generation 门禁，但不得再次写入密码。

use std::{future::Future, pin::Pin};

use crate::account::index::AccountRecord;
use crate::commands::auth::{AccountSummaryDto, AuthCommandError, CREDENTIAL_SAVE_WARNING};
use crate::state::AppState;

/// 启动恢复或切换账号后返回给前端的会话结果。
///
/// `status` 使用 camelCase 标签；各变体内字段同样使用 camelCase。
/// 该结构不得包含 Token、密码或协议内部错误码作为主文案。
#[derive(Debug, serde::Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RestoreSessionDto {
    /// Token 有效，已发布会话并打开对应账号数据库。
    Success {
        /// 当前账号的非密钥摘要。
        account: AccountSummaryDto,
        /// 本次同步后的本地群组列表。
        groups: Vec<crate::commands::groups::GroupDto>,
        /// 非阻塞提示；更新最后账号失败时只放普通用户文案。
        warnings: Vec<String>,
    },
    /// 需要用户重新登录；不得自动进入主界面。
    NeedsLogin {
        /// 十进制字符串形式的用户 ID。
        uid: String,
        /// 用户输入的邮箱或手机号，仅用于展示和回填。
        display_account: String,
        /// 首次主登录使用的登录方式标识。
        login_type: i32,
        /// 系统凭据库是否已保存该账号登录密码。
        has_saved_password: bool,
    },
    /// 索引中没有任何账号，或最后账号记录已丢失。
    NoAccount,
    /// 网络等暂时失败，Token 仍保留，允许用户重试。
    Retryable {
        /// 十进制字符串形式的用户 ID。
        uid: String,
        /// 普通用户可理解的失败说明，不含协议码或内部实现细节。
        message: String,
    },
}

/// 传输失败时返回给前端的普通文案。
const NETWORK_RETRY_MESSAGE: &str = "网络连接失败，请重试";

/// 按当前代际恢复指定 UID 的会话。
///
/// 生产路径调用真实 `user_detail` 并拉取远端群组。测试应使用
/// [`restore_uid_with_user_detail`] 注入结果，避免访问网络。
pub async fn restore_uid(
    state: &AppState,
    generation: u64,
    uid: i64,
) -> Result<RestoreSessionDto, AuthCommandError> {
    #[cfg(test)]
    {
        let http = state.http.clone();
        return restore_uid_with_user_detail(
            state,
            generation,
            uid,
            move |token| {
                let http = http.clone();
                let token = token.to_string();
                async move { http.openchat_user.user_detail(&token).await }
            },
            Some(Vec::new()),
        )
        .await;
    }

    #[cfg(not(test))]
    {
        let state_for_groups = state.clone();
        let http = state.http.clone();
        restore_uid_with_services(
            state,
            generation,
            uid,
            move |token| {
                let http = http.clone();
                let token = token.to_string();
                async move { http.openchat_user.user_detail(&token).await }
            },
            move |token| {
                let token = token.to_string();
                Box::pin(async move {
                    crate::commands::groups::fetch_remote_groups(&state_for_groups, &token).await
                }) as GroupFetchFuture
            },
        )
        .await
    }
}

/// 启动恢复：只尝试索引中的最后账号，且该记录 `has_token` 必须为 `true`。
///
/// 没有最后账号或索引为空时返回 [`RestoreSessionDto::NoAccount`]。
/// 最后账号存在但已退出（`has_token == false`）时直接返回
/// [`RestoreSessionDto::NeedsLogin`]，不得读取或校验凭据库中可能残留的 Token。
pub async fn restore_session(state: &AppState) -> Result<RestoreSessionDto, AuthCommandError> {
    let generation = begin_restore_transition(state).await?;
    match last_restore_target(state).await? {
        LastRestoreTarget::NoAccount => Ok(RestoreSessionDto::NoAccount),
        LastRestoreTarget::NeedsLogin(record) => Ok(needs_login(&record)),
        LastRestoreTarget::Restore(uid) => restore_uid(state, generation, uid).await,
    }
}

/// 使用可注入的用户详情与群组快照恢复指定 UID。
///
/// `remote_groups` 为 `Some` 时跳过远端群组拉取，供测试注入空快照。
/// 打开账号库之后的失败仅在活动 UID 与 generation 仍属于本次打开时关闭数据库。
/// 成功发布会话后，更新最后使用记录失败只追加普通 warning，不得撤销已发布会话。
/// 本路径不写入密码。
#[cfg(test)]
pub(crate) async fn restore_uid_with_user_detail<F, Fut>(
    state: &AppState,
    generation: u64,
    uid: i64,
    user_detail: F,
    remote_groups: Option<Vec<im_store::group::GroupRow>>,
) -> Result<RestoreSessionDto, AuthCommandError>
where
    F: FnOnce(&str) -> Fut,
    Fut: Future<
        Output = Result<
            im_http::openchat_user::UserDetailResp,
            im_http::openchat_user::OpenChatUserError,
        >,
    >,
{
    restore_uid_with_services(state, generation, uid, user_detail, move |_token| {
        let remote_groups = remote_groups.clone();
        Box::pin(async move {
            match remote_groups {
                Some(groups) => Ok(groups),
                None => Err("restore_uid_with_user_detail requires injected groups".to_string()),
            }
        }) as GroupFetchFuture
    })
    .await
}

/// 测试辅助：同时注入用户详情与群组获取逻辑，覆盖恢复链路的失败收尾。
#[cfg(test)]
async fn restore_uid_with_injected_group_fetch<F, Fut, G>(
    state: &AppState,
    generation: u64,
    uid: i64,
    user_detail: F,
    group_fetch: G,
) -> Result<RestoreSessionDto, AuthCommandError>
where
    F: FnOnce(&str) -> Fut,
    Fut: Future<
        Output = Result<
            im_http::openchat_user::UserDetailResp,
            im_http::openchat_user::OpenChatUserError,
        >,
    >,
    G: FnOnce(&str) -> GroupFetchFuture,
{
    restore_uid_with_services(state, generation, uid, user_detail, group_fetch).await
}

/// 注入用户详情与群组获取逻辑，统一覆盖生产恢复与测试场景。
///
/// `generation` 必须来自调用方开始的恢复/切换过渡，后续 migrate/open/publish 全程沿用。
async fn restore_uid_with_services<F, Fut, G>(
    state: &AppState,
    generation: u64,
    uid: i64,
    user_detail: F,
    group_fetch: G,
) -> Result<RestoreSessionDto, AuthCommandError>
where
    F: FnOnce(&str) -> Fut,
    Fut: Future<
        Output = Result<
            im_http::openchat_user::UserDetailResp,
            im_http::openchat_user::OpenChatUserError,
        >,
    >,
    G: FnOnce(&str) -> GroupFetchFuture,
{
    let record = match load_account_record(state, uid).await? {
        Some(record) => record,
        None => {
            return Ok(RestoreSessionDto::NeedsLogin {
                uid: uid.to_string(),
                display_account: String::new(),
                login_type: 0,
                has_saved_password: false,
            });
        }
    };

    let token = match state.credentials.token(uid).await? {
        Some(token) if !token.trim().is_empty() => zeroize::Zeroizing::new(token),
        _ => {
            if record.has_token {
                if let Err(error) = state.account_index.mark_logged_out(uid).await {
                    tracing::warn!(error = %error, uid, "failed to mark missing token as logged out");
                }
            }
            return Ok(needs_login(&record));
        }
    };

    match user_detail(token.as_str()).await {
        Ok(_) => {}
        Err(im_http::openchat_user::OpenChatUserError::Business(_)) => {
            delete_rejected_token(state, uid).await;
            return Ok(needs_login(&record));
        }
        Err(im_http::openchat_user::OpenChatUserError::Transport(_))
        | Err(im_http::openchat_user::OpenChatUserError::Decode(_)) => {
            return Ok(RestoreSessionDto::Retryable {
                uid: uid.to_string(),
                message: NETWORK_RETRY_MESSAGE.to_string(),
            });
        }
        Err(im_http::openchat_user::OpenChatUserError::Validation(_)) => {
            if let Err(error) = state.account_index.mark_logged_out(uid).await {
                tracing::warn!(error = %error, uid, "failed to mark validation failure as logged out");
            }
            return Ok(needs_login(&record));
        }
    }

    finish_successful_restore(state, generation, uid, &record, token.as_str(), group_fetch).await
}

/// 统一群组获取 future 类型，便于测试注入失败路径。
type GroupFetchFuture =
    Pin<Box<dyn Future<Output = Result<Vec<im_store::group::GroupRow>, String>> + Send>>;

/// 启动恢复需要处理的最后账号目标。
enum LastRestoreTarget {
    NoAccount,
    NeedsLogin(AccountRecord),
    Restore(i64),
}

/// 读取最后账号并按 `has_token` 决定是否进入 Token 校验。
async fn last_restore_target(state: &AppState) -> Result<LastRestoreTarget, AuthCommandError> {
    let index = state.account_index.load().await?;
    let Some(uid) = index.last_used_uid else {
        return Ok(LastRestoreTarget::NoAccount);
    };
    let Some(record) = index.accounts.into_iter().find(|item| item.uid == uid) else {
        return Ok(LastRestoreTarget::NoAccount);
    };
    if record.has_token {
        Ok(LastRestoreTarget::Restore(uid))
    } else {
        Ok(LastRestoreTarget::NeedsLogin(record))
    }
}

/// 从索引读取指定 UID 的账号记录。
async fn load_account_record(
    state: &AppState,
    uid: i64,
) -> Result<Option<AccountRecord>, AuthCommandError> {
    Ok(state
        .account_index
        .load()
        .await?
        .accounts
        .into_iter()
        .find(|item| item.uid == uid))
}

/// 由索引记录构造需要重新登录的结果，不含密钥。
fn needs_login(record: &AccountRecord) -> RestoreSessionDto {
    RestoreSessionDto::NeedsLogin {
        uid: record.uid.to_string(),
        display_account: record.display_account.clone(),
        login_type: record.login_type,
        has_saved_password: record.has_saved_password,
    }
}

/// 业务拒绝后删除 Token 并标记已退出；删除失败只记日志，不回传密钥。
async fn delete_rejected_token(state: &AppState, uid: i64) {
    if let Err(error) = state.credentials.delete_token(uid).await {
        tracing::warn!(error = %error, uid, "failed to delete rejected token");
    }
    if let Err(error) = state.account_index.mark_logged_out(uid).await {
        tracing::warn!(error = %error, uid, "failed to mark rejected token as logged out");
    }
}

/// 在 Token 已校验通过后打开账号库、发布会话并刷新最后使用记录。
///
/// 打库前先确认当前 generation 仍有效。群组同步或会话发布失败时，
/// [`crate::commands::auth::finish_login_after_opening_account`] 会按打开代际关闭活动库。
/// 本函数不再无条件 `close`。成功后不写入密码。
async fn finish_successful_restore(
    state: &AppState,
    generation: u64,
    uid: i64,
    record: &AccountRecord,
    token: &str,
    group_fetch: impl FnOnce(&str) -> GroupFetchFuture,
) -> Result<RestoreSessionDto, AuthCommandError> {
    if !state
        .connection_coordinator
        .is_generation_current(generation)
        .await
    {
        return Err("Connection generation changed before opening account database".into());
    }

    state.legacy_migrator.migrate_if_needed(uid).await?;
    let db = state.account_db.open(uid, generation).await?;
    let remote_groups = match group_fetch(token).await {
        Ok(groups) => groups,
        Err(error) => {
            state.account_db.close_if_opened_by(uid, generation).await;
            return Err(error.into());
        }
    };
    let groups = crate::commands::auth::finish_login_after_opening_account(
        state,
        generation,
        uid,
        token.to_string(),
        async { crate::commands::groups::apply_remote_groups(&db, &remote_groups).await },
    )
    .await?;

    let mut warnings = Vec::new();
    if let Err(error) = state.account_index.touch_last_used(uid).await {
        tracing::warn!(error = %error, uid, "failed to touch last used account");
        warnings.push(CREDENTIAL_SAVE_WARNING.to_string());
    }

    if state.app_handle.is_some() {
        crate::commands::chat::start_automatic_connection(state, generation);
    }

    Ok(RestoreSessionDto::Success {
        account: AccountSummaryDto {
            uid: uid.to_string(),
            display_account: record.display_account.clone(),
            login_type: record.login_type,
            has_saved_password: record.has_saved_password,
            is_current: true,
        },
        groups: groups
            .into_iter()
            .map(crate::commands::groups::GroupDto::from)
            .collect(),
        warnings,
    })
}

/// 开始一次恢复过渡，并清理旧运行时状态。
///
/// 启动恢复与账号切换一样，都必须先推进 generation，再清理消息密钥、待登录缓存和旧库，
/// 让晚到的旧恢复结果无法借用较新的代际完成发布。
async fn begin_restore_transition(state: &AppState) -> Result<u64, AuthCommandError> {
    let generation = crate::commands::auth::begin_auth_transition(
        &state.connection_coordinator,
        &state.chat_client,
        &state.auth_session,
        &state.monitoring_groups,
        &state.connected,
        state.app_handle.as_ref(),
    )
    .await?;
    state.message_crypto.clear().await;
    state.pending_login.clear().await;
    state.account_db.close().await;
    Ok(generation)
}

/// 测试中注入的用户详情结果，避免访问真实网络。
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum UserDetailOutcome {
    /// Token 有效，允许继续打库并发布会话。
    Success,
    /// 服务端业务拒绝，视为 Token 失效。
    BusinessRejected,
    /// 传输层失败，应保留 Token 供重试。
    TransportFailure,
    /// 本地校验失败，应视为需要重新登录而不是网络重试。
    ValidationFailure,
}

#[cfg(test)]
impl UserDetailOutcome {
    /// 转换成 `user_detail` 的返回值；成功响应不携带密钥。
    pub(crate) fn into_result(
        self,
    ) -> Result<im_http::openchat_user::UserDetailResp, im_http::openchat_user::OpenChatUserError>
    {
        match self {
            Self::Success => Ok(im_http::openchat_user::UserDetailResp {
                user_base: im_http::openchat_user::UserBase { uid: Some(42) },
            }),
            Self::BusinessRejected => Err(im_http::openchat_user::OpenChatUserError::Business(
                im_http::openchat_user::ApiBusinessError {
                    code: 401,
                    msg: "unauthorized".into(),
                    data: None,
                    display: None,
                    title: None,
                    params: None,
                },
            )),
            Self::TransportFailure => Err(im_http::openchat_user::OpenChatUserError::Transport(
                im_common::error::AppError::Http("simulated transport failure".into()),
            )),
            Self::ValidationFailure => Err(im_http::openchat_user::OpenChatUserError::Validation(
                "simulated validation failure".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        begin_restore_transition, restore_session, restore_uid_with_injected_group_fetch,
        restore_uid_with_user_detail, RestoreSessionDto, UserDetailOutcome,
    };

    /// 为恢复测试准备带账号索引和 Token 的状态。
    ///
    /// 调用方必须持有返回值直到测试结束。默认写入 UID 42、已保存密码和 Token。
    async fn restore_test_state(outcome: UserDetailOutcome) -> RestoreTestState {
        let (state, temp) = crate::state::test_state_with_account_foundation().await;
        seed_restore_account(&state, 42, "a@example.com", 4, true, true, true).await;
        RestoreTestState {
            state,
            outcome,
            user_detail_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            _temp: temp,
        }
    }

    /// 写入一条账号记录，并按标志写入内存凭据。
    async fn seed_restore_account(
        state: &crate::state::AppState,
        uid: i64,
        display_account: &str,
        login_type: i32,
        has_saved_password: bool,
        has_token_flag: bool,
        write_token: bool,
    ) {
        state
            .account_index
            .upsert(crate::account::index::AccountRecord::new(
                uid,
                display_account,
                login_type,
                100,
            ))
            .await
            .unwrap();
        state
            .account_index
            .set_secret_flags(uid, has_saved_password, has_token_flag)
            .await
            .unwrap();
        if write_token {
            state
                .credentials
                .set_token(uid, "restore-token")
                .await
                .unwrap();
        }
        if has_saved_password {
            state
                .credentials
                .set_password(uid, "saved-secret")
                .await
                .unwrap();
        }
    }

    /// 带注入结果的测试状态；`Deref` 到 [`crate::state::AppState`] 以便读取凭据。
    struct RestoreTestState {
        state: crate::state::AppState,
        outcome: UserDetailOutcome,
        user_detail_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        _temp: tempfile::TempDir,
    }

    impl std::ops::Deref for RestoreTestState {
        type Target = crate::state::AppState;

        fn deref(&self) -> &Self::Target {
            &self.state
        }
    }

    /// 使用测试状态中的注入结果恢复指定 UID，并统计 `user_detail` 调用次数。
    async fn restore_uid(
        state: &RestoreTestState,
        uid: i64,
    ) -> Result<RestoreSessionDto, crate::commands::auth::AuthCommandError> {
        let outcome = state.outcome;
        let calls = state.user_detail_calls.clone();
        let generation = begin_restore_transition(&state.state).await.unwrap();
        restore_uid_with_user_detail(
            &state.state,
            generation,
            uid,
            move |_token| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { outcome.into_result() }
            },
            Some(Vec::new()),
        )
        .await
    }

    /// 业务拒绝必须删除 Token，并返回需要重新登录，不得进入主界面。
    #[tokio::test]
    async fn rejected_token_requires_login_and_is_deleted() {
        let state = restore_test_state(UserDetailOutcome::BusinessRejected).await;
        let result = restore_uid(&state, 42).await.unwrap();
        assert!(matches!(result, RestoreSessionDto::NeedsLogin { uid, .. } if uid == "42"));
        assert_eq!(state.credentials.token(42).await.unwrap(), None);
    }

    /// 传输失败必须保留 Token，并返回可重试结果。
    #[tokio::test]
    async fn transport_failure_keeps_token_for_retry() {
        let state = restore_test_state(UserDetailOutcome::TransportFailure).await;
        let result = restore_uid(&state, 42).await.unwrap();
        assert!(matches!(
            result,
            RestoreSessionDto::Retryable { ref message, .. }
                if message == "网络连接失败，请重试"
                    && !message.contains("HTTP")
                    && !message.contains("validateToken")
        ));
        assert!(state.credentials.token(42).await.unwrap().is_some());
    }

    /// 有效 Token 必须发布会话、打开对应 UID 数据库，并返回账号摘要与群组。
    #[tokio::test]
    async fn valid_token_publishes_session_and_opens_uid_database() {
        let state = restore_test_state(UserDetailOutcome::Success).await;
        let result = restore_uid(&state, 42).await.unwrap();
        let RestoreSessionDto::Success {
            account, groups, ..
        } = result
        else {
            panic!("有效 Token 必须返回 Success");
        };
        assert_eq!(account.uid, "42");
        assert_eq!(account.display_account, "a@example.com");
        assert_eq!(account.login_type, 4);
        assert!(account.has_saved_password);
        assert!(account.is_current);
        assert!(groups.is_empty());
        let session = state
            .auth_session
            .read()
            .await
            .clone()
            .expect("有效 Token 必须发布 AuthSession");
        assert_eq!(session.uid, 42);
        state
            .account_db
            .require(42)
            .await
            .expect("有效 Token 必须打开对应 UID 数据库");
        let record = state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|item| item.uid == 42)
            .unwrap();
        assert!(record.has_saved_password, "恢复成功不得清掉已保存密码标志");
        assert_eq!(
            state.credentials.password(42).await.unwrap().as_deref(),
            Some("saved-secret")
        );
    }

    /// 空索引启动恢复必须返回无账号，不得尝试校验 Token。
    #[tokio::test]
    async fn restore_session_without_accounts_returns_no_account() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        let result = restore_session(&state).await.unwrap();
        assert!(matches!(result, RestoreSessionDto::NoAccount));
    }

    /// 最后账号已退出时，索引 `has_token=false` 优先于凭据库残留 Token。
    ///
    /// 不得调用用户详情，也不得删除残留 Token；更不得自动进入主界面。
    #[tokio::test]
    async fn logged_out_last_account_needs_login_without_validating_token() {
        let state = restore_test_state(UserDetailOutcome::Success).await;
        state
            .account_index
            .set_secret_flags(42, true, false)
            .await
            .unwrap();
        assert!(
            state.credentials.token(42).await.unwrap().is_some(),
            "本用例依赖凭据库仍残留 Token"
        );

        let result = restore_session(&state.state).await.unwrap();
        assert!(
            matches!(
                result,
                RestoreSessionDto::NeedsLogin {
                    ref uid,
                    ref display_account,
                    login_type,
                    has_saved_password
                } if uid == "42"
                    && display_account == "a@example.com"
                    && login_type == 4
                    && has_saved_password
            ),
            "已退出账号必须回到登录页且不得自动进入主界面"
        );
        assert_eq!(
            state
                .user_detail_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "已退出账号不得调用 user_detail"
        );
        assert!(
            state.credentials.token(42).await.unwrap().is_some(),
            "已退出路由不得删除残留 Token"
        );
        assert!(state.auth_session.read().await.is_none());
    }

    /// 打开账号库后若拉取远端群组失败，必须关闭活动库且不得留下会话。
    #[tokio::test]
    async fn group_fetch_failure_after_open_closes_database() {
        let state = restore_test_state(UserDetailOutcome::Success).await;
        let generation = begin_restore_transition(&state.state).await.unwrap();

        let result = restore_uid_with_injected_group_fetch(
            &state.state,
            generation,
            42,
            move |_token| async move { UserDetailOutcome::Success.into_result() },
            move |_token| Box::pin(async move { Err("simulated group fetch failure".to_string()) }),
        )
        .await;

        assert!(result.is_err(), "群组拉取失败必须向上返回错误");
        assert!(state.auth_session.read().await.is_none());
        let error = match state.account_db.active().await {
            Err(error) => error,
            Ok(_) => panic!("群组拉取失败后不得留下活动账号数据库"),
        };
        assert!(matches!(
            error,
            crate::account::AccountError::NoActiveDatabase
        ));
        let error = match state.account_db.require(42).await {
            Err(error) => error,
            Ok(_) => panic!("群组拉取失败后不得继续按 UID 取得数据库"),
        };
        assert!(matches!(
            error,
            crate::account::AccountError::NoActiveDatabase
        ));
    }

    /// 本地校验失败不是网络可重试，而是需要重新登录并标记索引已退出。
    #[tokio::test]
    async fn validation_failure_requires_login_and_marks_logged_out() {
        let state = restore_test_state(UserDetailOutcome::ValidationFailure).await;

        let result = restore_uid(&state, 42).await.unwrap();

        assert!(matches!(result, RestoreSessionDto::NeedsLogin { uid, .. } if uid == "42"));
        let record = state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|item| item.uid == 42)
            .unwrap();
        assert!(!record.has_token);
        assert!(state.credentials.token(42).await.unwrap().is_some());
        assert!(state.auth_session.read().await.is_none());
    }
}
