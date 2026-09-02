use std::{collections::HashSet, future::Future, sync::Arc};

use tauri::State;

use crate::commands::chat::{cancel_auth_and_disconnect, publish_disconnected_status_if_current};
use crate::state::{AppState, AuthSession};

struct LoginStateRefs<'a> {
    group_ops: &'a tokio::sync::Mutex<()>,
    connection_coordinator: &'a crate::state::ConnectionCoordinator,
    auth_session: &'a tokio::sync::RwLock<Option<AuthSession>>,
    monitoring_groups: &'a tokio::sync::RwLock<HashSet<i64>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuthCommandError {
    Business {
        code: i32,
        msg: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Vec<String>>,
    },
    Other {
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

#[derive(Debug, serde::Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LoginResultDto {
    Success {
        uid: String,
        groups: Vec<crate::commands::groups::GroupDto>,
    },
    Challenge {
        code: i32,
        validate_token: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pending: Option<Vec<im_http::openchat_user::ValidateModelVo>>,
    },
}

#[derive(Debug, PartialEq)]
enum RemoteLogin {
    Success { uid: Option<i64>, token: String },
    Challenge(LoginChallenge),
}

#[derive(Debug, PartialEq)]
struct LoginChallenge {
    code: i32,
    validate_token: String,
    message: String,
    pending: Option<Vec<im_http::openchat_user::ValidateModelVo>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginChallengeData {
    validate_token: Option<String>,
    #[serde(default, rename = "validateModelVOS", alias = "pending")]
    pending: Option<Vec<im_http::openchat_user::ValidateModelVo>>,
}

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

#[tauri::command]
pub async fn verify_validations(
    state: State<'_, AppState>,
    mut request: im_http::openchat_user::VerifyReq,
) -> Result<im_http::openchat_user::VerifyResp, AuthCommandError> {
    hash_verify_passwords(&mut request);
    state
        .http
        .openchat_user
        .verify(&request)
        .await
        .map_err(AuthCommandError::from)
}

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

fn login_password_md5(value: &str) -> String {
    use md5::{Digest, Md5};

    let first = format!("{:x}", Md5::digest(value.as_bytes()));
    format!("{:x}", Md5::digest(format!("{first}!@#b%^&*9").as_bytes()))
}

fn double_md5(value: &str) -> String {
    use md5::{Digest, Md5};

    let first = format!("{:x}", Md5::digest(value.as_bytes()));
    format!("{:x}", Md5::digest(first.as_bytes()))
}

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

    // Publish nothing until remote authentication, group synchronization, and
    // old-connection cleanup have all succeeded.
    let remote_login = classify_remote_login(state.http.openchat_user.login(&request).await)?;
    let RemoteLogin::Success { uid, token } = remote_login else {
        let RemoteLogin::Challenge(challenge) = remote_login else {
            unreachable!()
        };
        return Ok(LoginResultDto::Challenge {
            code: challenge.code,
            validate_token: challenge.validate_token,
            message: challenge.message,
            pending: challenge.pending,
        });
    };
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
    let remote_groups = crate::commands::groups::fetch_remote_groups(&state, &token).await?;

    // 2. Apply the fetched groups locally and restore persisted monitor
    // selections before publishing the new authenticated session.
    let groups = finish_login_after_sync(
        LoginStateRefs {
            group_ops: &state.group_ops,
            connection_coordinator: &state.connection_coordinator,
            auth_session: &state.auth_session,
            monitoring_groups: &state.monitoring_groups,
        },
        generation,
        uid,
        token.clone(),
        async { crate::commands::groups::apply_remote_groups(&state.db, &remote_groups).await },
    )
    .await?;

    // 3. Keep HTTP login successful while TCP connects and retries in the background.
    crate::commands::chat::start_automatic_connection(&state, generation);

    // 4. Convert i64 identifiers only at the Tauri boundary.
    Ok(LoginResultDto::Success {
        uid: uid.to_string(),
        groups: groups
            .into_iter()
            .map(crate::commands::groups::GroupDto::from)
            .collect(),
    })
}

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

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    let generation = clear_session_state(
        state.connection_coordinator.as_ref(),
        state.chat_client.as_ref(),
        &state.auth_session,
        &state.monitoring_groups,
        &state.connected,
    )
    .await?;
    publish_disconnected_status_if_current(
        &state.connection_coordinator,
        generation,
        Some(state.app_handle()),
        &state.connected,
    )
    .await;
    Ok(())
}

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

async fn begin_auth_transition(
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
        begin_auth_transition, classify_remote_login, clear_session_state, finish_login_after_sync,
        hash_verify_passwords, AuthCommandError, LoginResultDto, LoginStateRefs, RemoteLogin,
    };

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
        };

        let value = serde_json::to_value(dto).unwrap();
        assert_eq!(value["status"], "success");
        assert_eq!(value["uid"], i64::MAX.to_string());
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
}
