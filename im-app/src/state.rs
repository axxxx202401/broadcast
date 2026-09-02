use std::collections::HashSet;
use std::sync::Arc;

use im_common::config::AppConfig;
use im_chat::ChatClient;
use im_http::http_clients::AppHttpClients;
use im_store::SqliteStore;
use tauri::AppHandle;

pub struct AppState {
    pub config: Arc<tokio::sync::RwLock<AppConfig>>,
    pub db: Arc<SqliteStore>,
    pub chat_client: Arc<tokio::sync::Mutex<Option<ChatClient>>>,
    pub token: Arc<tokio::sync::RwLock<Option<String>>>,
    pub uid: Arc<tokio::sync::RwLock<Option<i64>>>,
    pub monitoring_groups: Arc<tokio::sync::RwLock<HashSet<i64>>>,
    pub http: Arc<AppHttpClients>,
    pub connected: Arc<tokio::sync::RwLock<bool>>,
    pub app_handle: Option<AppHandle>,
}

impl AppState {
    pub fn app_handle(&self) -> &AppHandle {
        self.app_handle.as_ref().expect("app_handle not set")
    }
}
