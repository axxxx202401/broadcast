mod state;
mod commands;
mod monitor;

use std::sync::Arc;
use tauri::Manager;
use state::AppState;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("im_app=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let config = im_common::config::AppConfig::default();
            let db = futures::executor::block_on(async {
                im_store::SqliteStore::new("data/im_monitor.db").await
            })
            .unwrap();
            let state = AppState {
                config: Arc::new(tokio::sync::RwLock::new(config)),
                db: Arc::new(db),
                chat_client: Arc::new(tokio::sync::Mutex::new(None)),
                token: Arc::new(tokio::sync::RwLock::new(None)),
                uid: Arc::new(tokio::sync::RwLock::new(None)),
                monitoring_groups: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
            };
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::auth::logout,
            commands::auth::send_sms_code,
            commands::groups::fetch_group_list,
            commands::groups::toggle_monitor,
            commands::chat::connect_chat,
            commands::chat::disconnect_chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
