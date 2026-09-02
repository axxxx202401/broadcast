use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn send_sms_code(
    _state: State<'_, AppState>,
    _phone: String,
    _country_code: i32,
    _gt4_dto: serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Phase 2: implement SMS code sending
    todo!("Phase 2")
}

#[tauri::command]
pub async fn login(
    _state: State<'_, AppState>,
    _phone: String,
    _country_code: i32,
    _validate_token: String,
) -> Result<serde_json::Value, String> {
    // Phase 2: implement login
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
