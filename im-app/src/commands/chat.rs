use tauri::{State, Emitter};
use crate::state::AppState;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use prost::Message;

#[tauri::command]
pub async fn connect_chat(state: State<'_, AppState>) -> Result<(), String> {
    let config = state.config.read().await.clone();
    let mut client = state.chat_client.lock().await;
    let app_handle = state.app_handle().clone();

    let mut chat_client = im_chat::ChatClient::new(config)
        .with_app_handle(app_handle.clone());

    // Clone state refs for the closure
    let monitoring_groups = state.monitoring_groups.clone();
    let db = state.db.clone();
    let connected = state.connected.clone();
    let app_handle_for_closure = app_handle.clone();

    chat_client.on_message(move |msg_id: u16, content: &[u8]| {
        match msg_id {
            2202 => {
                // PushGroupMessage
                if let Ok(push_msg) = im_proto::PushGroupMessage::decode(content) {
                    let mg = monitoring_groups.clone();
                    let db = db.clone();
                    let app = app_handle_for_closure.clone();
                    for group_msg in push_msg.group_msg {
                        let gid = group_msg.group_id;
                        let should_monitor = mg.blocking_read().contains(&gid);
                        if should_monitor {
                            let db = db.clone();
                            let app = app.clone();
                            let msg_content = group_msg.content.clone();
                            let msg_id_val = group_msg.msg_id;
                            let send_uid = group_msg.send_uid;
                            let send_time = group_msg.send_time;
                            let msg_type = group_msg.msg_type;
                            tokio::spawn(async move {
                                let record = im_store::message::MessageRecord {
                                    msg_id: msg_id_val,
                                    group_id: gid,
                                    send_uid,
                                    msg_type,
                                    content: msg_content,
                                    send_time,
                                    content_md5: group_msg.content_md5.clone(),
                                };
                                if let Err(e) = db.messages.insert(&record).await {
                                    tracing::error!("Failed to insert message: {}", e);
                                }
                                let content_b64 = STANDARD.encode(&group_msg.content);
                                if let Err(e) = app.emit(
                                    "new_message",
                                    &serde_json::json!({
                                        "group_id": gid,
                                        "send_uid": send_uid,
                                        "msg_type": msg_type,
                                        "send_time": send_time,
                                        "content_b64": content_b64,
                                    }),
                                ) {
                                    tracing::error!("Failed to emit event: {}", e);
                                }
                            });
                        }
                    }
                }
            }
            1100 => {
                // LoginServer response — update connection status
                *connected.blocking_write() = true;
            }
            _ => {}
        }
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
    *state.connected.write().await = false;
    Ok(())
}

#[tauri::command]
pub async fn get_messages(
    state: State<'_, AppState>,
    group_id: i64,
    limit: usize,
    offset: usize,
) -> Result<serde_json::Value, String> {
    let messages = state
        .db
        .messages
        .get_by_group(group_id, limit, offset)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(messages).map_err(|e| e.to_string())
}
