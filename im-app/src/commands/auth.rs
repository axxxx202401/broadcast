use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn send_sms_code(
    state: State<'_, AppState>,
    phone: String,
    country_code: i32,
    gt4_dto: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let result = state
        .http
        .openchat_user
        .send_sms_captcha(&phone, country_code, &gt4_dto)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn login(
    state: State<'_, AppState>,
    phone: String,
    country_code: i32,
    validate_token: String,
) -> Result<serde_json::Value, String> {
    // 1. Login via openchat user client
    let login_result = state
        .http
        .openchat_user
        .login(&phone, country_code, &validate_token)
        .await
        .map_err(|e| e.to_string())?;

    let uid = login_result
        .uid
        .ok_or("Login response missing uid")?;
    let token = login_result
        .token
        .ok_or("Login response missing token")?;

    // 2. Store uid and token in state
    {
        let mut stored_uid = state.uid.write().await;
        *stored_uid = Some(uid);
    }
    {
        let mut stored_token = state.token.write().await;
        *stored_token = Some(token.clone());
    }

    // 3. Build ClientInfo and fetch group list
    let device = state.config.read().await.device.clone();
    let client_info = im_proto::ClientInfo {
        session_id: String::new(),
        app_ver: device.app_ver,
        package_code: device.package_code,
        plat: im_proto::Platform::Android as i32,
        language: device.language,
        sys_mac: device.sys_mac.clone(),
        sys_model: device.sys_model.clone(),
        token,
        version: format!("{}-{}", device.app_ver, device.package_code),
    };

    let groups = state
        .http
        .im_biz
        .fetch_group_list(&client_info)
        .await
        .map_err(|e| e.to_string())?;

    // 4. Upsert groups into DB
    for group_info in &groups {
        let group_row = im_store::group::GroupRow {
            group_id: group_info.group_id,
            name: group_info.name.clone(),
            pic: group_info.pic.clone(),
            host_id: group_info.host_id,
            member_count: group_info.member_count,
            created_at: 0,
            monitored: 0,
            updated_at: chrono::Utc::now().timestamp_millis(),
        };
        state
            .db
            .groups
            .insert_or_update(&group_row)
            .await
            .map_err(|e| e.to_string())?;
    }

    // 5. Return result
    let groups_json = serde_json::to_value(&groups).map_err(|e| e.to_string())?;
    serde_json::to_value(serde_json::json!({
        "uid": uid,
        "groups": groups_json
    })).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    let mut token = state.token.write().await;
    *token = None;
    let mut uid = state.uid.write().await;
    *uid = None;
    Ok(())
}
