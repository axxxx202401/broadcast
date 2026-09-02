use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tracing::{info, warn, error};

use crate::frame::{decode_frame, encode_frame};
use im_common::config::AppConfig;
use im_proto::{ClientInfo, LoginSessionMessage};
use prost::Message;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tauri::Emitter;

pub type MessageHandler = Box<dyn Fn(u16, &[u8]) + Send + Sync>;

#[derive(Clone)]
pub struct ChatClient {
    config: AppConfig,
    stream: Option<Arc<tokio::sync::Mutex<TcpStream>>>,
    handler: Option<Arc<MessageHandler>>,
    /// Optional Tauri app handle for emitting events.
    pub app_handle: Option<tauri::AppHandle>,
}

impl std::fmt::Debug for ChatClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatClient")
            .field("config", &self.config)
            .field("stream", &self.stream)
            .finish_non_exhaustive()
    }
}

impl ChatClient {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            stream: None,
            handler: None,
            app_handle: None,
        }
    }

    pub fn with_app_handle(mut self, handle: tauri::AppHandle) -> Self {
        self.app_handle = Some(handle);
        self
    }

    pub fn on_message<F>(&mut self, handler: F)
    where
        F: Fn(u16, &[u8]) + Send + Sync + 'static,
    {
        self.handler = Some(Arc::new(Box::new(handler)));
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!(
            "{}:{}",
            self.config.server.im_chat_host, self.config.server.im_chat_port
        );
        info!("Connecting to IM chat server: {}", addr);
        let stream = TcpStream::connect(&addr).await?;
        let stream = Arc::new(tokio::sync::Mutex::new(stream));
        self.stream = Some(stream.clone());

        // Start background read task
        let reader = ReadTask {
            stream: stream.clone(),
            handler: self.handler.clone(),
            app_handle: self.app_handle.clone(),
            leftover: Vec::new(),
        };
        tokio::spawn(reader.run());

        Ok(())
    }

    pub async fn disconnect(&mut self) {
        self.stream = None;
    }

    pub async fn login(&self, token: &str, _uid: i64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = self.stream.as_ref().ok_or("Not connected")?;
        let mut conn = stream.lock().await;

        let login_msg = LoginSessionMessage {
            clinet_info: Some(ClientInfo {
                session_id: "".to_string(),
                app_ver: self.config.device.app_ver,
                package_code: self.config.device.package_code,
                plat: im_proto::Platform::Android as i32,
                language: self.config.device.language,
                sys_mac: self.config.device.sys_mac.clone(),
                sys_model: self.config.device.sys_model.clone(),
                token: token.to_string(),
                version: format!("{}-{}", self.config.device.app_ver, self.config.device.package_code),
            }),
            latest_login_time: 0,
            install_code: self.config.device.sys_mac.clone(),
            push_tag: 1,
        };

        let body = login_msg.encode_to_vec();
        let frame = encode_frame(1100, &body, true, false); // encrypted=true, zipped=false

        tokio::io::AsyncWriteExt::write_all(&mut *conn, &frame).await?;
        tokio::io::AsyncWriteExt::flush(&mut *conn).await?;

        info!("Login message sent to IM chat server");
        Ok(())
    }

    pub async fn send(&self, message_id: u16, content: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = self.stream.as_ref().ok_or("Not connected")?;
        let mut conn = stream.lock().await;
        let frame = encode_frame(message_id, content, true, false);
        tokio::io::AsyncWriteExt::write_all(&mut *conn, &frame).await?;
        tokio::io::AsyncWriteExt::flush(&mut *conn).await?;
        Ok(())
    }
}

/// Background task that reads from the TCP stream and dispatches decoded frames
/// to the message handler.
///
/// FIX: Unlike the brief's skeleton which used `read_to_end` into a shared
/// `buf` (accumulating all data and losing frame boundaries), this version
/// keeps a persistent buffer (`leftover`) that holds incomplete frames between
/// loop iterations. Each `read` call appends only newly received bytes to
/// `self.leftover`, then `handle_data` consumes complete frames from it,
/// leaving any trailing incomplete data for the next iteration.
struct ReadTask {
    stream: Arc<tokio::sync::Mutex<TcpStream>>,
    handler: Option<Arc<MessageHandler>>,
    app_handle: Option<tauri::AppHandle>,
    /// Accumulates bytes across read iterations; holds the tail of an
    /// incomplete frame after each `handle_data` call.
    leftover: Vec<u8>,
}

impl ReadTask {
    async fn run(mut self) {
        loop {
            // Read newly available bytes into a temporary buffer, then
            // append them to `leftover`. This preserves incomplete frames
            // across iterations instead of losing them as `read_to_end`
            // on a shared buffer would do.
            let mut partial = vec![0u8; 4096];
            {
                let mut conn = self.stream.lock().await;
                match conn.read(&mut partial).await {
                    Ok(0) => {
                        warn!("Connection closed by server");
                        break;
                    }
                    Ok(n) => {
                        self.leftover.extend_from_slice(&partial[..n]);
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }
            // `conn` is now dropped; safe to mutably borrow `self`
            self.handle_data().await;
        }
    }

    async fn handle_data(&mut self) {
        while self.leftover.len() >= 8 {
            match decode_frame(&self.leftover) {
                Ok((_msg_id, content)) => {
                    let consumed = 8 + content.len();
                    if let Some(handler) = &self.handler {
                        handler(_msg_id, &content);
                    }
                    if let Some(app_handle) = &self.app_handle {
                        let content_b64 = STANDARD.encode(content);
                        if let Err(e) = app_handle.emit("chat_event", &serde_json::json!({"type": "message", "msg_id": _msg_id, "content": content_b64})) {
                            warn!("Failed to emit event: {}", e);
                        }
                    }
                    self.leftover.drain(..consumed);
                }
                Err(_) => break,
            }
        }
    }
}
