use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn fetch_group_list(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let groups = state.db.groups.list_monitored().await.map_err(|e| e.to_string())?;
    serde_json::to_value(groups).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_monitor(
    state: State<'_, AppState>,
    group_id: i64,
    monitored: bool,
) -> Result<(), String> {
    state
        .db
        .groups
        .toggle_monitored(group_id, monitored)
        .await
        .map_err(|e| e.to_string())?;
    let mut monitoring = state.monitoring_groups.write().await;
    if monitored {
        monitoring.insert(group_id);
    } else {
        monitoring.remove(&group_id);
    }
    Ok(())
}
