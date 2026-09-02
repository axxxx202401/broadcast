use std::collections::HashSet;
use std::sync::Arc;

use im_common::config::AppConfig;
use im_chat::ChatClient;
use im_store::SqliteStore;

pub struct AppState {
    pub config: Arc<tokio::sync::RwLock<AppConfig>>,
    pub db: Arc<SqliteStore>,
    pub chat_client: Arc<tokio::sync::Mutex<Option<ChatClient>>>,
    pub token: Arc<tokio::sync::RwLock<Option<String>>>,
    pub uid: Arc<tokio::sync::RwLock<Option<i64>>>,
    pub monitoring_groups: Arc<tokio::sync::RwLock<HashSet<i64>>>,
}
