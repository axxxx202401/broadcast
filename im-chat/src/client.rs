//! 聊天 TCP 客户端、可克隆发送端及后台读循环。
//!
//! 本模块负责连接生命周期、登录与应用消息封帧，以及服务端帧的顺序分派。

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
    decode_transport_frame, encode_frame_with_header, FrameDecodeError, MAX_DECOMPRESSED_BODY_SIZE,
};

const TCP_GZIP_THRESHOLD: usize = 128;
const SERVER_ERROR_MESSAGE_ID: u16 = 9999;

/// 消息回调返回的异步结果。
///
/// 回调返回错误时，后台读任务会停止处理后续帧，并按受控退出流程关闭写端、等待断开回调。
pub type MessageFuture = Pin<Box<dyn Future<Output = AppResult<()>> + Send>>;
/// 收到完整服务端帧后调用的消息回调。
///
/// 参数依次为消息 ID 和解码后的正文。回调在后台读任务中按帧顺序等待完成；
/// 回调期间不会分派下一帧。回调 panic 时通过 `catch_unwind` 捕获并记录错误，
/// 随后以协议错误终止读循环，触发正常的写端清理与断开回调。
pub type MessageHandler = Box<dyn Fn(u16, Vec<u8>) -> MessageFuture + Send + Sync>;
/// 断开通知处理器返回的异步工作。
///
/// 后台读任务受控退出和 [`ChatClient::disconnect`] 主动通知时都会等待该 future 完成。
pub type DisconnectFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
/// 连接被关闭后调用的回调。
///
/// 后台读任务因正常 EOF、读取或协议错误、消息处理器返回错误或 panic 而受控退出时，
/// 会先清理共享写端，再调用并等待该回调；[`ChatClient::disconnect`] 主动通知时
/// 也会等待它。[`ChatClient::force_abort`] 是同步兜底，不会调用或等待该回调。
pub type DisconnectHandler = Box<dyn Fn() -> DisconnectFuture + Send + Sync>;

/// 可独立持有的聊天连接发送端。
///
/// 该类型共享 [`ChatClient`] 当前连接的 TCP 写端。取得实例并不保证连接之后仍然有效：
/// 客户端断开、读任务因协议或 I/O 错误退出后，发送会返回未连接错误。
#[derive(Clone, Debug)]
pub struct ChatSender {
    config: AppConfig,
    pub(crate) stream: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>,
}

impl ChatSender {
    /// 构造客户端帧并写入当前连接。
    ///
    /// 发送前由内部帧构造逻辑添加 X-One、加密正文并按阈值决定是否 gzip；
    /// 随后等待共享写端锁，完整写入并刷新。调用方应先成功连接。
    ///
    /// # 错误
    ///
    /// 帧构造失败、客户端已断开，或 TCP 写入/刷新失败时返回错误。
    /// 本方法没有内置取消或超时；需要这些能力时使用 [`Self::send_cancellable`]。
    /// TCP I/O 错误时无法从返回值判断线路已写入多少数据或服务端是否收到，
    /// 具体风险与处理要求见 [`Self::send_cancellable`]。
    pub async fn send(
        &self,
        message_id: u16,
        content: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let frame = build_client_frame(&self.config, message_id, content)?;
        self.write_frame(&frame).await
    }

    /// 在可取消且有时限的条件下构造并发送客户端帧。
    ///
    /// 帧在进入取消/超时选择之前构造；构造成功后，发送过程包括等待共享写端锁、
    /// 写入全部字节和刷新。取消分支具有选择优先级，超时覆盖整个写端操作。
    ///
    /// # 错误
    ///
    /// 帧构造失败、取消令牌触发、等待或写入超过 `timeout`、客户端已断开，
    /// 或 TCP 写入/刷新失败时返回错误。取消、超时或 I/O 错误发生时，线路上可能
    /// 已写入 0 字节、部分帧或完整帧；返回错误不能证明服务端是否收到该帧。
    /// 已写入的部分帧不会回滚，继续复用该流可能造成协议错位，上层应按连接失败策略处理，
    /// 而不能把错误视为可在同一连接上安全重试的依据。
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

    /// 串行取得共享写端，写入完整帧并刷新。
    ///
    /// 客户端已断开时返回未连接错误；锁等待、写入与刷新均由调用方的 future 驱动，
    /// 本函数自身不设置取消或超时策略。
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

/// 构造消息 ID 为 1100 的登录帧。
///
/// 正文是 `LoginSessionMessage`，其中嵌入 `ClientInfo`。协议字段名
/// `clinet_info` 是现有 protobuf 的兼容拼写，不能更正为 `client_info`。
/// `_uid` 当前不参与帧构造；`Platform::Android`、空 `session_id`、
/// `latest_login_time = 0`、`push_tag = 1` 等值均按既有协议实现原样编码，
/// 此处不推断其业务含义。最终正文交给 [`build_client_frame`] 加工。
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

/// 将明文应用正文封装为客户端 TCP 帧。
///
/// 函数先校验明文不超过 [`MAX_DECOMPRESSED_BODY_SIZE`]，生成 X-One 字符串，
/// 再使用正文 AES 密钥加密 `content`。加密结果达到 128 字节时才进行 gzip。
/// 帧头 metadata 标记正文已加密、是否按阈值进行 gzip、系统版本信息已加密、
/// 非上报帧及协议版本 0；帧正文依次为 4 字节大端 X-One 长度、X-One 字节和
/// 加密后（可能 gzip）的正文，最后交给 `encode_frame_with_header` 写入消息 ID 与线长。
///
/// # 错误
///
/// 明文超限、X-One 或 AES 初始化/处理失败、gzip 失败，或最终帧编码失败时返回错误。
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

/// IM 聊天 TCP 长连接客户端。
///
/// 客户端保存连接配置、可选写端和后台读任务。调用 [`Self::connect`] 成功后才能登录
/// 或发送；服务端关闭连接、读帧无效、消息回调报错，以及主动断开都会使现有发送端失效。
/// `Drop` 仅同步中止任务并尽力释放写端，不执行异步关闭流程。
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
    /// 使用给定配置创建尚未连接的客户端。
    ///
    /// 创建过程不进行网络 I/O，也不安装消息或断开回调。
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            stream: None,
            reader_task: None,
            handler: None,
            disconnect_handler: None,
        }
    }

    /// 设置收到完整服务端帧时调用的异步处理器。
    ///
    /// 后续调用会替换已有处理器。处理器快照在 [`Self::connect`] 时交给读任务，
    /// 因而连接建立后再次设置只影响下一次连接。处理器返回错误会终止当前读任务。
    pub fn on_message<F, Fut>(&mut self, handler: F)
    where
        F: Fn(u16, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        self.handler = Some(Arc::new(Box::new(move |message_id, content| {
            Box::pin(handler(message_id, content))
        })));
    }

    /// 设置当前连接结束时调用的异步处理器。
    ///
    /// 后续调用会替换已有处理器。后台读任务使用 [`Self::connect`] 时取得的快照；
    /// [`Self::disconnect`] 则使用调用时客户端保存的处理器，因此连接期间替换处理器时，
    /// 两条关闭路径可能观察到不同版本。正常的异步关闭路径会等待处理器完成；
    /// [`Self::force_abort`] 与 `Drop` 不调用它。
    pub fn on_disconnect<F, Fut>(&mut self, handler: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.disconnect_handler = Some(Arc::new(Box::new(move || Box::pin(handler()))));
    }

    /// 连接配置中的聊天服务器并启动后台读任务。
    ///
    /// 本方法先检查旧读任务：仍在运行时拒绝重复连接；已结束时先等待并回收它。
    /// TCP 连接成功后拆分读写端，保存共享写端，并启动持有读端、回调快照和半包缓冲区
    /// 的后台读任务。连接建立本身不会自动发送登录帧。
    ///
    /// # 错误
    ///
    /// 当前读任务仍在运行时返回 [`AppError::AlreadyConnected`]；旧任务 join 失败、
    /// 地址连接失败时也返回错误。若错误发生在新 TCP 连接建立前，客户端保持未建立
    /// 新连接的状态。
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

        // 读任务独占读端，并与所有发送句柄共享同一个可撤销写端。
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

    /// 异步关闭当前连接并停止后台读任务。
    ///
    /// 方法先从客户端移除共享写端并尝试 `shutdown`，再中止仍在运行的读任务并等待
    /// 任务结束。若本次调用实际取走写端或中止读任务，则在资源处理后调用当前客户端上的
    /// 断开处理器；本方法自身最多调用一次，重复调用通常是空操作。若它与后台读任务退出
    /// 并发，两个路径的回调调用并未通过共享标志合并，调用方不应依赖全局“严格一次”语义。
    ///
    /// 写端关闭、任务 join 和断开回调没有超时限制；关闭及 join 错误会被忽略。
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

    /// 同步中止客户端持有的连接资源。
    ///
    /// 本方法是异步断开超时或 `Drop` 时的兜底：中止读任务，并在能够立即取得写端锁时
    /// 移除写端。它不等待任务结束、不执行 TCP `shutdown`，也不调用或等待断开处理器。
    /// 若写端锁正被占用，本次调用无法立即移除写端；外部 [`ChatSender`] 仍可能持有共享
    /// 写端，因此此兜底不保证套接字已经立即关闭。需要完整关闭流程时应优先调用
    /// [`Self::disconnect`]。
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

    /// 在当前连接上发送登录帧。
    ///
    /// 本方法以消息 ID 1100 构造登录 protobuf 客户端帧，取得共享写端锁后完整写入并刷新。
    /// 调用方必须先成功调用 [`Self::connect`]；本方法只发送请求，不等待登录响应。
    ///
    /// # 错误
    ///
    /// 登录帧构造失败、客户端未连接或已经断开，以及 TCP 写入/刷新失败时返回错误。
    /// 本方法没有内置取消或超时；I/O 错误同样可能发生在写入部分帧或完整帧之后，
    /// 不能据返回错误判断服务端是否收到，处理要求见 [`ChatSender::send_cancellable`]。
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

    /// 通过当前连接发送一条应用消息。
    ///
    /// 本方法取得 [`ChatSender`] 并委托给 [`ChatSender::send`]，因此会构造客户端帧、
    /// 等待共享写端锁、写入并刷新。调用方必须先成功连接。
    ///
    /// # 错误
    ///
    /// 客户端未连接或已断开、帧构造失败，或 TCP 写入/刷新失败时返回错误。
    /// 本方法没有内置取消或超时；底层 I/O 错误的写入进度与连接复用风险见
    /// [`ChatSender::send`] 和 [`ChatSender::send_cancellable`]。
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

    /// 返回共享当前写端的可克隆发送器。
    ///
    /// 尚未调用 [`Self::connect`]，或连接状态已从客户端移除时返回 `None`。
    /// 返回 `Some` 只表示取得了共享句柄；连接可能随后断开，实际发送仍可能返回未连接错误。
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

/// 持有 TCP 读端并按顺序分派完整服务端帧的后台任务。
///
/// TCP 读取不对应帧边界，因此任务把每次新增字节追加到 `leftover`。完整帧被逐个消费，
/// 尾部半包继续保留到下一次读取，避免把一次 `read` 误当成一帧或丢失跨读取的正文。
/// 正常 EOF、读取/协议错误或消息处理器返回/抛出错误均会离开循环，随后清理写端并
/// 等待断开回调；`Drop` 作为同步兜底关闭写端，但不等待断开回调。
struct ReadTask {
    reader: OwnedReadHalf,
    stream: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>,
    handler: Option<Arc<MessageHandler>>,
    disconnect_handler: Option<Arc<DisconnectHandler>>,
    body_aes_key: String,
    /// 跨读取累积字节，并在每次处理后保留尚不完整的尾帧。
    leftover: Vec<u8>,
}

impl ReadTask {
    /// 运行读取、增量解帧和受控退出后的连接清理。
    ///
    /// 循环通过 `break` 结束时会移除并关闭共享写端，再等待断开回调。消息处理器 panic
    /// 经 `catch_unwind` 捕获后以协议错误分支 `break`，同样经过尾部清理。
    async fn run(mut self) {
        loop {
            // 每次只追加本轮实际读取的字节，既保留半包，也允许一次读取包含多个帧。
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

    /// 消费当前缓冲区内的所有完整帧。
    ///
    /// 解码成功后先按线长从 `leftover` 排出该帧，再记录消息 ID 9999 的
    /// `ErrrMessage`（解码日志失败也不改变流程），最后等待普通消息处理器完成。
    /// `Incomplete` 保留全部未消费字节并等待下次读取；`Invalid` 返回错误，使读任务终止。
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
            match decode_transport_frame(&self.body_aes_key, &self.leftover) {
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
                        let handler = handler.clone();
                        let mid = frame.message_id;
                        // 用 catch_unwind 兜底：消息处理器 panic 不致于无声终止读任务。
                        // AssertUnwindSafe 保证 Future 跨 catch_unwind 边界后仍可继续 await。
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(mid, frame.content.clone())));
                        match result {
                            Ok(future) => future.await?,
                            Err(panic_err) => {
                                error!(message_id = mid, "Message handler panicked, terminating connection");
                                let msg = if let Some(s) = panic_err.downcast_ref::<&str>() {
                                    s.to_string()
                                } else if let Some(s) = panic_err.downcast_ref::<String>() {
                                    s.clone()
                                } else {
                                    "unknown panic".to_string()
                                };
                                return Err(AppError::TcpFrame(format!("message handler panicked: {msg}")));
                            }
                        }
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

impl Drop for ReadTask {
    fn drop(&mut self) {
        // 同步兜底：读任务因 panic 异常终止时，Tokio 任务已结束，这里仅尝试关闭写端，
        // 无法等待异步断开回调（Drop 是同步的）。正常路径由 run() 尾部处理。
        if let Ok(mut writer) = self.stream.try_lock() {
            if let Some(mut w) = writer.take() {
                let _ = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(
                        tokio::io::AsyncWriteExt::shutdown(&mut w)
                    )
                });
            }
        }
    }
}
