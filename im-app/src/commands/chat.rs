//! 聊天连接与消息查询命令。
//!
//! 本模块把 Tauri 命令、认证会话、TCP 聊天客户端、连接状态机和 SQLite
//! 消息存储串联起来。连接流程按“取得认证会话 → 申请带 generation/attempt
//! 标识的连接许可 → TCP connect → login → 等待 1201 登录成功推送 → 安装客户端
//! → 发布 connected → 启动心跳”推进；任一阶段失败都会按当前门禁清理资源并发布
//! 可确认的终态。断线重连同样受 generation、attempt 和认证会话约束，旧连接不能
//! 覆盖新会话。
//!
//! 入站回调只负责把帧送入有界队列。工作协程串行解码并处理 1201、2202 和 2205：
//! 2202 中仅受监控群的消息写入数据库并尝试发送 `new_message` 事件，但所有成功
//! 处理或无需持久化的消息都会按群汇总后发送 2102 回执。持久化失败的消息不回执；
//! 事件发送失败只记录日志，仍视为已持久化并回执。取消采用 fail-closed 语义，
//! 关闭接收端并丢弃剩余帧，避免陈旧连接继续写库或通知界面。

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::state::AppState;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use prost::Message;
use tauri::{Emitter, State};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// TCP 建连最长等待 15 秒。
const CHAT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 登录请求及 1201 登录成功推送各自最长等待 15 秒。
const CHAT_LOGIN_TIMEOUT: Duration = Duration::from_secs(15);
/// 主动断开及连接失败清理最长等待 1 秒。
const CHAT_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// 心跳或 2102 回执单次发送最长等待 15 秒。
const CHAT_SEND_TIMEOUT: Duration = Duration::from_secs(15);
/// 心跳间隔为 30 秒，短于服务端 60 秒超时窗口。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// 入站帧队列最多容纳 8 项，满载时回调等待以形成背压。
const MESSAGE_QUEUE_CAPACITY: usize = 8;
/// 单个已解码入站帧最大为 8 MiB，超限帧令回调返回错误。
const MAX_QUEUED_MESSAGE_SIZE: usize = 8 * 1024 * 1024;

/// 暴露给前端的群消息。
///
/// 64 位标识以十进制字符串表示，避免 JavaScript 数值精度损失；二进制正文统一使用
/// 标准 Base64。实时消息的 `stored_at` 为 `None`，历史查询结果则包含数据库写入时间。
#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageDto {
    /// 消息 ID 的十进制字符串。
    pub msg_id: String,
    /// 群 ID 的十进制字符串。
    pub group_id: String,
    /// 发送者用户 ID 的十进制字符串。
    pub send_uid: String,
    /// 协议定义的消息类型。
    pub msg_type: i32,
    /// 标准 Base64 编码的原始消息正文。
    pub content_b64: String,
    /// 服务端记录的发送时间。
    pub send_time: i64,
    /// 消息正文的 MD5 摘要。
    pub content_md5: String,
    /// 数据库写入时间；实时推送尚无该值。
    pub stored_at: Option<i64>,
}

fn stored_message_parts(
    message: &im_proto::GroupMessage,
) -> (im_store::message::MessageRecord, MessageDto) {
    let dto = MessageDto {
        msg_id: message.msg_id.to_string(),
        group_id: message.group_id.to_string(),
        send_uid: message.send_uid.to_string(),
        msg_type: message.msg_type,
        content_b64: STANDARD.encode(&message.content),
        send_time: message.send_time,
        content_md5: message.content_md5.clone(),
        stored_at: None,
    };
    let record = im_store::message::MessageRecord {
        msg_id: message.msg_id,
        group_id: message.group_id,
        send_uid: message.send_uid,
        msg_type: message.msg_type,
        content: message.content.clone(),
        send_time: message.send_time,
        content_md5: message.content_md5.clone(),
        raw_proto: Some(message.encode_to_vec()),
    };
    (record, dto)
}

fn message_dto_from_row(row: im_store::message::MessageRow) -> MessageDto {
    MessageDto {
        msg_id: row.msg_id.to_string(),
        group_id: row.group_id.to_string(),
        send_uid: row.send_uid.to_string(),
        msg_type: row.msg_type,
        content_b64: STANDARD.encode(row.content),
        send_time: row.send_time,
        content_md5: row.content_md5,
        stored_at: Some(row.stored_at),
    }
}

/// 一次连接及其后台任务所需的共享应用资源快照。
#[derive(Clone)]
struct ConnectionContext {
    config: Arc<tokio::sync::RwLock<im_common::config::AppConfig>>,
    db: Arc<im_store::SqliteStore>,
    chat_client: Arc<crate::state::ClientSlot>,
    auth_session: Arc<tokio::sync::RwLock<Option<crate::state::AuthSession>>>,
    monitoring_groups: Arc<tokio::sync::RwLock<std::collections::HashSet<i64>>>,
    coordinator: Arc<crate::state::ConnectionCoordinator>,
    connected: Arc<tokio::sync::RwLock<bool>>,
    shutdown: CancellationToken,
    app_handle: tauri::AppHandle,
}

impl ConnectionContext {
    fn from_state(state: &AppState) -> Self {
        Self {
            config: state.config.clone(),
            db: state.db.clone(),
            chat_client: state.chat_client.clone(),
            auth_session: state.auth_session.clone(),
            monitoring_groups: state.monitoring_groups.clone(),
            coordinator: state.connection_coordinator.clone(),
            connected: state.connected.clone(),
            shutdown: state.shutdown.clone(),
            app_handle: state.app_handle().clone(),
        }
    }
}

/// 保证未正常收尾的连接尝试最终撤销的 RAII 守卫。
///
/// 守卫只有在成功安装或显式失败处理后才会解除。若命令 future 被取消或提前退出，
/// `Drop` 会派生异步清理任务；清理仍经 generation/attempt 门禁，只撤销当前尝试，
/// 并尝试断开其拥有的客户端、重标认证会话及发布 disconnected。异步清理不保证在
/// `drop` 返回前完成，断开超时也只记录警告。
struct ConnectionAttemptGuard {
    armed: bool,
    generation: u64,
    attempt_id: u64,
    coordinator: Arc<crate::state::ConnectionCoordinator>,
    client_slot: Arc<crate::state::ClientSlot>,
    auth_session: Arc<tokio::sync::RwLock<Option<crate::state::AuthSession>>>,
    connected: Arc<tokio::sync::RwLock<bool>>,
    app_handle: Option<tauri::AppHandle>,
    cleanup_finished: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ConnectionAttemptGuard {
    fn new(
        generation: u64,
        attempt_id: u64,
        coordinator: Arc<crate::state::ConnectionCoordinator>,
        client_slot: Arc<crate::state::ClientSlot>,
        auth_session: Arc<tokio::sync::RwLock<Option<crate::state::AuthSession>>>,
        connected: Arc<tokio::sync::RwLock<bool>>,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            armed: true,
            generation,
            attempt_id,
            coordinator,
            client_slot,
            auth_session,
            connected,
            app_handle,
            cleanup_finished: None,
        }
    }

    #[cfg(test)]
    fn new_for_test(
        generation: u64,
        attempt_id: u64,
        coordinator: Arc<crate::state::ConnectionCoordinator>,
        client_slot: Arc<crate::state::ClientSlot>,
        auth_session: Arc<tokio::sync::RwLock<Option<crate::state::AuthSession>>>,
        connected: Arc<tokio::sync::RwLock<bool>>,
    ) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (cleanup_finished, receiver) = tokio::sync::oneshot::channel();
        (
            Self {
                armed: true,
                generation,
                attempt_id,
                coordinator,
                client_slot,
                auth_session,
                connected,
                app_handle: None,
                cleanup_finished: Some(cleanup_finished),
            },
            receiver,
        )
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ConnectionAttemptGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let coordinator = self.coordinator.clone();
        let generation = self.generation;
        let attempt_id = self.attempt_id;
        let client_slot = self.client_slot.clone();
        let auth_session = self.auth_session.clone();
        let connected = self.connected.clone();
        let app_handle = self.app_handle.clone();
        let cleanup_finished = self.cleanup_finished.take();
        tokio::spawn(async move {
            let next_generation = coordinator
                .cancel_and_advance_if_current(generation, attempt_id)
                .await;
            if let Ok(Some(next_generation)) = next_generation {
                let disconnect_result = disconnect_owned_chat_client_with_timeout(
                    &client_slot,
                    crate::state::ConnectionAttemptKey::new(generation, attempt_id),
                    CHAT_DISCONNECT_TIMEOUT,
                )
                .await;
                coordinator
                    .retag_session_if_current(next_generation, &auth_session)
                    .await;
                publish_disconnected_status_if_current(
                    &coordinator,
                    next_generation,
                    app_handle.as_ref(),
                    &connected,
                )
                .await;
                if let Err(error) = disconnect_result {
                    tracing::warn!("Dropped connect attempt cleanup failed: {error}");
                }
            }
            if let Some(cleanup_finished) = cleanup_finished {
                let _ = cleanup_finished.send(());
            }
        });
    }
}

/// 从网络回调转交给串行工作协程的已解码帧。
struct IncomingFrame {
    message_id: u16,
    content: Vec<u8>,
}

/// 校验帧大小后送入有界队列。
///
/// 队列已满时等待容量而不丢帧；等待期间若连接取消，或 receiver 已关闭，则返回
/// TCP 帧错误。正文超出 8 MiB 时不会进入队列。
async fn enqueue_incoming_frame(
    sender: &mpsc::Sender<IncomingFrame>,
    frame: IncomingFrame,
    cancellation: &CancellationToken,
) -> Result<(), im_common::error::AppError> {
    if frame.content.len() > MAX_QUEUED_MESSAGE_SIZE {
        return Err(im_common::error::AppError::TcpFrame(format!(
            "decoded message size {} exceeds queue limit {}",
            frame.content.len(),
            MAX_QUEUED_MESSAGE_SIZE
        )));
    }
    let message_id = frame.message_id;
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(im_common::error::AppError::TcpFrame(format!(
            "connection cancelled before message {} could be delivered",
            message_id
        ))),
        result = sender.send(frame) => result.map_err(|_| {
            im_common::error::AppError::TcpFrame(format!(
                "message queue closed before message {} could be delivered",
                message_id
            ))
        }),
    }
}

#[async_trait::async_trait]
/// 将协议分派与应用副作用隔离，便于精确验证处理顺序。
trait MessageEffects: Send + Sync {
    /// 查询该群在处理当前消息时是否仍处于监控集合。
    async fn is_monitored(&self, group_id: i64) -> bool;
    /// 先写入 SQLite，再尝试发送 `new_message`；仅写库失败时返回 `false`。
    async fn persist_and_emit(&self, message: im_proto::GroupMessage) -> bool;
    /// 按群发送包含完整消息 ID 列表的 2102 接收回执。
    async fn acknowledge_group_messages(
        &self,
        group_id: i64,
        msg_ids: Vec<i64>,
    ) -> Result<(), im_common::error::AppError>;
}

/// 使用真实应用状态执行监控查询、持久化、事件和回执副作用。
struct ConnectionMessageEffects {
    context: ConnectionContext,
    sender: Arc<tokio::sync::OnceCell<im_chat::ChatSender>>,
    cancellation: CancellationToken,
}

#[async_trait::async_trait]
impl MessageEffects for ConnectionMessageEffects {
    async fn is_monitored(&self, group_id: i64) -> bool {
        self.context
            .monitoring_groups
            .read()
            .await
            .contains(&group_id)
    }

    async fn persist_and_emit(&self, message: im_proto::GroupMessage) -> bool {
        let (record, dto) = stored_message_parts(&message);
        if let Err(error) = self.context.db.messages.insert(&record).await {
            tracing::error!("Failed to insert message: {error}");
            return false;
        }
        if let Err(error) = self.context.app_handle.emit("new_message", &dto) {
            tracing::error!("Failed to emit new_message: {error}");
        }
        true
    }

    async fn acknowledge_group_messages(
        &self,
        group_id: i64,
        msg_ids: Vec<i64>,
    ) -> Result<(), im_common::error::AppError> {
        let sender = self.sender.get().ok_or_else(|| {
            im_common::error::AppError::TcpFrame(
                "chat sender unavailable for group receipt".to_string(),
            )
        })?;
        let receipt = im_proto::ReceiveGroupMessage {
            msg_id: msg_ids,
            group_id,
        };
        sender
            .send_cancellable(
                2102,
                &receipt.encode_to_vec(),
                &self.cancellation,
                CHAT_SEND_TIMEOUT,
            )
            .await
            .map_err(|error| im_common::error::AppError::TcpFrame(error.to_string()))
    }
}

/// 已完成 connect、login 并收到 1201，但尚待状态机安装的连接资源。
struct EstablishedConnection {
    client: im_chat::ChatClient,
    installed: Arc<AtomicBool>,
    connection_lost: Arc<AtomicBool>,
    connection_cancellation: CancellationToken,
    _message_worker: tokio::task::JoinHandle<()>,
}

#[tauri::command]
/// 建立聊天连接并在成功后启动心跳。
///
/// `state` 提供当前认证会话、连接协调器、客户端槽位、数据库和事件句柄。命令依次
/// 完成连接阶段并等待 1201；成功返回 `Ok(())`。未登录、重复连接、阶段超时、取消、
/// 协议确认失败或安装门禁失效时返回字符串错误。过程中会更新 `connected` 并发送
/// `connection_status`；失败路径尝试取消工作协程、断开本次拥有的客户端和发布
/// `disconnected`，但网络断开受超时约束，事件发送失败仅记日志。
pub async fn connect_chat(state: State<'_, AppState>) -> Result<(), String> {
    connect_chat_inner(&state).await
}

#[tauri::command]
/// 返回协调器当前连接状态。
///
/// `state` 只用于读取连接阶段；`Connected` 返回 `"connected"`，
/// `Connecting`/`Reconnecting` 返回 `"connecting"`，其余返回 `"disconnected"`。
/// 本命令无网络、数据库或广播副作用；当前实现保留 `Result` 以适配 Tauri 命令接口。
pub async fn get_connection_status(state: State<'_, AppState>) -> Result<String, String> {
    let status = match state.connection_coordinator.phase().await {
        crate::state::ConnectionPhase::Connected => "connected",
        crate::state::ConnectionPhase::Connecting | crate::state::ConnectionPhase::Reconnecting => {
            "connecting"
        }
        crate::state::ConnectionPhase::Idle => "disconnected",
    };
    Ok(status.to_string())
}

/// 执行连接命令的完整受门禁编排。
///
/// 先验证认证会话并申请许可，再发布 connecting，链接本代取消信号与应用 shutdown，
/// 建连、登录并等待 1201。成功安装客户端后复核断线标志和所有权，发布 connected
/// 并启动心跳；过期、安装失败或中途断线则取消连接任务并清理本次资源。守卫覆盖命令
/// future 被外部丢弃的路径，避免状态永久停留在 connecting。
async fn connect_chat_inner(state: &AppState) -> Result<(), String> {
    let auth_session = authenticated_session_for_connect(&state.auth_session).await?;
    let permit = begin_connection_attempt(
        &state.connection_coordinator,
        &state.chat_client,
        auth_session.generation,
    )
    .await?;
    let context = ConnectionContext::from_state(state);
    let generation = permit.generation();
    let attempt_id = permit.attempt_id();
    let mut attempt_guard = ConnectionAttemptGuard::new(
        generation,
        attempt_id,
        context.coordinator.clone(),
        context.chat_client.clone(),
        context.auth_session.clone(),
        context.connected.clone(),
        Some(context.app_handle.clone()),
    );
    if !publish_connecting_status_if_current(
        &state.connection_coordinator,
        generation,
        attempt_id,
        state.app_handle(),
        &state.connected,
    )
    .await
    {
        return Err("Connection operation became stale before starting".to_string());
    }
    let generation_cancellation =
        linked_cancellation(permit.cancellation_token(), context.shutdown.clone());
    let established = establish_connection(
        context.clone(),
        auth_session.clone(),
        generation,
        attempt_id,
        generation_cancellation.clone(),
    )
    .await;
    let established = match established {
        Ok(established) => established,
        Err(error) => {
            fail_initial_connection_and_publish(
                &state.connection_coordinator,
                generation,
                attempt_id,
                &state.connected,
                |status| state.app_handle().emit("connection_status", status),
            )
            .await;
            attempt_guard.disarm();
            return Err(error);
        }
    };

    let EstablishedConnection {
        client: chat_client,
        installed,
        connection_lost,
        connection_cancellation,
        _message_worker,
    } = established;
    let install_result = state
        .connection_coordinator
        .install_if_current(
            &permit,
            &auth_session,
            &state.auth_session,
            &state.chat_client,
            &installed,
            chat_client,
        )
        .await;
    state
        .connection_coordinator
        .finish_connect(generation, attempt_id)
        .await;
    match install_result {
        Ok(()) => {
            if connection_lost.load(Ordering::Acquire) {
                handle_connection_loss(
                    context,
                    auth_session,
                    generation,
                    attempt_id,
                    generation_cancellation,
                )
                .await;
                attempt_guard.disarm();
                return Err("Chat disconnected during connection installation".to_string());
            }
            let sender = chat_sender_if_owned(
                &state.chat_client,
                crate::state::ConnectionAttemptKey::new(generation, attempt_id),
            )
            .await
            .ok_or_else(|| "Installed chat client ownership changed".to_string())?;
            let published = publish_connected_status_if_current(
                &state.connection_coordinator,
                generation,
                state.app_handle(),
                &state.connected,
            )
            .await;
            if published {
                start_heartbeat(
                    context,
                    auth_session,
                    generation,
                    attempt_id,
                    generation_cancellation,
                    connection_cancellation,
                    sender,
                );
                attempt_guard.disarm();
                Ok(())
            } else {
                connection_cancellation.cancel();
                let _ = disconnect_owned_chat_client_with_timeout(
                    &state.chat_client,
                    crate::state::ConnectionAttemptKey::new(generation, attempt_id),
                    CHAT_DISCONNECT_TIMEOUT,
                )
                .await;
                attempt_guard.disarm();
                Err("Chat disconnected before connection completed".to_string())
            }
        }
        Err((error, mut stale_client)) => {
            connection_cancellation.cancel();
            fail_initial_connection_and_publish(
                &state.connection_coordinator,
                generation,
                attempt_id,
                &state.connected,
                |status| state.app_handle().emit("connection_status", status),
            )
            .await;
            disconnect_local_client(&mut stale_client).await;
            attempt_guard.disarm();
            Err(error)
        }
    }
}

/// 为指定认证代启动后台自动连接。
///
/// 仅在同一 generation 的认证会话仍存在且应用未 shutdown 时重试；成功、
/// `AlreadyConnected` 或 `Connecting` 立即结束，其他错误按指数退避继续。
pub(crate) fn start_automatic_connection(state: &AppState, generation: u64) {
    let state = state.clone();
    let auth_session = state.auth_session.clone();
    let shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        let connect_state = state.clone();
        retry_automatic_connection(
            generation,
            auth_session,
            shutdown,
            move || {
                let state = connect_state.clone();
                async move { connect_chat_inner(&state).await }
            },
            tokio::time::sleep,
        )
        .await;
    });
}

/// 执行可替换连接动作与休眠器的自动连接重试循环。
///
/// 每轮调用前复核认证 generation 和 shutdown；普通错误后按指数退避，休眠阶段也
/// 响应 shutdown。`AlreadyConnected` 与 `Connecting` 表示已有流程接管，因此终止
/// 当前循环而不是制造第二个连接尝试。
async fn retry_automatic_connection<Connect, ConnectFuture, Sleep, SleepFuture>(
    generation: u64,
    auth_session: Arc<tokio::sync::RwLock<Option<crate::state::AuthSession>>>,
    shutdown: CancellationToken,
    mut connect: Connect,
    mut sleep: Sleep,
) where
    Connect: FnMut() -> ConnectFuture,
    ConnectFuture: Future<Output = Result<(), String>>,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: Future<Output = ()>,
{
    let mut backoff = im_chat::reconnect::ExponentialBackoff::default();
    loop {
        let is_current = auth_session
            .read()
            .await
            .as_ref()
            .is_some_and(|session| session.generation == generation);
        if !is_current || shutdown.is_cancelled() {
            return;
        }

        match connect().await {
            Ok(()) => return,
            Err(error) if error == "AlreadyConnected" || error == "Connecting" => return,
            Err(error) => tracing::warn!(
                generation,
                %error,
                "Automatic chat connection failed; retrying"
            ),
        }

        let delay = backoff
            .next()
            .expect("exponential connection backoff is infinite");
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            _ = sleep(delay) => {}
        }
    }
}

/// 读取连接所需的认证会话；不存在时返回 `Not logged in`。
pub(crate) async fn authenticated_session_for_connect(
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
) -> Result<crate::state::AuthSession, String> {
    auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())
}

/// 将当前连接代的取消信号与应用 shutdown 合并为一个单向令牌。
fn linked_cancellation(
    generation_cancellation: CancellationToken,
    shutdown: CancellationToken,
) -> CancellationToken {
    let linked = CancellationToken::new();
    let output = linked.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = generation_cancellation.cancelled() => {}
            _ = shutdown.cancelled() => {}
        }
        output.cancel();
    });
    linked
}

/// 建立一个尚未安装到全局槽位的聊天连接。
///
/// 先创建容量为 8 的 mpsc 队列和串行消息 worker，再安装入站 handler 与断线回调。
/// 网络阶段依次执行 connect、取得回执 sender、login，并等待 worker 解码有效 1201。
/// 每阶段受取消和超时控制。断线回调先标记 loss 并取消本连接；仅客户端已经安装时
/// 才进入重连处理。失败时取消 worker、尝试断开本地客户端，并最多等待 1 秒回收
/// worker；这里不承诺清理原子完成或底层连接立即关闭。
async fn establish_connection(
    context: ConnectionContext,
    auth_session: crate::state::AuthSession,
    generation: u64,
    attempt_id: u64,
    generation_cancellation: CancellationToken,
) -> Result<EstablishedConnection, String> {
    let connection_cancellation = generation_cancellation.child_token();
    let installed = Arc::new(AtomicBool::new(false));
    let connection_lost = Arc::new(AtomicBool::new(false));
    let (frame_sender, frame_receiver) = mpsc::channel(MESSAGE_QUEUE_CAPACITY);
    let (login_sender, login_receiver) = tokio::sync::oneshot::channel();
    let worker_cancellation = connection_cancellation.clone();
    let receipt_sender = Arc::new(tokio::sync::OnceCell::new());
    let worker_effects = Arc::new(ConnectionMessageEffects {
        context: context.clone(),
        sender: receipt_sender.clone(),
        cancellation: connection_cancellation.clone(),
    });
    let message_worker = tokio::spawn(async move {
        run_message_worker(
            frame_receiver,
            worker_effects,
            worker_cancellation,
            login_sender,
        )
        .await;
    });

    let config = context.config.read().await.clone();
    let mut chat_client = im_chat::ChatClient::new(config);
    let message_cancellation = connection_cancellation.clone();
    chat_client.on_message(move |message_id, content| {
        let sender = frame_sender.clone();
        let cancellation = message_cancellation.clone();
        async move {
            if cancellation.is_cancelled() {
                return Ok(());
            }
            enqueue_incoming_frame(
                &sender,
                IncomingFrame {
                    message_id,
                    content,
                },
                &cancellation,
            )
            .await
        }
    });

    let disconnect_context = context.clone();
    let disconnect_session = auth_session.clone();
    let disconnect_generation_cancellation = generation_cancellation.clone();
    let disconnect_connection_cancellation = connection_cancellation.clone();
    let disconnect_installed = installed.clone();
    let disconnect_connection_lost = connection_lost.clone();
    chat_client.on_disconnect(move || {
        let context = disconnect_context.clone();
        let session = disconnect_session.clone();
        let generation_cancellation = disconnect_generation_cancellation.clone();
        let connection_cancellation = disconnect_connection_cancellation.clone();
        let installed = disconnect_installed.clone();
        let connection_lost = disconnect_connection_lost.clone();
        async move {
            connection_lost.store(true, Ordering::Release);
            connection_cancellation.cancel();
            if installed.load(Ordering::Acquire) {
                handle_connection_loss(
                    context,
                    session,
                    generation,
                    attempt_id,
                    generation_cancellation,
                )
                .await;
            }
        }
    });

    let network_result = run_cancellable_with_timeout(
        "Chat connect",
        CHAT_CONNECT_TIMEOUT,
        &connection_cancellation,
        chat_client.connect(),
    )
    .await;
    let network_result = match network_result {
        Ok(()) => {
            let sender = chat_client
                .sender()
                .ok_or_else(|| "Chat sender unavailable after connect".to_string())?;
            receipt_sender
                .set(sender)
                .map_err(|_| "Chat receipt sender initialized twice".to_string())?;
            run_cancellable_with_timeout(
                "Chat login",
                CHAT_LOGIN_TIMEOUT,
                &connection_cancellation,
                chat_client.login(&auth_session.token, auth_session.uid),
            )
            .await
        }
        Err(error) => Err(error),
    };
    let network_result = match network_result {
        Ok(()) => {
            run_cancellable_with_timeout(
                "Chat login acknowledgement",
                CHAT_LOGIN_TIMEOUT,
                &connection_cancellation,
                async {
                    login_receiver
                        .await
                        .map_err(|_| "Connection closed before login acknowledgement")
                },
            )
            .await
        }
        Err(error) => Err(error),
    };

    if let Err(error) = network_result {
        connection_cancellation.cancel();
        disconnect_local_client(&mut chat_client).await;
        let _ = tokio::time::timeout(CHAT_DISCONNECT_TIMEOUT, message_worker).await;
        return Err(error);
    }

    Ok(EstablishedConnection {
        client: chat_client,
        installed,
        connection_lost,
        connection_cancellation,
        _message_worker: message_worker,
    })
}

/// 驱动当前连接的串行消息处理循环。
async fn run_message_worker(
    receiver: mpsc::Receiver<IncomingFrame>,
    effects: Arc<dyn MessageEffects>,
    cancellation: CancellationToken,
    login_sender: tokio::sync::oneshot::Sender<()>,
) {
    run_message_worker_with_effects(receiver, effects, cancellation, login_sender).await;
}

/// 按入队顺序处理聊天推送及其副作用。
///
/// 1201 必须能解码为 `PushLoginSuccessMessage` 才完成登录 oneshot；解码失败会取消
/// 连接并停止 worker。2202 解码失败只丢弃该帧并继续；成功后逐条读取最新监控状态，
/// 受监控消息按“数据库 insert → 尝试 emit”处理，写库失败的不进入回执，未监控消息
/// 不落库但仍回执。之后按群 ID 有序发送覆盖全部可处理消息的 2102；任一回执失败会
/// 取消连接并停止后续处理。2205 当前仅记录预留日志。取消分支优先，退出时关闭
/// receiver 并丢弃排队帧，防止陈旧连接产生写库或事件副作用。
async fn run_message_worker_with_effects(
    mut receiver: mpsc::Receiver<IncomingFrame>,
    effects: Arc<dyn MessageEffects>,
    cancellation: CancellationToken,
    login_sender: tokio::sync::oneshot::Sender<()>,
) {
    let mut login_sender = Some(login_sender);
    loop {
        let frame = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            frame = receiver.recv() => match frame {
                Some(frame) => frame,
                None => break,
            }
        };
        match frame.message_id {
            im_chat::heartbeat::PUSH_LOGIN_SUCCESS => {
                match im_proto::PushLoginSuccessMessage::decode(frame.content.as_slice()) {
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!("Failed to decode PushLoginSuccessMessage: {error}");
                        cancellation.cancel();
                        break;
                    }
                }
                if let Some(sender) = login_sender.take() {
                    let _ = sender.send(());
                }
            }
            im_chat::heartbeat::PUSH_GROUP_MESSAGE => {
                let push = match im_proto::PushGroupMessage::decode(frame.content.as_slice()) {
                    Ok(push) => push,
                    Err(error) => {
                        tracing::warn!("Failed to decode PushGroupMessage: {error}");
                        continue;
                    }
                };
                let mut receipts = std::collections::BTreeMap::<i64, Vec<i64>>::new();
                for group_message in push.group_msg {
                    let group_id = group_message.group_id;
                    let msg_id = group_message.msg_id;
                    let monitored = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return,
                        monitored = effects.is_monitored(group_id) => monitored,
                    };
                    let handled = if monitored {
                        tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => return,
                            persisted = effects.persist_and_emit(group_message) => persisted,
                        }
                    } else {
                        true
                    };
                    if handled {
                        receipts.entry(group_id).or_default().push(msg_id);
                    }
                }
                for (group_id, msg_ids) in receipts {
                    let receipt_result = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return,
                        result = effects.acknowledge_group_messages(group_id, msg_ids) => result,
                    };
                    if let Err(error) = receipt_result {
                        tracing::error!(
                            group_id,
                            "Failed to acknowledge received group messages: {error}"
                        );
                        cancellation.cancel();
                        return;
                    }
                }
            }
            im_chat::heartbeat::PUSH_RECALL_GROUP_MESSAGE => {
                tracing::info!("Received group-message recall push (2205); handling reserved");
            }
            message_id => tracing::debug!("Ignoring unsupported chat message {message_id}"),
        }
    }
    // 取消采用故障关闭（fail-closed）策略：丢弃接收端会清空排队帧，
    // 从而阻止陈旧连接继续写入 SQLite 或发送界面事件。
    receiver.close();
}

/// 启动 30 秒周期心跳，并在发送失败时进入受门禁的断线处理。
fn start_heartbeat(
    context: ConnectionContext,
    auth_session: crate::state::AuthSession,
    generation: u64,
    attempt_id: u64,
    generation_cancellation: CancellationToken,
    connection_cancellation: CancellationToken,
    sender: im_chat::ChatSender,
) {
    tokio::spawn(async move {
        let send_cancellation = connection_cancellation.clone();
        let result = im_chat::heartbeat::heartbeat_loop(
            HEARTBEAT_INTERVAL,
            connection_cancellation.clone(),
            move || {
                let sender = sender.clone();
                let cancellation = send_cancellation.clone();
                async move {
                    let (message_id, payload) = im_chat::heartbeat::heartbeat_message();
                    sender
                        .send_cancellable(message_id, payload, &cancellation, CHAT_SEND_TIMEOUT)
                        .await
                        .map_err(|error| error.to_string())
                }
            },
        )
        .await;
        if let Err(error) = result {
            tracing::warn!("Heartbeat failed: {error}");
            connection_cancellation.cancel();
            handle_connection_loss(
                context,
                auth_session,
                generation,
                attempt_id,
                generation_cancellation,
            )
            .await;
        }
    });
}

/// 将当前连接切换为 reconnecting、释放其客户端并在释放后启动重连。
///
/// generation/attempt 不再匹配时直接忽略；返回仅表示清理和重连任务已安排，不表示
/// 客户端已经立即断开或重连已经完成。
fn handle_connection_loss(
    context: ConnectionContext,
    auth_session: crate::state::AuthSession,
    generation: u64,
    attempt_id: u64,
    generation_cancellation: CancellationToken,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let app_handle = context.app_handle.clone();
        let connected = context.connected.clone();
        if !context
            .coordinator
            .begin_reconnect_and_publish_if_current(generation, attempt_id, || async move {
                apply_connecting_status(&app_handle, &connected).await;
            })
            .await
        {
            return;
        }

        let disconnected_client = take_installed_client_if_owned(
            &context.chat_client,
            crate::state::ConnectionAttemptKey::new(generation, attempt_id),
        )
        .await;
        let (cleanup_release, cleanup_released) = tokio::sync::oneshot::channel();
        let (cleanup_done, cleanup_finished) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = cleanup_released.await;
            drop(disconnected_client);
            let _ = cleanup_done.send(());
        });
        let reconnect_context = context.clone();
        tokio::spawn(async move {
            let _ = cleanup_finished.await;
            run_reconnect_loop(
                reconnect_context,
                auth_session,
                generation,
                attempt_id,
                generation_cancellation,
            )
            .await;
        });
        let _ = cleanup_release.send(());
    })
}

/// 仅在槽位客户端属于指定 generation/attempt 时将其取出。
async fn take_installed_client_if_owned(
    client_slot: &crate::state::ClientSlot,
    owner: crate::state::ConnectionAttemptKey,
) -> Option<im_chat::ChatClient> {
    let mut slot = client_slot.lock().await;
    let is_owned = slot
        .as_ref()
        .is_some_and(|installed| installed.key() == owner);
    is_owned.then(|| slot.take().expect("owned client checked above").client)
}

/// 按指数退避重建连接，并仅在原 generation/attempt 仍有效时安装。
///
/// 安装后再次检查连接是否已丢失，再按当前所有权取得 sender、发布 connected 并
/// 恢复心跳；陈旧结果会被取消并尝试断开，不能覆盖较新的会话或连接。
async fn run_reconnect_loop(
    context: ConnectionContext,
    auth_session: crate::state::AuthSession,
    generation: u64,
    attempt_id: u64,
    generation_cancellation: CancellationToken,
) {
    let action_context = context.clone();
    let action_session = auth_session.clone();
    let action_cancellation = generation_cancellation.clone();
    let established = im_chat::reconnect::reconnect_loop(
        generation_cancellation.clone(),
        im_chat::reconnect::ExponentialBackoff::default(),
        move || {
            establish_connection(
                action_context.clone(),
                action_session.clone(),
                generation,
                attempt_id,
                action_cancellation.clone(),
            )
        },
        tokio::time::sleep,
    )
    .await;
    let Some(established) = established else {
        return;
    };
    let EstablishedConnection {
        client,
        installed,
        connection_lost,
        connection_cancellation,
        _message_worker,
    } = established;
    let install = context
        .coordinator
        .install_reconnected_if_current(
            crate::state::ConnectionAttemptKey::new(generation, attempt_id),
            &auth_session,
            &context.auth_session,
            &context.chat_client,
            &installed,
            client,
        )
        .await;
    match install {
        Ok(()) => {
            if connection_lost.load(Ordering::Acquire) {
                handle_connection_loss(
                    context,
                    auth_session,
                    generation,
                    attempt_id,
                    generation_cancellation,
                )
                .await;
                return;
            }
            let Some(sender) = chat_sender_if_owned(
                &context.chat_client,
                crate::state::ConnectionAttemptKey::new(generation, attempt_id),
            )
            .await
            else {
                tracing::debug!("Reconnected client ownership changed before heartbeat start");
                return;
            };
            let app_handle = context.app_handle.clone();
            let connected = context.connected.clone();
            if publish_connected_status_if_current(
                &context.coordinator,
                generation,
                &app_handle,
                &connected,
            )
            .await
            {
                start_heartbeat(
                    context,
                    auth_session,
                    generation,
                    attempt_id,
                    generation_cancellation,
                    connection_cancellation,
                    sender,
                );
            }
        }
        Err((error, mut stale_client)) => {
            connection_cancellation.cancel();
            tracing::debug!("Discarding stale reconnected client: {error}");
            disconnect_local_client(&mut stale_client).await;
        }
    }
}

/// 仅从指定 generation/attempt 所拥有的客户端取得发送句柄。
async fn chat_sender_if_owned(
    client_slot: &crate::state::ClientSlot,
    owner: crate::state::ConnectionAttemptKey,
) -> Option<im_chat::ChatSender> {
    let slot = client_slot.lock().await;
    slot.as_ref()
        .filter(|installed| installed.key() == owner)
        .and_then(|installed| installed.client.sender())
}

/// 在客户端槽位为空时申请连接许可。
///
/// 已安装客户端返回 `AlreadyConnected`；协调器已有连接流程返回 `Connecting`。
pub(crate) async fn begin_connection_attempt(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
    generation: u64,
) -> Result<crate::state::ConnectionPermit, String> {
    if client_slot.lock().await.is_some() {
        return Err("AlreadyConnected".to_string());
    }
    coordinator
        .begin_connect(generation)
        .await
        .map_err(|error| {
            if error.contains("already in progress") {
                "Connecting".to_string()
            } else {
                error
            }
        })
}

/// 尝试优雅断开局部客户端，1 秒超时后强制中止其资源。
async fn disconnect_local_client(client: &mut im_chat::ChatClient) {
    if tokio::time::timeout(CHAT_DISCONNECT_TIMEOUT, client.disconnect())
        .await
        .is_err()
    {
        client.force_abort();
    }
}

/// 在显式取消与超时之间竞速执行操作。
///
/// `tokio::select!` 对取消分支使用偏置，因此已取消时返回取消错误；超时或底层错误
/// 转成带操作名的字符串。本函数只停止等待并丢弃 future，不保证外部资源立即释放。
async fn run_cancellable_with_timeout<T, E, F>(
    operation_name: &str,
    timeout: Duration,
    cancellation: &CancellationToken,
    operation: F,
) -> Result<T, String>
where
    E: Display,
    F: Future<Output = Result<T, E>>,
{
    tokio::select! {
        _ = cancellation.cancelled() => Err(format!("{operation_name} cancelled")),
        result = tokio::time::timeout(timeout, operation) => {
            match result {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Err(format!("{operation_name} timed out after {timeout:?}")),
            }
        }
    }
}

/// 先更新兼容布尔状态，再尽力广播对应字符串状态。
///
/// 广播失败只记录警告，已写入的状态不会回滚。
async fn mark_connection_status_and_broadcast<F, E>(
    connected: &tokio::sync::RwLock<bool>,
    value: bool,
    status: &'static str,
    broadcast: F,
) where
    E: Display,
    F: FnOnce(&'static str) -> Result<(), E>,
{
    *connected.write().await = value;
    if let Err(error) = broadcast(status) {
        tracing::warn!("Failed to broadcast connection status {status}: {error}");
    }
}

/// 将兼容布尔状态设为断开并尝试广播；广播失败不回滚状态。
pub(crate) async fn mark_disconnected_and_broadcast<F, E>(
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) where
    E: Display,
    F: FnOnce(&'static str) -> Result<(), E>,
{
    mark_connection_status_and_broadcast(connected, false, "disconnected", broadcast).await;
}

/// 将兼容布尔状态设为已连接并尝试广播；广播失败不回滚状态。
pub(crate) async fn mark_connected_and_broadcast<F, E>(
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) where
    E: Display,
    F: FnOnce(&'static str) -> Result<(), E>,
{
    mark_connection_status_and_broadcast(connected, true, "connected", broadcast).await;
}

/// 将兼容布尔状态设为未连接并尝试广播 connecting。
pub(crate) async fn mark_connecting_and_broadcast<F, E>(
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) where
    E: Display,
    F: FnOnce(&'static str) -> Result<(), E>,
{
    mark_connection_status_and_broadcast(connected, false, "connecting", broadcast).await;
}

async fn apply_connected_status(
    app_handle: &tauri::AppHandle,
    connected: &tokio::sync::RwLock<bool>,
) {
    mark_connected_and_broadcast(connected, |status| {
        app_handle.emit("connection_status", status)
    })
    .await
}

async fn apply_connecting_status(
    app_handle: &tauri::AppHandle,
    connected: &tokio::sync::RwLock<bool>,
) {
    mark_connecting_and_broadcast(connected, |status| {
        app_handle.emit("connection_status", status)
    })
    .await
}

async fn publish_connecting_status_if_current(
    coordinator: &crate::state::ConnectionCoordinator,
    generation: u64,
    attempt_id: u64,
    app_handle: &tauri::AppHandle,
    connected: &tokio::sync::RwLock<bool>,
) -> bool {
    coordinator
        .publish_connecting_if_current(generation, attempt_id, || async {
            apply_connecting_status(app_handle, connected).await;
        })
        .await
}

/// 仅在 generation 仍为当前值时发布 disconnected。
pub(crate) async fn publish_disconnected_status_if_current(
    coordinator: &crate::state::ConnectionCoordinator,
    generation: u64,
    app_handle: Option<&tauri::AppHandle>,
    connected: &tokio::sync::RwLock<bool>,
) -> bool {
    publish_disconnected_status_with_if_current(coordinator, generation, connected, |status| {
        match app_handle {
            Some(app_handle) => app_handle
                .emit("connection_status", status)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    })
    .await
}

async fn publish_disconnected_status_with_if_current<F, E>(
    coordinator: &crate::state::ConnectionCoordinator,
    generation: u64,
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) -> bool
where
    F: FnOnce(&'static str) -> Result<(), E>,
    E: Display,
{
    coordinator
        .publish_disconnected_if_current(generation, || async {
            mark_disconnected_and_broadcast(connected, broadcast).await;
        })
        .await
}

async fn fail_initial_connection_and_publish<F, E>(
    coordinator: &crate::state::ConnectionCoordinator,
    generation: u64,
    attempt_id: u64,
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) -> bool
where
    F: FnOnce(&'static str) -> Result<(), E>,
    E: Display,
{
    coordinator
        .fail_connect_if_current(generation, attempt_id)
        .await;
    coordinator.finish_connect(generation, attempt_id).await;
    publish_disconnected_status_with_if_current(coordinator, generation, connected, broadcast).await
}

async fn publish_connected_status_if_current(
    coordinator: &crate::state::ConnectionCoordinator,
    generation: u64,
    app_handle: &tauri::AppHandle,
    connected: &tokio::sync::RwLock<bool>,
) -> bool {
    coordinator
        .publish_connected_if_current(generation, || async {
            apply_connected_status(app_handle, connected).await;
        })
        .await
}

#[cfg(test)]
/// 测试入口：推进连接代并在默认时限内断开旧客户端。
pub(crate) async fn cancel_connection_and_disconnect(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
) -> Result<ConnectionResetOutcome, String> {
    cancel_connection_and_disconnect_with_timeout(coordinator, client_slot, CHAT_DISCONNECT_TIMEOUT)
        .await
}

/// 推进 generation、清除认证并尝试断开原连接。
///
/// 返回的新 generation 即使断开超时也已经生效；断开结果保存在返回值中。
pub(crate) async fn cancel_auth_and_disconnect(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
) -> Result<ConnectionResetOutcome, String> {
    let (generation, owner) = coordinator
        .cancel_and_advance_clearing_auth(auth_session)
        .await?;
    let disconnect_result = match owner {
        Some(owner) => {
            disconnect_owned_chat_client_with_timeout(client_slot, owner, CHAT_DISCONNECT_TIMEOUT)
                .await
        }
        None => Ok(false),
    };
    Ok(ConnectionResetOutcome {
        generation,
        disconnect_result,
    })
}

/// 取消连接或认证后的代际推进与网络断开结果。
pub(crate) struct ConnectionResetOutcome {
    /// 推进后的当前 generation。
    pub generation: u64,
    /// 是否实际取得并断开客户端，或断开过程中产生的错误。
    pub disconnect_result: Result<bool, String>,
}

async fn cancel_connection_and_disconnect_with_timeout(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
    timeout: Duration,
) -> Result<ConnectionResetOutcome, String> {
    let (generation, owner) = coordinator.cancel_and_advance_with_owner().await?;
    let disconnect_result = match owner {
        Some(owner) => disconnect_owned_chat_client_with_timeout(client_slot, owner, timeout).await,
        None => Ok(false),
    };
    Ok(ConnectionResetOutcome {
        generation,
        disconnect_result,
    })
}

#[cfg(test)]
/// 测试入口：主动断开并把保留的认证会话重标到新 generation。
pub(crate) async fn disconnect_current_session(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
) -> Result<u64, String> {
    disconnect_current_session_with_timeout(
        coordinator,
        client_slot,
        auth_session,
        CHAT_DISCONNECT_TIMEOUT,
    )
    .await
}

#[cfg(test)]
async fn disconnect_current_session_with_timeout(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
    timeout: Duration,
) -> Result<u64, String> {
    let reset =
        cancel_connection_and_disconnect_with_timeout(coordinator, client_slot, timeout).await?;
    coordinator
        .retag_session_if_current(reset.generation, auth_session)
        .await;
    reset.disconnect_result?;
    Ok(reset.generation)
}

/// 取消当前连接、重标会话并发布 disconnected，最后返回网络断开结果。
///
/// 因状态发布早于检查 `disconnect_result`，即使断开超时，调用方也会先观察到
/// disconnected；这不承诺底层网络资源已经立即完成关闭。
async fn disconnect_current_session_and_publish_with_timeout<F, E>(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
    connected: &tokio::sync::RwLock<bool>,
    timeout: Duration,
    broadcast: F,
) -> Result<(), String>
where
    F: FnOnce(&'static str) -> Result<(), E>,
    E: Display,
{
    let reset =
        cancel_connection_and_disconnect_with_timeout(coordinator, client_slot, timeout).await?;
    coordinator
        .retag_session_if_current(reset.generation, auth_session)
        .await;
    publish_disconnected_status_with_if_current(
        coordinator,
        reset.generation,
        connected,
        broadcast,
    )
    .await;
    reset.disconnect_result?;
    Ok(())
}

/// 在总时限内取得槽位并断开指定所有者的客户端。
///
/// 槽位等待也计入超时；客户端在调用 `disconnect` 前已从槽位移除，因此超时返回
/// 错误时不会重新放回，但也不承诺底层异步断开已完成。
async fn disconnect_owned_chat_client_with_timeout(
    client_slot: &crate::state::ClientSlot,
    owner: crate::state::ConnectionAttemptKey,
    timeout: Duration,
) -> Result<bool, String> {
    let disconnect = async {
        let mut slot = client_slot.lock().await;
        let is_owned = slot
            .as_ref()
            .is_some_and(|installed| installed.key() == owner);
        if !is_owned {
            return false;
        }
        let mut installed = slot.take().expect("owned client checked above");
        drop(slot);
        installed.client.disconnect().await;
        true
    };
    match tokio::time::timeout(timeout, disconnect).await {
        Ok(disconnected) => Ok(disconnected),
        Err(_) => Err(format!("Chat disconnect timed out after {timeout:?}")),
    }
}

#[tauri::command]
/// 取消当前连接代并发布断开状态。
///
/// `state` 提供协调器、客户端槽位、认证会话和事件句柄。命令推进 generation，
/// 重标仍有效的认证会话，尝试在 1 秒内断开所拥有的客户端，并发送
/// `connection_status = "disconnected"`。成功返回 `Ok(())`；协调失败或断开超时
/// 返回字符串错误。状态发布发生在返回断开错误之前，广播失败仅记录日志；超时不
/// 表示底层资源已同步完成优雅断开。
pub async fn disconnect_chat(state: State<'_, AppState>) -> Result<(), String> {
    disconnect_current_session_and_publish_with_timeout(
        state.connection_coordinator.as_ref(),
        state.chat_client.as_ref(),
        state.auth_session.as_ref(),
        &state.connected,
        CHAT_DISCONNECT_TIMEOUT,
        |status| state.app_handle().emit("connection_status", status),
    )
    .await
}

#[tauri::command]
/// 分页查询指定群的已存储消息。
///
/// `state` 用于访问 SQLite；`group_id` 必须是可解析的 64 位十进制整数，`limit`
/// 范围为 1..=200，`offset` 最大为 1_000_000 且二者须可转换为 SQLite 整数。
/// 成功返回按存储层顺序映射的 [`MessageDto`]；参数非法或数据库查询失败时返回
/// 字符串错误。本命令不改变连接或数据库内容，也不发送前端事件。
pub async fn get_messages(
    state: State<'_, AppState>,
    group_id: String,
    limit: usize,
    offset: usize,
) -> Result<Vec<MessageDto>, String> {
    let group_id = super::parse_i64_id(&group_id, "group_id")?;
    let (limit, offset) = validate_message_page(limit, offset)?;
    let messages = state
        .db
        .messages
        .get_by_group(group_id, limit, offset)
        .await
        .map_err(|e| e.to_string())?;
    Ok(messages.into_iter().map(message_dto_from_row).collect())
}

fn validate_message_page(limit: usize, offset: usize) -> Result<(usize, usize), String> {
    if !(1..=im_store::message::MAX_MESSAGE_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "limit must be between 1 and {}",
            im_store::message::MAX_MESSAGE_PAGE_LIMIT
        ));
    }
    if offset > im_store::message::MAX_MESSAGE_PAGE_OFFSET {
        return Err(format!(
            "offset exceeds maximum {}",
            im_store::message::MAX_MESSAGE_PAGE_OFFSET
        ));
    }
    i64::try_from(limit).map_err(|_| "limit exceeds supported integer range".to_string())?;
    i64::try_from(offset).map_err(|_| "offset exceeds supported integer range".to_string())?;
    Ok((limit, offset))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    use im_common::config::AppConfig;
    use tokio::io::AsyncReadExt;

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use prost::Message;

    use crate::state::{AuthSession, ConnectionCoordinator, InstalledClient};

    use super::{
        begin_connection_attempt, cancel_connection_and_disconnect, disconnect_current_session,
        disconnect_current_session_and_publish_with_timeout,
        disconnect_current_session_with_timeout, disconnect_owned_chat_client_with_timeout,
        enqueue_incoming_frame, fail_initial_connection_and_publish, linked_cancellation,
        mark_connected_and_broadcast, mark_disconnected_and_broadcast, message_dto_from_row,
        retry_automatic_connection, run_cancellable_with_timeout, run_message_worker_with_effects,
        stored_message_parts, validate_message_page, ConnectionAttemptGuard, IncomingFrame,
        MessageEffects, HEARTBEAT_INTERVAL, MAX_QUEUED_MESSAGE_SIZE, MESSAGE_QUEUE_CAPACITY,
    };

    fn installed_client(client: im_chat::ChatClient) -> InstalledClient {
        InstalledClient::new(crate::state::ConnectionAttemptKey::new(0, 1), client)
    }

    // 分页边界：拒绝零值、超限与整数溢出，同时接受最大合法 limit/offset。
    #[test]
    fn message_page_rejects_zero_excessive_and_overflowing_values() {
        assert!(validate_message_page(0, 0).is_err());
        assert!(validate_message_page(201, 0).is_err());
        assert!(validate_message_page(1, 1_000_001).is_err());
        if usize::BITS > 63 {
            assert!(validate_message_page(1, (i64::MAX as usize) + 1).is_err());
        }
        assert_eq!(
            validate_message_page(200, 1_000_000).unwrap(),
            (200, 1_000_000)
        );
    }

    // 自动连接：瞬时失败后保留同代认证会话并重试，第二次成功即停止。
    #[tokio::test]
    async fn automatic_connection_retries_initial_failure_without_clearing_login() {
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 7,
        })));
        let shutdown = tokio_util::sync::CancellationToken::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = attempts.clone();

        retry_automatic_connection(
            7,
            auth_session.clone(),
            shutdown,
            move || {
                let attempt = observed_attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        Err("tcp unavailable".to_string())
                    } else {
                        Ok(())
                    }
                }
            },
            |_| async {},
        )
        .await;

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(auth_session.read().await.as_ref().unwrap().generation, 7);
    }

    // 初次连接失败：协调器回到 Idle，并只发布权威的 disconnected 终态。
    #[tokio::test]
    async fn initial_connection_failure_publishes_authoritative_disconnected_terminal_state() {
        let coordinator = ConnectionCoordinator::new();
        let permit = coordinator.begin_connect(0).await.unwrap();
        let connected = tokio::sync::RwLock::new(false);
        let statuses = std::sync::Mutex::new(Vec::new());

        assert!(
            fail_initial_connection_and_publish(
                &coordinator,
                permit.generation(),
                permit.attempt_id(),
                &connected,
                |status| {
                    statuses.lock().unwrap().push(status);
                    Ok::<(), String>(())
                },
            )
            .await
        );

        assert_eq!(*statuses.lock().unwrap(), ["disconnected"]);
        assert_eq!(
            coordinator.phase().await,
            crate::state::ConnectionPhase::Idle
        );
    }

    // 队列上限：已解码正文超过 8 MiB 时在入队前明确拒绝。
    #[tokio::test]
    async fn bounded_message_queue_rejects_oversize() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(MESSAGE_QUEUE_CAPACITY);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let oversized = enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 2202,
                content: vec![0; MAX_QUEUED_MESSAGE_SIZE + 1],
            },
            &cancellation,
        )
        .await
        .unwrap_err();
        assert!(oversized.to_string().contains("exceeds queue limit"));
    }

    // 2102 兼容性：回执字段及 wire 编码必须与 Java 服务端契约一致。
    #[test]
    fn group_delivery_receipt_matches_java_proto_contract() {
        let receipt = im_proto::ReceiveGroupMessage {
            msg_id: vec![70, 71],
            group_id: 7,
        };

        assert_eq!(receipt.encode_to_vec(), vec![10, 2, 70, 71, 16, 7]);
    }

    // 心跳配置：30 秒周期必须短于服务端 60 秒失活窗口。
    #[test]
    fn heartbeat_interval_is_below_java_server_timeout() {
        assert!(HEARTBEAT_INTERVAL < std::time::Duration::from_secs(60));
    }

    // 背压语义：队列满时发送者等待，释放容量后两帧仍保持顺序且不丢失。
    #[tokio::test]
    async fn bounded_message_queue_waits_for_capacity_without_message_loss() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let cancellation = tokio_util::sync::CancellationToken::new();
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 3203,
                content: Vec::new(),
            },
            &cancellation,
        )
        .await
        .unwrap();

        let pending_sender = sender.clone();
        let pending_cancellation = cancellation.clone();
        let pending = tokio::spawn(async move {
            enqueue_incoming_frame(
                &pending_sender,
                IncomingFrame {
                    message_id: 3204,
                    content: Vec::new(),
                },
                &pending_cancellation,
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());

        assert_eq!(receiver.recv().await.unwrap().message_id, 3203);
        pending.await.unwrap().unwrap();
        assert_eq!(receiver.recv().await.unwrap().message_id, 3204);
    }

    // RAII 清理：连接命令被中止时守卫取消 worker、推进代际并恢复空闲状态。
    #[tokio::test]
    async fn dropping_connect_attempt_guard_clears_connecting_and_cancels_workers() {
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let permit = coordinator.begin_connect(0).await.unwrap();
        let cancellation = permit.cancellation_token();
        let attempt_id = permit.attempt_id();
        let observed_cancellation = cancellation.clone();
        let worker_coordinator = coordinator.clone();
        let worker = tokio::spawn(async move {
            observed_cancellation.cancelled().await;
            worker_coordinator.finish_connect(0, attempt_id).await;
        });
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        })));
        let connected = Arc::new(tokio::sync::RwLock::new(false));
        let guard = ConnectionAttemptGuard::new(
            0,
            attempt_id,
            coordinator.clone(),
            slot.clone(),
            auth_session.clone(),
            connected,
            None,
        );
        let command = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        command.abort();
        let _ = command.await;

        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            cancellation.cancelled(),
        )
        .await
        .expect("dropped command must cancel generation workers");
        worker.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(200), async {
            loop {
                if coordinator.phase().await == crate::state::ConnectionPhase::Idle
                    && slot.lock().await.is_none()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropped command must restore idle state and clear client slot");
        assert_eq!(auth_session.read().await.as_ref().unwrap().generation, 1);
    }

    // attempt 门禁：旧守卫延迟析构不能取消同一 generation 内的新尝试。
    #[tokio::test]
    async fn stale_attempt_guard_drop_cannot_cancel_new_attempt_in_same_generation() {
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let old = coordinator.begin_connect(0).await.unwrap();
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        })));
        let connected = Arc::new(tokio::sync::RwLock::new(false));
        let (guard, cleanup_finished) = ConnectionAttemptGuard::new_for_test(
            old.generation(),
            old.attempt_id(),
            coordinator.clone(),
            slot,
            auth_session,
            connected,
        );
        coordinator
            .fail_connect_if_current(old.generation(), old.attempt_id())
            .await;
        let current = coordinator.begin_connect(0).await.unwrap();

        drop(guard);
        cleanup_finished.await.unwrap();

        assert_eq!(
            coordinator.phase().await,
            crate::state::ConnectionPhase::Connecting
        );
        assert!(!current.cancellation_token().is_cancelled());
        coordinator
            .finish_connect(current.generation(), current.attempt_id())
            .await;
    }

    // generation 门禁：旧清理等待期间安装的新代客户端不能被旧守卫取走。
    #[tokio::test]
    async fn old_guard_waiting_for_finish_cannot_take_new_generation_client() {
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let old = coordinator.begin_connect(0).await.unwrap();
        let old_cancellation = old.cancellation_token();
        let observed_cancellation = old_cancellation.clone();
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        let auth_session = Arc::new(tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "old-token".to_string(),
            generation: 0,
        })));
        let connected = Arc::new(tokio::sync::RwLock::new(false));
        let (guard, cleanup_finished) = ConnectionAttemptGuard::new_for_test(
            old.generation(),
            old.attempt_id(),
            coordinator.clone(),
            slot.clone(),
            auth_session.clone(),
            connected,
        );
        let (release_finish, finish_released) = tokio::sync::oneshot::channel();
        let worker_coordinator = coordinator.clone();
        let old_generation = old.generation();
        let old_attempt_id = old.attempt_id();
        let old_worker = tokio::spawn(async move {
            old_cancellation.cancelled().await;
            finish_released.await.unwrap();
            worker_coordinator
                .finish_connect(old_generation, old_attempt_id)
                .await;
        });

        drop(guard);
        observed_cancellation.cancelled().await;
        *auth_session.write().await = Some(AuthSession {
            uid: 84,
            token: "new-token".to_string(),
            generation: 1,
        });
        let current = coordinator.begin_connect(1).await.unwrap();
        let current_session = auth_session.read().await.clone().unwrap();
        coordinator
            .install_if_current(
                &current,
                &current_session,
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();

        release_finish.send(()).unwrap();
        old_worker.await.unwrap();
        cleanup_finished.await.unwrap();

        let installed = slot.lock().await;
        let installed = installed
            .as_ref()
            .expect("new client must remain installed");
        assert_eq!(
            installed.key(),
            crate::state::ConnectionAttemptKey::new(1, current.attempt_id())
        );
        assert!(matches!(installed, InstalledClient { .. }));
    }

    #[derive(Default)]
    struct FakeMessageEffects {
        monitored: HashSet<i64>,
        persisted: tokio::sync::Mutex<Vec<i64>>,
        acknowledged: tokio::sync::Mutex<Vec<(i64, Vec<i64>)>>,
    }

    #[async_trait::async_trait]
    impl MessageEffects for FakeMessageEffects {
        async fn is_monitored(&self, group_id: i64) -> bool {
            self.monitored.contains(&group_id)
        }

        async fn persist_and_emit(&self, message: im_proto::GroupMessage) -> bool {
            self.persisted.lock().await.push(message.msg_id);
            true
        }

        async fn acknowledge_group_messages(
            &self,
            group_id: i64,
            msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            self.acknowledged.lock().await.push((group_id, msg_ids));
            Ok(())
        }
    }

    struct SharedMonitoringEffects {
        monitored: Arc<tokio::sync::RwLock<HashSet<i64>>>,
        persisted: tokio::sync::Mutex<Vec<i64>>,
        acknowledged: tokio::sync::Mutex<Vec<(i64, Vec<i64>)>>,
    }

    #[async_trait::async_trait]
    impl MessageEffects for SharedMonitoringEffects {
        async fn is_monitored(&self, group_id: i64) -> bool {
            self.monitored.read().await.contains(&group_id)
        }

        async fn persist_and_emit(&self, message: im_proto::GroupMessage) -> bool {
            self.persisted.lock().await.push(message.msg_id);
            true
        }

        async fn acknowledge_group_messages(
            &self,
            group_id: i64,
            msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            self.acknowledged.lock().await.push((group_id, msg_ids));
            Ok(())
        }
    }

    // 监控快照：远端刷新移除群后，worker 不再持久化该群消息但仍发送回执。
    #[tokio::test]
    async fn snapshot_refresh_removes_unavailable_group_before_worker_checks_monitoring() {
        let store = im_store::SqliteStore::new(":memory:").await.unwrap();
        let group_ops = tokio::sync::Mutex::new(());
        let monitoring = Arc::new(tokio::sync::RwLock::new([7].into_iter().collect()));
        let group = |group_id| im_store::group::GroupRow {
            group_id,
            name: format!("Group {group_id}"),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 1,
        };
        store
            .groups
            .sync_remote_groups(&[group(7), group(8)])
            .await
            .unwrap();
        crate::commands::groups::sync_remote_groups_and_refresh_monitoring(
            &group_ops,
            &store,
            &monitoring,
            &[group(8)],
        )
        .await
        .unwrap();
        let effects = Arc::new(SharedMonitoringEffects {
            monitored: monitoring,
            persisted: tokio::sync::Mutex::new(Vec::new()),
            acknowledged: tokio::sync::Mutex::new(Vec::new()),
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let push = im_proto::PushGroupMessage {
            group_msg: vec![im_proto::GroupMessage {
                msg_id: 70,
                group_id: 7,
                ..Default::default()
            }],
            ..Default::default()
        };
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: push.encode_to_vec(),
            })
            .await
            .unwrap();
        drop(sender);

        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        assert!(effects.persisted.lock().await.is_empty());
        assert_eq!(*effects.acknowledged.lock().await, [(7, vec![70])]);
    }

    // 协议主路径：有效 1201 完成登录；2202 仅持久化监控群，并为两群全量回执。
    #[tokio::test]
    async fn message_worker_accepts_valid_1201_and_only_processes_monitored_2202() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            persisted: tokio::sync::Mutex::new(Vec::new()),
            acknowledged: tokio::sync::Mutex::new(Vec::new()),
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let (login_sender, login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation,
            login_sender,
        ));
        let login = im_proto::PushLoginSuccessMessage {
            login_time: 1_725_000_000_000,
            user_key_pair: Some(im_proto::KeyPairBase {
                public_key: "user-public".to_string(),
                private_key: "user-private".to_string(),
                key_version: 7,
                ..Default::default()
            }),
            web_key_pair: Some(im_proto::KeyPairBase {
                public_key: "web-public".to_string(),
                key_version: 8,
                ..Default::default()
            }),
            bf_web_online: true,
        };
        sender
            .send(IncomingFrame {
                message_id: 1201,
                content: login.encode_to_vec(),
            })
            .await
            .unwrap();
        let push = im_proto::PushGroupMessage {
            group_msg: vec![
                im_proto::GroupMessage {
                    msg_id: 70,
                    group_id: 7,
                    ..Default::default()
                },
                im_proto::GroupMessage {
                    msg_id: 80,
                    group_id: 8,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: push.encode_to_vec(),
            })
            .await
            .unwrap();
        sender
            .send(IncomingFrame {
                message_id: 2205,
                content: b"reserved recall payload".to_vec(),
            })
            .await
            .unwrap();
        drop(sender);

        login_receiver.await.unwrap();
        worker.await.unwrap();
        assert_eq!(*effects.persisted.lock().await, [70]);
        assert_eq!(
            *effects.acknowledged.lock().await,
            [(7, vec![70]), (8, vec![80])]
        );
    }

    // 1201 合法最小值：零 login_time 的默认消息仍是有效登录确认。
    #[tokio::test]
    async fn message_worker_accepts_zero_login_time_1201() {
        let effects = Arc::new(FakeMessageEffects::default());
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let observed_cancellation = cancellation.clone();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects,
            cancellation,
            login_sender,
        ));

        sender
            .send(IncomingFrame {
                message_id: 1201,
                content: im_proto::PushLoginSuccessMessage::default().encode_to_vec(),
            })
            .await
            .unwrap();
        drop(sender);

        login_receiver.await.unwrap();
        worker.await.unwrap();
        assert!(!observed_cancellation.is_cancelled());
    }

    // 1201 失败关闭：畸形字节及错误消息类型均不触发 oneshot，并取消连接。
    #[tokio::test]
    async fn message_worker_rejects_malformed_or_login_session_payload_1201() {
        for payload in [
            vec![0xFF, 0xFF],
            im_proto::LoginSessionMessage {
                clinet_info: Some(im_proto::ClientInfo {
                    token: "session-token".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }
            .encode_to_vec(),
        ] {
            let effects = Arc::new(FakeMessageEffects::default());
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            let (login_sender, login_receiver) = tokio::sync::oneshot::channel();
            let cancellation = tokio_util::sync::CancellationToken::new();
            let observed_cancellation = cancellation.clone();
            let worker = tokio::spawn(run_message_worker_with_effects(
                receiver,
                effects,
                cancellation,
                login_sender,
            ));
            sender
                .send(IncomingFrame {
                    message_id: 1201,
                    content: payload,
                })
                .await
                .unwrap();
            drop(sender);

            assert!(login_receiver.await.is_err());
            worker.await.unwrap();
            assert!(observed_cancellation.is_cancelled());
        }
    }

    // Wire 类型防线：LoginSession 字段一无法误解码成 PushLoginSuccess。
    #[test]
    fn login_session_field_one_has_incompatible_wire_type_for_push_login_success() {
        let payload = im_proto::LoginSessionMessage {
            clinet_info: Some(im_proto::ClientInfo {
                token: "session-token".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
        .encode_to_vec();

        assert!(im_proto::PushLoginSuccessMessage::decode(payload.as_slice()).is_err());
    }

    // fail-closed：取消先于收帧生效时，排队的 2202 不产生持久化副作用。
    #[tokio::test]
    async fn cancelled_message_worker_discards_queued_frames() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            persisted: tokio::sync::Mutex::new(Vec::new()),
            acknowledged: tokio::sync::Mutex::new(Vec::new()),
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let push = im_proto::PushGroupMessage {
            group_msg: vec![im_proto::GroupMessage {
                msg_id: 70,
                group_id: 7,
                ..Default::default()
            }],
            ..Default::default()
        };
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: push.encode_to_vec(),
            })
            .await
            .unwrap();
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();

        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        assert!(effects.persisted.lock().await.is_empty());
    }

    // shutdown 联动：应用关闭令 generation 链接令牌及时取消后台任务。
    #[tokio::test]
    async fn app_shutdown_cancels_generation_linked_tasks() {
        let generation = tokio_util::sync::CancellationToken::new();
        let shutdown = tokio_util::sync::CancellationToken::new();
        let linked = linked_cancellation(generation, shutdown.clone());

        shutdown.cancel();

        tokio::time::timeout(std::time::Duration::from_millis(100), linked.cancelled())
            .await
            .expect("shutdown must cancel connection tasks promptly");
    }

    // 状态广播降级：listener 失败不阻止兼容布尔值切换为 disconnected。
    #[tokio::test]
    async fn disconnected_status_ignores_broadcast_failure() {
        let connected = tokio::sync::RwLock::new(true);
        let observed = std::sync::Mutex::new(None);

        mark_disconnected_and_broadcast(&connected, |status| {
            *observed.lock().unwrap() = Some(status.to_string());
            Err::<(), _>("listener unavailable".to_string())
        })
        .await;

        assert!(!*connected.read().await);
        assert_eq!(observed.lock().unwrap().as_deref(), Some("disconnected"));
    }

    // 状态同步：connected 同时更新兼容布尔值并向观察者广播。
    #[tokio::test]
    async fn connected_status_sets_state_and_broadcasts() {
        let connected = tokio::sync::RwLock::new(false);
        let observed = std::sync::Mutex::new(None);

        mark_connected_and_broadcast(&connected, |status| {
            *observed.lock().unwrap() = Some(status.to_string());
            Ok::<(), &str>(())
        })
        .await;

        assert!(*connected.read().await);
        assert_eq!(observed.lock().unwrap().as_deref(), Some("connected"));
    }

    // 重复连接：已安装客户端与进行中尝试分别返回稳定的前端错误码。
    #[tokio::test]
    async fn duplicate_connect_is_rejected_before_starting_operation() {
        let coordinator = ConnectionCoordinator::new();
        let installed = tokio::sync::Mutex::new(Some(installed_client(im_chat::ChatClient::new(
            AppConfig::default(),
        ))));

        let installed_error = begin_connection_attempt(&coordinator, &installed, 0)
            .await
            .err()
            .unwrap();
        assert_eq!(installed_error, "AlreadyConnected");

        let empty = tokio::sync::Mutex::new(None);
        let operation = coordinator.begin_connect(0).await.unwrap();
        let connecting_error = begin_connection_attempt(&coordinator, &empty, 0)
            .await
            .err()
            .unwrap();
        assert_eq!(connecting_error, "Connecting");
        coordinator
            .finish_connect(operation.generation(), operation.attempt_id())
            .await;
    }

    // 超时诊断：阻塞 connect 到期后返回包含操作名和时长的错误。
    #[tokio::test]
    async fn connection_timeout_returns_clear_error() {
        let cancellation = tokio_util::sync::CancellationToken::new();

        let error = run_cancellable_with_timeout(
            "Chat connect",
            std::time::Duration::from_millis(10),
            &cancellation,
            std::future::pending::<Result<(), std::io::Error>>(),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Chat connect timed out after 10ms");
    }

    // 取消优先级：令牌已取消时不等待较长 login 超时。
    #[tokio::test]
    async fn connection_wait_prefers_explicit_cancellation() {
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();

        let error = run_cancellable_with_timeout(
            "Chat login",
            std::time::Duration::from_secs(15),
            &cancellation,
            std::future::pending::<Result<(), std::io::Error>>(),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Chat login cancelled");
    }

    // 断开超时：客户端先移出槽位并推进 generation，随后报告超时且释放套接字。
    #[tokio::test]
    async fn disconnect_timeout_returns_error_and_force_aborts_resources() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut config = AppConfig::default();
        config.server.im_chat_host = address.ip().to_string();
        config.server.im_chat_port = address.port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0u8; 1];
            socket.read(&mut byte).await.unwrap()
        });
        let mut client = im_chat::ChatClient::new(config);
        client.on_disconnect(std::future::pending);
        client.connect().await.unwrap();
        let slot = tokio::sync::Mutex::new(None);
        let coordinator = ConnectionCoordinator::new();
        let auth_session = tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        }));
        let permit = coordinator.begin_connect(0).await.unwrap();
        coordinator
            .install_if_current(
                &permit,
                auth_session.read().await.as_ref().unwrap(),
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                client,
            )
            .await
            .unwrap();
        coordinator
            .finish_connect(permit.generation(), permit.attempt_id())
            .await;

        let error = disconnect_current_session_with_timeout(
            &coordinator,
            &slot,
            &auth_session,
            std::time::Duration::from_millis(10),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Chat disconnect timed out after 10ms");
        assert!(slot.lock().await.is_none());
        assert_eq!(auth_session.read().await.as_ref().unwrap().generation, 1);
        let next = coordinator.begin_connect(1).await.unwrap();
        coordinator
            .finish_connect(next.generation(), next.attempt_id())
            .await;
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), server)
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }

    // 超时范围：等待客户端槽位锁也计入 disconnect 的总时限。
    #[tokio::test]
    async fn disconnect_timeout_includes_waiting_for_client_slot() {
        let slot = Arc::new(tokio::sync::Mutex::new(Some(installed_client(
            im_chat::ChatClient::new(AppConfig::default()),
        ))));
        let guard = slot.lock().await;
        let task_slot = slot.clone();
        let task = tokio::spawn(async move {
            disconnect_owned_chat_client_with_timeout(
                &task_slot,
                crate::state::ConnectionAttemptKey::new(0, 1),
                std::time::Duration::from_millis(10),
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            task.is_finished(),
            "slot acquisition must be inside the disconnect timeout"
        );
        drop(guard);
        assert_eq!(
            task.await.unwrap().unwrap_err(),
            "Chat disconnect timed out after 10ms"
        );
    }

    // 终态顺序：即使网络断开超时，也先发布 disconnected 再向调用方返回错误。
    #[tokio::test]
    async fn disconnect_timeout_publishes_disconnected_before_returning_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut config = AppConfig::default();
        config.server.im_chat_host = address.ip().to_string();
        config.server.im_chat_port = address.port();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let mut client = im_chat::ChatClient::new(config);
        client.on_disconnect(std::future::pending);
        client.connect().await.unwrap();
        let slot = tokio::sync::Mutex::new(None);
        let coordinator = ConnectionCoordinator::new();
        let auth_session = tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        }));
        let permit = coordinator.begin_connect(0).await.unwrap();
        coordinator
            .install_if_current(
                &permit,
                auth_session.read().await.as_ref().unwrap(),
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                client,
            )
            .await
            .unwrap();
        coordinator
            .finish_connect(permit.generation(), permit.attempt_id())
            .await;
        let connected = tokio::sync::RwLock::new(true);
        let statuses = std::sync::Mutex::new(Vec::new());

        let error = disconnect_current_session_and_publish_with_timeout(
            &coordinator,
            &slot,
            &auth_session,
            &connected,
            std::time::Duration::from_millis(10),
            |status| {
                statuses.lock().unwrap().push(status);
                Ok::<(), &str>(())
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Chat disconnect timed out after 10ms");
        assert!(!*connected.read().await);
        assert_eq!(*statuses.lock().unwrap(), ["disconnected"]);
        server.abort();
    }

    // 主动取消：被阻塞的连接尝试及时收到取消，且无客户端时返回未执行断开。
    #[tokio::test]
    async fn blocked_connection_is_cancelled_promptly() {
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        let permit = coordinator.begin_connect(0).await.unwrap();
        let cancellation = permit.cancellation_token();
        let generation = permit.generation();
        let attempt_id = permit.attempt_id();
        let worker_coordinator = coordinator.clone();
        let worker = tokio::spawn(async move {
            cancellation.cancelled().await;
            worker_coordinator
                .finish_connect(generation, attempt_id)
                .await;
        });

        let reset = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            cancel_connection_and_disconnect(&coordinator, &slot),
        )
        .await
        .expect("disconnect must cancel blocked connect promptly")
        .unwrap();

        assert_eq!(reset.generation, 1);
        assert!(!reset.disconnect_result.unwrap());
        worker.await.unwrap();
    }

    // 陈旧结果：generation 推进后，旧连接完成也不能安装进全局槽位。
    #[tokio::test]
    async fn stale_connection_result_cannot_install_after_generation_changes() {
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        let expected_session = AuthSession {
            uid: 42,
            token: "old-token".to_string(),
            generation: 0,
        };
        let auth_session = tokio::sync::RwLock::new(Some(expected_session.clone()));
        let permit = coordinator.begin_connect(0).await.unwrap();
        let installed = AtomicBool::new(false);
        let cancellation = permit.cancellation_token();
        let generation = permit.generation();
        let attempt_id = permit.attempt_id();
        let worker_coordinator = coordinator.clone();
        let worker = tokio::spawn(async move {
            cancellation.cancelled().await;
            worker_coordinator
                .finish_connect(generation, attempt_id)
                .await;
        });
        coordinator.cancel_and_advance().await.unwrap();
        worker.await.unwrap();

        let error = coordinator
            .install_if_current(
                &permit,
                &expected_session,
                &auth_session,
                &slot,
                &installed,
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap_err()
            .0;

        assert!(error.contains("stale"));
        assert!(slot.lock().await.is_none());
    }

    // 会话延续：主动断开重标认证 generation，使后续手动重连仍可申请许可。
    #[tokio::test]
    async fn disconnect_retags_session_for_a_later_reconnect() {
        let coordinator = ConnectionCoordinator::new();
        let slot = tokio::sync::Mutex::new(None);
        let auth_session = tokio::sync::RwLock::new(Some(AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        }));

        disconnect_current_session(&coordinator, &slot, &auth_session)
            .await
            .unwrap();

        assert_eq!(auth_session.read().await.as_ref().unwrap().generation, 1);
        let permit = coordinator.begin_connect(1).await.unwrap();
        coordinator
            .finish_connect(permit.generation(), permit.attempt_id())
            .await;
    }

    // 优雅断开：helper 取走所属客户端并等待 disconnect callback 完成。
    #[tokio::test]
    async fn disconnect_helper_takes_client_and_waits_for_shutdown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut config = AppConfig::default();
        config.server.im_chat_host = address.ip().to_string();
        config.server.im_chat_port = address.port();
        let disconnected = Arc::new(AtomicBool::new(false));
        let observed = disconnected.clone();
        let mut client = im_chat::ChatClient::new(config);
        client.on_disconnect(move || {
            let observed = observed.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                observed.store(true, Ordering::SeqCst);
            }
        });
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accepted_tx.send(()).unwrap();
            let mut byte = [0u8; 1];
            socket.read(&mut byte).await.unwrap()
        });

        client.connect().await.unwrap();
        accepted_rx.await.unwrap();
        let slot = tokio::sync::Mutex::new(Some(installed_client(client)));

        assert!(disconnect_owned_chat_client_with_timeout(
            &slot,
            crate::state::ConnectionAttemptKey::new(0, 1),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap());

        assert!(slot.lock().await.is_none());
        assert!(disconnected.load(Ordering::SeqCst));
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), server)
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }

    // Base64 契约：实时和历史 DTO 对二进制正文编码一致，数据库保留原始 protobuf。
    #[test]
    fn realtime_and_stored_message_dtos_share_base64_content_contract() {
        let message = im_proto::GroupMessage {
            msg_id: 700,
            group_id: 9,
            send_uid: 8,
            msg_type: 3,
            content: vec![0, 1, 254, 255],
            send_time: 1234,
            content_md5: "digest".to_string(),
            ..Default::default()
        };

        let (record, realtime) = stored_message_parts(&message);
        let stored = message_dto_from_row(im_store::message::MessageRow {
            msg_id: record.msg_id,
            group_id: record.group_id,
            send_uid: record.send_uid,
            msg_type: record.msg_type,
            content: record.content.clone(),
            send_time: record.send_time,
            content_md5: record.content_md5.clone(),
            stored_at: 5678,
            raw_proto: record.raw_proto.clone(),
        });

        assert_eq!(realtime.content_b64, STANDARD.encode(&message.content));
        assert_eq!(stored.content_b64, realtime.content_b64);
        assert_eq!(record.raw_proto, Some(message.encode_to_vec()));
        assert!(serde_json::to_value(stored)
            .unwrap()
            .get("content")
            .is_none());
    }

    // 大整数契约：全部 i64 标识序列化为十进制字符串，避免前端精度损失。
    #[test]
    fn message_dto_serializes_all_i64_identifiers_as_decimal_strings() {
        let message = im_proto::GroupMessage {
            msg_id: i64::MAX,
            group_id: i64::MAX - 1,
            send_uid: i64::MAX - 2,
            ..Default::default()
        };

        let (_, dto) = stored_message_parts(&message);
        let json = serde_json::to_value(dto).unwrap();

        assert_eq!(json["msg_id"], i64::MAX.to_string());
        assert_eq!(json["group_id"], (i64::MAX - 1).to_string());
        assert_eq!(json["send_uid"], (i64::MAX - 2).to_string());
    }
}
