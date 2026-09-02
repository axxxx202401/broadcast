use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn connect_chat(state: State<'_, AppState>) -> Result<(), String> {
    let config = state.config.read().await.clone();
    let mut client = state.chat_client.lock().await;

    let mut chat_client = im_chat::ChatClient::new(config);
    chat_client.on_message(move |_msg_id: u16, _content: &[u8]| {
        // Phase 2: handle incoming messages
    });

    chat_client
        .connect()
        .await
        .map_err(|e| e.to_string())?;

    if let Some(token) = state.token.read().await.clone() {
        if let Some(uid) = *state.uid.read().await {
            chat_client
                .login(&token, uid)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    *client = Some(chat_client);
    Ok(())
}

#[tauri::command]
pub async fn disconnect_chat(state: State<'_, AppState>) -> Result<(), String> {
    let mut client = state.chat_client.lock().await;
    client.take();
    Ok(())
}
