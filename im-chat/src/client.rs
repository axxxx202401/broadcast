use im_common::config::AppConfig;
use im_common::error::{AppError, AppResult};
use im_common::{aes::AesCipher, tcp_head::TcpFrameHeader, version_key::HeaderManager};
use im_proto::{ClientInfo, LoginSessionMessage};
use prost::Message;
use std::{future::Future, io::Write, pin::Pin, sync::Arc};
use tokio::io::AsyncReadExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

use crate::frame::{
    decode_server_frame, encode_frame_with_header, FrameDecodeError, MAX_DECOMPRESSED_BODY_SIZE,
};

const TCP_GZIP_THRESHOLD: usize = 128;
const SERVER_ERROR_MESSAGE_ID: u16 = 9999;

pub type MessageFuture = Pin<Box<dyn Future<Output = AppResult<()>> + Send>>;
pub type MessageHandler = Box<dyn Fn(u16, Vec<u8>) -> MessageFuture + Send + Sync>;
pub type DisconnectFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
pub type DisconnectHandler = Box<dyn Fn() -> DisconnectFuture + Send + Sync>;

#[derive(Clone, Debug)]
pub struct ChatSender {
    config: AppConfig,
    pub(crate) stream: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>,
}

impl ChatSender {
    pub async fn send(
        &self,
        message_id: u16,
        content: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let frame = build_client_frame(&self.config, message_id, content)?;
        self.write_frame(&frame).await
    }

    pub async fn send_cancellable(
        &self,
        message_id: u16,
        content: &[u8],
        cancellation: &tokio_util::sync::CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let frame = build_client_frame(&self.config, message_id, content)?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err("Chat send cancelled".into()),
            result = tokio::time::timeout(timeout, self.write_frame(&frame)) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err(format!("Chat send timed out after {timeout:?}").into()),
                }
            }
        }
    }

    async fn write_frame(
        &self,
        frame: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut stream = self.stream.lock().await;
        let writer = stream.as_mut().ok_or("Not connected")?;
        tokio::io::AsyncWriteExt::write_all(writer, frame).await?;
        tokio::io::AsyncWriteExt::flush(writer).await?;
        Ok(())
    }
}

pub(crate) fn build_login_frame(config: &AppConfig, token: &str, _uid: i64) -> AppResult<Vec<u8>> {
    let login_msg = LoginSessionMessage {
        clinet_info: Some(ClientInfo {
            session_id: "".to_string(),
            app_ver: config.device.app_ver,
            package_code: config.device.package_code,
            plat: im_proto::Platform::Android as i32,
            language: config.device.language,
            sys_mac: config.device.sys_mac.clone(),
            sys_model: config.device.sys_model.clone(),
            token: token.to_string(),
            version: format!("{}-{}", config.device.app_ver, config.device.package_code),
        }),
        latest_login_time: 0,
        install_code: config.device.sys_mac.clone(),
        push_tag: 1,
    };

    build_client_frame(config, 1100, &login_msg.encode_to_vec())
}

fn build_client_frame(config: &AppConfig, message_id: u16, content: &[u8]) -> AppResult<Vec<u8>> {
    if content.len() > MAX_DECOMPRESSED_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "application body length {} exceeds limit {}",
            content.len(),
            MAX_DECOMPRESSED_BODY_SIZE
        )));
    }

    let x_one = HeaderManager::try_new(
        config.server.version_secret_name.clone(),
        config.server.header_aes_key.clone(),
    )?
    .build_x_one()?;
    let x_one_len = u32::try_from(x_one.len())
        .map_err(|_| AppError::TcpFrame("X-One length exceeds u32".to_string()))?;

    let cipher = AesCipher::try_new(config.server.body_aes_key.as_bytes())?;
    let encrypted = cipher.encrypt(content)?;
    let zipped = encrypted.len() >= TCP_GZIP_THRESHOLD;
    let transformed = if zipped {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&encrypted)?;
        encoder.finish()?
    } else {
        encrypted
    };

    let head = TcpFrameHeader::build_with_metadata(true, zipped, true, false, 0);
    debug!(
        message_id,
        head_0 = format_args!("0x{:02X}", head[0]),
        head_1 = format_args!("0x{:02X}", head[1]),
        plaintext_len = content.len(),
        x_one_len = x_one.len(),
        transformed_len = transformed.len(),
        zipped,
        "Built TCP client frame"
    );
    let mut body = Vec::with_capacity(4 + x_one.len() + transformed.len());
    body.extend_from_slice(&x_one_len.to_be_bytes());
    body.extend_from_slice(x_one.as_bytes());
    body.extend_from_slice(&transformed);
    encode_frame_with_header(message_id, &body, head)
}

pub struct ChatClient {
    config: AppConfig,
    stream: Option<Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>>,
    reader_task: Option<tokio::task::JoinHandle<()>>,
    handler: Option<Arc<MessageHandler>>,
    disconnect_handler: Option<Arc<DisconnectHandler>>,
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
            reader_task: None,
            handler: None,
            disconnect_handler: None,
        }
    }

    pub fn on_message<F, Fut>(&mut self, handler: F)
    where
        F: Fn(u16, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        self.handler = Some(Arc::new(Box::new(move |message_id, content| {
            Box::pin(handler(message_id, content))
        })));
    }

    pub fn on_disconnect<F, Fut>(&mut self, handler: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.disconnect_handler = Some(Arc::new(Box::new(move || Box::pin(handler()))));
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(reader_task) = &self.reader_task {
            if !reader_task.is_finished() {
                return Err(AppError::AlreadyConnected.into());
            }
        }
        if let Some(reader_task) = self.reader_task.take() {
            reader_task
                .await
                .map_err(|error| AppError::TcpFrame(format!("reader task failed: {}", error)))?;
        }

        let addr = format!(
            "{}:{}",
            self.config.server.im_chat_host, self.config.server.im_chat_port
        );
        info!("Connecting to IM chat server: {}", addr);
        let stream = TcpStream::connect(&addr).await?;
        let (reader, writer) = stream.into_split();
        let stream = Arc::new(tokio::sync::Mutex::new(Some(writer)));
        self.stream = Some(stream.clone());

        // Start background read task
        let reader = ReadTask {
            reader,
            stream,
            handler: self.handler.clone(),
            disconnect_handler: self.disconnect_handler.clone(),
            body_aes_key: self.config.server.body_aes_key.clone(),
            leftover: Vec::new(),
        };
        self.reader_task = Some(tokio::spawn(reader.run()));

        Ok(())
    }

    pub async fn disconnect(&mut self) {
        let mut notify = false;
        if let Some(stream) = self.stream.take() {
            if let Some(mut writer) = stream.lock().await.take() {
                notify = true;
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut writer).await;
            }
        }
        if let Some(reader_task) = self.reader_task.take() {
            if !reader_task.is_finished() {
                notify = true;
                reader_task.abort();
            }
            let _ = reader_task.await;
        }
        if notify {
            self.notify_disconnected().await;
        }
    }

    /// Synchronously terminate owned connection resources.
    ///
    /// Graceful shutdown should use [`Self::disconnect`] first. This method is
    /// the timeout and drop fallback.
    pub fn force_abort(&mut self) {
        if let Some(reader_task) = self.reader_task.take() {
            reader_task.abort();
        }
        if let Some(stream) = self.stream.take() {
            if let Ok(mut writer) = stream.try_lock() {
                writer.take();
            }
        }
    }

    pub async fn login(
        &self,
        token: &str,
        uid: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let frame = build_login_frame(&self.config, token, uid)?;
        self.write_frame(&frame).await?;

        info!("Login message sent to IM chat server");
        Ok(())
    }

    pub async fn send(
        &self,
        message_id: u16,
        content: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.sender()
            .ok_or("Not connected")?
            .send(message_id, content)
            .await
    }

    pub fn sender(&self) -> Option<ChatSender> {
        self.stream.as_ref().map(|stream| ChatSender {
            config: self.config.clone(),
            stream: stream.clone(),
        })
    }

    async fn write_frame(
        &self,
        frame: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = self.stream.as_ref().ok_or("Not connected")?;
        let mut stream = stream.lock().await;
        let writer = stream.as_mut().ok_or("Not connected")?;
        tokio::io::AsyncWriteExt::write_all(writer, frame).await?;
        tokio::io::AsyncWriteExt::flush(writer).await?;
        Ok(())
    }

    async fn notify_disconnected(&self) {
        if let Some(handler) = &self.disconnect_handler {
            handler().await;
        }
    }
}

impl Drop for ChatClient {
    fn drop(&mut self) {
        self.force_abort();
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
    reader: OwnedReadHalf,
    stream: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>,
    handler: Option<Arc<MessageHandler>>,
    disconnect_handler: Option<Arc<DisconnectHandler>>,
    body_aes_key: String,
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
            match self.reader.read(&mut partial).await {
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
            // `conn` is now dropped; safe to mutably borrow `self`
            if let Err(frame_error) = self.handle_data().await {
                error!(
                    "Invalid TCP frame; terminating read task and treating connection as lost: {}",
                    frame_error
                );
                break;
            }
        }
        if let Some(mut writer) = self.stream.lock().await.take() {
            if let Err(close_error) = tokio::io::AsyncWriteExt::shutdown(&mut writer).await {
                warn!("Failed to close invalid connection: {}", close_error);
            }
        }
        self.notify_disconnected().await;
    }

    async fn handle_data(&mut self) -> Result<(), AppError> {
        while self.leftover.len() >= 8 {
            let message_id = u16::from_be_bytes([self.leftover[2], self.leftover[3]]);
            let content_len = u32::from_be_bytes([
                self.leftover[4],
                self.leftover[5],
                self.leftover[6],
                self.leftover[7],
            ]);
            debug!(
                head_0 = format_args!("0x{:02X}", self.leftover[0]),
                head_1 = format_args!("0x{:02X}", self.leftover[1]),
                message_id,
                content_len,
                buffered_len = self.leftover.len(),
                "Received TCP frame header"
            );
            match decode_server_frame(&self.body_aes_key, &self.leftover) {
                Ok(frame) => {
                    self.leftover.drain(..frame.wire_len);
                    if frame.message_id == SERVER_ERROR_MESSAGE_ID {
                        match im_proto::ErrrMessage::decode(frame.content.as_slice()) {
                            Ok(server_error) => error!(
                                error_code = server_error.error_msg_code,
                                error_message = %server_error.error_msg,
                                message_protocol_id = server_error.message_protocol_id,
                                "IM chat server rejected TCP request"
                            ),
                            Err(decode_error) => error!(
                                content_len = frame.content.len(),
                                error = %decode_error,
                                "Failed to decode IM chat server error response"
                            ),
                        }
                    }
                    if let Some(handler) = &self.handler {
                        handler(frame.message_id, frame.content).await?;
                    }
                }
                Err(FrameDecodeError::Incomplete { .. }) => return Ok(()),
                Err(FrameDecodeError::Invalid(error)) => {
                    return Err(AppError::TcpFrame(format!(
                        "head=[0x{:02X},0x{:02X}] message_id={} content_len={} buffered_len={} decode_error={}",
                        self.leftover[0],
                        self.leftover[1],
                        message_id,
                        content_len,
                        self.leftover.len(),
                        error
                    )));
                }
            }
        }
        Ok(())
    }

    async fn notify_disconnected(&self) {
        if let Some(handler) = &self.disconnect_handler {
            handler().await;
        }
    }
}
