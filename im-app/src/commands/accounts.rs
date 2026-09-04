//! 账号列表、启动恢复、切换、暂停会话和移除等 Tauri 命令。
//!
//! 这些命令只返回账号摘要与恢复结果，不得把 Token 或密码送出 IPC。
//! 切换与移除当前账号时复用认证代际推进，避免旧连接或旧数据库污染新账号。

use tauri::State;

use crate::account::session::{
    begin_restore_transition, restore_session as restore_session_inner, restore_uid,
    RestoreSessionDto,
};
use crate::commands::auth::{AccountSummaryDto, AuthCommandError, CREDENTIAL_CLEAR_WARNING};
use crate::commands::parse_i64_id;
use crate::state::AppState;

/// 暂停当前会话后返回给前端的结果；不得包含 Token 或密码。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseSessionDto {
    /// 暂停前的会话 UID；无活动会话时为 `None`。
    /// 前端用它在添加账号登录页提供返回，不得当作已删除 Token 的信号。
    pub uid: Option<String>,
}

/// 移除账号后返回给前端的结果。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAccountResultDto {
    /// 非阻塞提示；凭据删除失败时只放普通用户文案。
    pub warnings: Vec<String>,
    /// 删除后索引中的 `last_used_uid`；无剩余账号时为 `None`。
    /// 前端用它回填登录页默认账号，不得依赖列表写入顺序。
    pub next_uid: Option<String>,
}

/// 启动时恢复最后使用且仍持有 Token 的账号。
///
/// 没有账号返回 [`RestoreSessionDto::NoAccount`]；已退出账号返回
/// [`RestoreSessionDto::NeedsLogin`] 且不会校验残留 Token。
#[tauri::command]
pub async fn restore_session(
    state: State<'_, AppState>,
) -> Result<RestoreSessionDto, AuthCommandError> {
    restore_session_inner(&state).await
}

/// 返回全部已保存账号的非密钥摘要。
///
/// `isCurrent` 仅当已发布会话的 UID 与该记录一致时为 `true`。响应不得包含 Token 或密码。
#[tauri::command]
pub async fn list_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<AccountSummaryDto>, AuthCommandError> {
    list_accounts_inner(&state).await
}

/// 切换到指定 UID：先推进认证代际并清理旧运行时，再恢复目标账号。
///
/// `uid` 为十进制字符串。切换会清空消息密钥、待登录缓存并关闭旧账号库，
/// 随后按当前代际发布目标会话。
#[tauri::command]
pub async fn switch_account(
    state: State<'_, AppState>,
    uid: String,
) -> Result<RestoreSessionDto, AuthCommandError> {
    let uid = parse_i64_id(&uid, "uid")?;
    switch_account_inner(&state, uid).await
}

/// 暂停当前会话：断开 TCP 并清理运行时，但保留 Token 与 `has_token`。
///
/// 供「添加账号」进入登录页使用。返回暂停前的 UID，前端可用它返回上一账号。
/// 本命令不得删除凭据或把索引标成已退出。
#[tauri::command]
pub async fn pause_session(
    state: State<'_, AppState>,
) -> Result<PauseSessionDto, AuthCommandError> {
    pause_session_inner(&state).await
}

/// 移除指定账号的索引与凭据，但保留该 UID 的 SQLite 文件。
///
/// 若移除的是当前会话账号，会先按退出方式清理运行时。凭据删除失败只返回 warning，
/// 索引仍会删除，因此后续启动不会再列出该账号。
#[tauri::command]
pub async fn remove_account(
    state: State<'_, AppState>,
    uid: String,
) -> Result<RemoveAccountResultDto, AuthCommandError> {
    let uid = parse_i64_id(&uid, "uid")?;
    remove_account_inner(&state, uid).await
}

/// 读取账号索引并标注当前会话。
pub(crate) async fn list_accounts_inner(
    state: &AppState,
) -> Result<Vec<AccountSummaryDto>, AuthCommandError> {
    let current_uid = state
        .auth_session
        .read()
        .await
        .as_ref()
        .map(|session| session.uid);
    let index = state.account_index.load().await?;
    Ok(index
        .accounts
        .into_iter()
        .map(|record| AccountSummaryDto {
            uid: record.uid.to_string(),
            display_account: record.display_account,
            login_type: record.login_type,
            has_saved_password: record.has_saved_password,
            is_current: current_uid == Some(record.uid),
        })
        .collect())
}

/// 推进代际、清理旧运行时后恢复目标 UID。
pub(crate) async fn switch_account_inner(
    state: &AppState,
    uid: i64,
) -> Result<RestoreSessionDto, AuthCommandError> {
    let generation = prepare_account_switch(state).await?;
    restore_uid(state, generation, uid).await
}

/// 开始账号切换：与启动恢复共用同一套代际推进和运行时清理。
pub(crate) async fn prepare_account_switch(state: &AppState) -> Result<u64, AuthCommandError> {
    begin_restore_transition(state).await
}

/// 断开当前连接并清理运行时，保留所有账号的 Token 与退出标志。
pub(crate) async fn pause_session_inner(
    state: &AppState,
) -> Result<PauseSessionDto, AuthCommandError> {
    let uid = state
        .auth_session
        .read()
        .await
        .as_ref()
        .map(|session| session.uid);
    begin_restore_transition(state).await?;
    Ok(PauseSessionDto {
        uid: uid.map(|value| value.to_string()),
    })
}

/// 删除账号索引与凭据；若该 UID 是当前会话则先清理运行时。
pub(crate) async fn remove_account_inner(
    state: &AppState,
    uid: i64,
) -> Result<RemoveAccountResultDto, AuthCommandError> {
    let current_uid = state
        .auth_session
        .read()
        .await
        .as_ref()
        .map(|session| session.uid);
    if current_uid == Some(uid) {
        prepare_account_switch(state).await?;
    }

    let mut warnings = Vec::new();
    if let Err(error) = state.credentials.delete_token(uid).await {
        tracing::warn!(error = %error, uid, "failed to delete account token");
        warnings.push(CREDENTIAL_CLEAR_WARNING.to_string());
    }
    if let Err(error) = state.credentials.delete_password(uid).await {
        tracing::warn!(error = %error, uid, "failed to delete account password");
        if !warnings.iter().any(|item| item == CREDENTIAL_CLEAR_WARNING) {
            warnings.push(CREDENTIAL_CLEAR_WARNING.to_string());
        }
    }
    state.account_index.remove(uid).await?;
    let next_uid = state
        .account_index
        .load()
        .await?
        .last_used_uid
        .map(|value| value.to_string());
    Ok(RemoveAccountResultDto { warnings, next_uid })
}

#[cfg(test)]
mod tests {
    use super::{
        list_accounts_inner, pause_session_inner, prepare_account_switch, remove_account_inner,
    };
    use crate::account::session::{
        restore_uid_with_user_detail, RestoreSessionDto, UserDetailOutcome,
    };
    use crate::commands::auth::logout_inner;
    use crate::state::AuthSession;

    /// 写入一条账号记录及对应凭据，供列表、切换和移除测试复用。
    async fn seed_account(
        state: &crate::state::AppState,
        uid: i64,
        display_account: &str,
        login_type: i32,
        has_saved_password: bool,
        token: Option<&str>,
    ) {
        state
            .account_index
            .upsert(crate::account::index::AccountRecord::new(
                uid,
                display_account,
                login_type,
                uid,
            ))
            .await
            .unwrap();
        state
            .account_index
            .set_secret_flags(uid, has_saved_password, token.is_some())
            .await
            .unwrap();
        if let Some(token) = token {
            state.credentials.set_token(uid, token).await.unwrap();
        }
        if has_saved_password {
            state
                .credentials
                .set_password(uid, "saved-secret")
                .await
                .unwrap();
        }
    }

    /// 使用注入的用户详情恢复指定 UID，避免真实 HTTP。
    async fn restore_for_test(
        state: &crate::state::AppState,
        uid: i64,
        outcome: UserDetailOutcome,
    ) -> RestoreSessionDto {
        let generation = prepare_account_switch(state).await.unwrap();
        restore_uid_with_user_detail(
            state,
            generation,
            uid,
            move |_token| async move { outcome.into_result() },
            Some(Vec::new()),
        )
        .await
        .unwrap()
    }

    /// 列表只返回摘要，并用当前会话标注 `isCurrent`，序列化结果不得出现密钥。
    #[tokio::test]
    async fn list_accounts_marks_current_session_without_secrets() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        seed_account(&state, 84, "13800138000", 3, false, Some("token-84")).await;
        *state.auth_session.write().await = Some(AuthSession {
            uid: 42,
            token: "token-42".into(),
            generation: 0,
        });

        let accounts = list_accounts_inner(&state).await.unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].uid, "42");
        assert!(accounts[0].is_current);
        assert!(accounts[0].has_saved_password);
        assert_eq!(accounts[1].uid, "84");
        assert!(!accounts[1].is_current);
        assert!(!accounts[1].has_saved_password);

        let body = serde_json::to_string(&accounts).unwrap();
        assert!(!body.contains("token-42"));
        assert!(!body.contains("token-84"));
        assert!(!body.contains("saved-secret"));
    }

    /// 切换只断开来源运行时，必须保留来源账号 Token，切回时才能用原 Token 恢复。
    #[tokio::test]
    async fn switch_account_keeps_source_token_and_can_switch_back() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        seed_account(&state, 84, "b@example.com", 4, false, Some("token-84")).await;
        restore_for_test(&state, 42, UserDetailOutcome::Success).await;

        let switched = switch_account_inner_for_test(&state, 84, UserDetailOutcome::Success).await;
        assert!(matches!(
            switched,
            RestoreSessionDto::Success { ref account, .. } if account.uid == "84"
        ));
        assert_eq!(
            state.credentials.token(42).await.unwrap().as_deref(),
            Some("token-42"),
            "切走时不得删除来源 Token"
        );
        let source = state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|item| item.uid == 42)
            .expect("来源账号索引必须保留");
        assert!(source.has_token, "切走时不得把来源标记为已退出");

        let back = switch_account_inner_for_test(&state, 42, UserDetailOutcome::Success).await;
        assert!(matches!(
            back,
            RestoreSessionDto::Success { ref account, .. } if account.uid == "42"
        ));
        assert_eq!(
            state
                .auth_session
                .read()
                .await
                .as_ref()
                .map(|session| session.uid),
            Some(42)
        );
    }

    /// 添加账号只断开当前 TCP/会话，不得删除 Token 或把索引标成已退出。
    #[tokio::test]
    async fn pause_session_disconnects_without_deleting_token() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        restore_for_test(&state, 42, UserDetailOutcome::Success).await;
        state
            .pending_login
            .insert(
                "issued",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 4,
                    password: Some(zeroize::Zeroizing::new("secret".into())),
                    password_reused: false,
                },
            )
            .await;

        let result = pause_session_inner(&state).await.unwrap();
        assert_eq!(result.uid.as_deref(), Some("42"));
        assert_eq!(
            state.credentials.token(42).await.unwrap().as_deref(),
            Some("token-42")
        );
        let record = state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|item| item.uid == 42)
            .unwrap();
        assert!(record.has_token);
        assert!(state.auth_session.read().await.is_none());
        assert!(state.pending_login.take("issued").await.is_none());
        let error = match state.account_db.require(42).await {
            Err(error) => error,
            Ok(_) => panic!("暂停会话后不得保留活动账号数据库"),
        };
        assert!(matches!(
            error,
            crate::account::AccountError::NoActiveDatabase
                | crate::account::AccountError::ActiveUidMismatch { .. }
        ));
    }

    /// 切换必须推进代际、清理待登录缓存和消息密钥、关闭旧库，再恢复目标 UID。
    #[tokio::test]
    async fn switch_account_begins_transition_then_restores_target() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        seed_account(&state, 84, "b@example.com", 4, false, Some("token-84")).await;
        restore_for_test(&state, 42, UserDetailOutcome::Success).await;
        state
            .pending_login
            .insert(
                "issued",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 4,
                    password: Some(zeroize::Zeroizing::new("secret".into())),
                    password_reused: false,
                },
            )
            .await;
        state
            .message_crypto
            .install_own_private_key("old-private-key".into())
            .await;
        let old_generation = state.connection_coordinator.current_generation().await;
        state
            .account_db
            .require(42)
            .await
            .expect("切换前应打开当前账号数据库");

        let result = switch_account_inner_for_test(&state, 84, UserDetailOutcome::Success).await;
        assert!(matches!(
            result,
            RestoreSessionDto::Success { ref account, .. } if account.uid == "84"
        ));
        assert!(
            state.connection_coordinator.current_generation().await > old_generation,
            "切换必须推进 generation"
        );
        assert!(state.pending_login.take("issued").await.is_none());
        assert!(!state.message_crypto.has_own_private_key().await);
        state
            .account_db
            .require(84)
            .await
            .expect("切换成功后必须打开目标 UID 数据库");
        assert_eq!(
            state
                .auth_session
                .read()
                .await
                .as_ref()
                .map(|session| session.uid),
            Some(84)
        );
    }

    /// 移除账号必须删除索引、Token 和密码，但保留该 UID 的 SQLite 文件。
    #[tokio::test]
    async fn remove_account_deletes_index_and_secrets_but_keeps_sqlite() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        let db_path = state.account_db.database_path(42).unwrap();
        state.account_db.open(42, 0).await.unwrap();
        assert!(db_path.exists(), "打开账号库后测试必须能观察到 SQLite 文件");
        *state.auth_session.write().await = Some(AuthSession {
            uid: 42,
            token: "token-42".into(),
            generation: 0,
        });

        let result = remove_account_inner(&state, 42).await.unwrap();
        assert!(result.warnings.is_empty());
        assert_eq!(result.next_uid, None);
        assert!(state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .is_empty());
        assert_eq!(state.credentials.token(42).await.unwrap(), None);
        assert_eq!(state.credentials.password(42).await.unwrap(), None);
        assert!(db_path.exists(), "移除账号不得删除 SQLite 文件或账号目录");
        assert!(state.auth_session.read().await.is_none());
    }

    /// 移除当前账号后 `next_uid` 必须是剩余账号中 `last_used_at` 最大者，而非写入顺序首项。
    #[tokio::test]
    async fn remove_account_returns_next_uid_as_most_recently_used_remaining() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        // last_used_at 用显式时间戳：B 最近、C 次之、A 当前但将被移除。
        state
            .account_index
            .upsert(crate::account::index::AccountRecord::new(
                1,
                "a@example.com",
                4,
                100,
            ))
            .await
            .unwrap();
        state
            .account_index
            .upsert(crate::account::index::AccountRecord::new(
                3,
                "c@example.com",
                4,
                200,
            ))
            .await
            .unwrap();
        state
            .account_index
            .upsert(crate::account::index::AccountRecord::new(
                2,
                "b@example.com",
                4,
                300,
            ))
            .await
            .unwrap();
        state.account_index.touch_last_used(1).await.unwrap();
        *state.auth_session.write().await = Some(AuthSession {
            uid: 1,
            token: "token-1".into(),
            generation: 0,
        });

        let result = remove_account_inner(&state, 1).await.unwrap();
        assert_eq!(result.next_uid.as_deref(), Some("2"));
        let index = state.account_index.load().await.unwrap();
        assert_eq!(index.last_used_uid, Some(2));
        assert_eq!(
            index
                .accounts
                .iter()
                .map(|item| item.uid)
                .collect::<Vec<_>>(),
            vec![3, 2],
            "列表写入顺序仍以 C 在前，不得把 next_uid 误当成 accounts[0]"
        );
    }

    /// 凭据删除失败时仍移除索引，并返回不含密钥的 warning。
    #[tokio::test]
    async fn remove_account_warns_when_credential_delete_fails() {
        let store = std::sync::Arc::new(
            crate::account::credentials::FailingDeleteCredentialStore::default(),
        );
        let (state, _temp) = crate::state::test_state_with_credentials(store).await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;

        let result = remove_account_inner(&state, 42).await.unwrap();
        assert_eq!(
            result.warnings,
            vec!["本次无法完全清除登录信息".to_string()]
        );
        assert_eq!(result.next_uid, None);
        assert!(state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .is_empty());
        assert!(
            state.credentials.token(42).await.unwrap().is_some(),
            "删除失败时 Token 仍可能残留，但索引已移除"
        );
        let body = serde_json::to_string(&result).unwrap();
        assert!(!body.contains("token-42"));
        assert!(!body.contains("saved-secret"));
    }

    /// 退出必须先标记索引已退出并保留密码；删除 Token 失败时不得再被启动恢复自动使用。
    #[tokio::test]
    async fn logout_keeps_password_and_blocks_restore_when_token_delete_fails() {
        let store = std::sync::Arc::new(
            crate::account::credentials::FailingDeleteCredentialStore::default(),
        );
        let (state, _temp) = crate::state::test_state_with_credentials(store).await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        *state.auth_session.write().await = Some(AuthSession {
            uid: 42,
            token: "token-42".into(),
            generation: 0,
        });
        state
            .pending_login
            .insert(
                "issued",
                crate::account::pending_login::PendingLogin {
                    display_account: "a@example.com".into(),
                    primary_login_type: 4,
                    password: Some(zeroize::Zeroizing::new("secret".into())),
                    password_reused: false,
                },
            )
            .await;

        let result = logout_inner(&state).await.unwrap();
        assert_eq!(
            result.warnings,
            vec!["本次无法完全清除登录信息".to_string()]
        );
        assert_eq!(
            state.credentials.password(42).await.unwrap().as_deref(),
            Some("saved-secret")
        );
        assert!(state.credentials.token(42).await.unwrap().is_some());
        let index = state.account_index.load().await.unwrap();
        assert_eq!(index.last_used_uid, Some(42));
        let record = index.accounts.iter().find(|item| item.uid == 42).unwrap();
        assert!(!record.has_token);
        assert!(record.has_saved_password);
        assert!(state.pending_login.take("issued").await.is_none());
        assert!(state.auth_session.read().await.is_none());

        let restored = crate::account::session::restore_session(&state)
            .await
            .unwrap();
        assert!(
            matches!(restored, RestoreSessionDto::NeedsLogin { ref uid, .. } if uid == "42"),
            "索引已退出后即使残留 Token 也不得自动进入主界面"
        );
    }

    /// 索引无法标记退出且 Token 删除也失败时，不得报告干净退出，也不得自动恢复进主界面。
    #[tokio::test]
    async fn failing_index_mark_logged_out_is_not_clean_logout() {
        let store = std::sync::Arc::new(
            crate::account::credentials::FailingDeleteCredentialStore::default(),
        );
        let (state, _temp) = crate::state::test_state_with_credentials(store).await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        *state.auth_session.write().await = Some(crate::state::AuthSession {
            uid: 42,
            token: "token-42".into(),
            generation: 0,
        });
        state.account_index.set_fail_mutates(true);

        let result = logout_inner(&state).await;
        let error = match result {
            Err(error) => error,
            Ok(ok) => panic!("索引未能标记退出时不得返回成功: {ok:?}"),
        };
        assert_eq!(error, "本次无法确认已退出，请重试");
        assert!(
            !error.contains("无法安全保存"),
            "退出确认失败不得使用保存失败文案"
        );
        assert!(state.auth_session.read().await.is_none());
        assert!(
            state.credentials.token(42).await.unwrap().is_some(),
            "本用例强制 Token 删除失败，残留 Token 仍在"
        );
        let record = state
            .account_index
            .load()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|item| item.uid == 42)
            .unwrap();
        assert!(
            record.has_token,
            "索引改写失败后 has_token 必须仍为 true，才能暴露自动恢复风险"
        );

        let restored = crate::account::session::restore_session(&state).await;
        assert!(
            !matches!(restored, Ok(RestoreSessionDto::Success { .. })),
            "索引未能退出且 Token 仍在时，不得把退出当成可自动登录的 Success"
        );
    }

    /// 旧切换在较新代际出现后不得借用“当前 generation”发布会话。
    #[tokio::test]
    async fn stale_switch_does_not_publish_after_newer_generation() {
        let (state, _temp) = crate::state::test_state_with_account_foundation().await;
        seed_account(&state, 42, "a@example.com", 4, true, Some("token-42")).await;
        seed_account(&state, 99, "c@example.com", 4, false, Some("token-99")).await;
        restore_for_test(&state, 99, UserDetailOutcome::Success).await;
        assert_eq!(
            state
                .auth_session
                .read()
                .await
                .as_ref()
                .map(|session| session.uid),
            Some(99)
        );

        let (prepared_tx, prepared_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let switch = {
            let state = state.clone();
            tokio::spawn(async move {
                switch_account_inner_with_gate(&state, 42, prepared_tx, release_rx).await
            })
        };

        prepared_rx.await.expect("旧切换必须先完成代际清理");
        let newer_generation = prepare_account_switch(&state).await.unwrap();
        assert!(
            newer_generation >= 2,
            "较新切换必须推进 generation，使旧切换失效"
        );
        release_tx.send(()).expect("必须放行旧切换继续恢复");
        let result = switch.await.unwrap();
        assert!(result.is_err(), "旧切换在较新代际出现后不得成功发布");
        assert_ne!(
            state
                .auth_session
                .read()
                .await
                .as_ref()
                .map(|session| session.uid),
            Some(42),
            "旧切换不得把当前会话改回 42"
        );
        let error = match state.account_db.require(42).await {
            Err(error) => error,
            Ok(_) => panic!("过期切换不得留下 UID 42 的活动数据库"),
        };
        assert!(matches!(
            error,
            crate::account::AccountError::NoActiveDatabase
                | crate::account::AccountError::ActiveUidMismatch { .. }
        ));
    }

    /// 为代际竞争测试准备可阻塞的切换：先完成标准切换清理，再等待主线程推进较新代际。
    async fn switch_account_inner_with_gate(
        state: &crate::state::AppState,
        uid: i64,
        prepared: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<RestoreSessionDto, crate::commands::auth::AuthCommandError> {
        let generation = prepare_account_switch(state).await?;
        let _ = prepared.send(());
        release.await.map_err(|_| "stale switch gate dropped")?;
        crate::account::session::restore_uid_with_user_detail(
            state,
            generation,
            uid,
            move |_token| async move { UserDetailOutcome::Success.into_result() },
            Some(Vec::new()),
        )
        .await
    }

    /// 测试辅助：复用真实切换清理流程，但把恢复依赖注入为可控结果。
    async fn switch_account_inner_for_test(
        state: &crate::state::AppState,
        uid: i64,
        outcome: UserDetailOutcome,
    ) -> RestoreSessionDto {
        let generation = prepare_account_switch(state).await.unwrap();
        crate::account::session::restore_uid_with_user_detail(
            state,
            generation,
            uid,
            move |_token| async move { outcome.into_result() },
            Some(Vec::new()),
        )
        .await
        .unwrap()
    }
}
