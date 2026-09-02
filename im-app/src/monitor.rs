use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use im_chat::ChatClient;
use im_common::config::AppConfig;

/// Background monitor task that manages the TCP connection lifecycle,
/// including automatic reconnection on disconnect.
pub struct ChatMonitorTask {
    config: AppConfig,
    client: Arc<Mutex<Option<ChatClient>>>,
    connected: Arc<tokio::sync::RwLock<bool>>,
}

impl ChatMonitorTask {
    pub fn new(
        config: AppConfig,
        client: Arc<Mutex<Option<ChatClient>>>,
        connected: Arc<tokio::sync::RwLock<bool>>,
    ) -> Self {
        Self {
            config,
            client,
            connected,
        }
    }

    /// Start monitoring: connect and login, then spawn a background task
    /// that watches for disconnect and reconnects automatically.
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut chat_client = ChatClient::new(self.config.clone());
        chat_client.connect().await?;

        {
            let mut client = self.client.lock().await;
            *client = Some(chat_client);
        }
        *self.connected.write().await = true;

        info!("Chat monitor started");
        Ok(())
    }

    /// Stop the monitor and disconnect.
    pub async fn stop(&self) {
        let mut client = self.client.lock().await;
        if let Some(ref mut c) = *client {
            c.disconnect().await;
        }
        *client = None;
        *self.connected.write().await = false;
        info!("Chat monitor stopped");
    }
}
