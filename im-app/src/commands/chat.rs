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

const CHAT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CHAT_LOGIN_TIMEOUT: Duration = Duration::from_secs(15);
const CHAT_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const CHAT_SEND_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MESSAGE_QUEUE_CAPACITY: usize = 8;
const MAX_QUEUED_MESSAGE_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageDto {
    pub msg_id: String,
    pub group_id: String,
    pub send_uid: String,
    pub msg_type: i32,
    pub content_b64: String,
    pub send_time: i64,
    pub content_md5: String,
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

struct IncomingFrame {
    message_id: u16,
    content: Vec<u8>,
}

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
trait MessageEffects: Send + Sync {
    async fn is_monitored(&self, group_id: i64) -> bool;
    async fn persist_and_emit(&self, message: im_proto::GroupMessage) -> bool;
    async fn acknowledge_group_messages(
        &self,
        group_id: i64,
        msg_ids: Vec<i64>,
    ) -> Result<(), im_common::error::AppError>;
}

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

struct EstablishedConnection {
    client: im_chat::ChatClient,
    installed: Arc<AtomicBool>,
    connection_lost: Arc<AtomicBool>,
    connection_cancellation: CancellationToken,
    _message_worker: tokio::task::JoinHandle<()>,
}

#[tauri::command]
pub async fn connect_chat(state: State<'_, AppState>) -> Result<(), String> {
    connect_chat_inner(&state).await
}

#[tauri::command]
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

pub(crate) async fn authenticated_session_for_connect(
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
) -> Result<crate::state::AuthSession, String> {
    auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())
}

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

async fn run_message_worker(
    receiver: mpsc::Receiver<IncomingFrame>,
    effects: Arc<dyn MessageEffects>,
    cancellation: CancellationToken,
    login_sender: tokio::sync::oneshot::Sender<()>,
) {
    run_message_worker_with_effects(receiver, effects, cancellation, login_sender).await;
}

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
    // Cancellation is fail-closed: dropping the receiver discards queued frames,
    // so no SQLite writes or UI events can occur for a stale connection.
    receiver.close();
}

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

async fn chat_sender_if_owned(
    client_slot: &crate::state::ClientSlot,
    owner: crate::state::ConnectionAttemptKey,
) -> Option<im_chat::ChatSender> {
    let slot = client_slot.lock().await;
    slot.as_ref()
        .filter(|installed| installed.key() == owner)
        .and_then(|installed| installed.client.sender())
}

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

async fn disconnect_local_client(client: &mut im_chat::ChatClient) {
    if tokio::time::timeout(CHAT_DISCONNECT_TIMEOUT, client.disconnect())
        .await
        .is_err()
    {
        client.force_abort();
    }
}

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

pub(crate) async fn mark_disconnected_and_broadcast<F, E>(
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) where
    E: Display,
    F: FnOnce(&'static str) -> Result<(), E>,
{
    mark_connection_status_and_broadcast(connected, false, "disconnected", broadcast).await;
}

pub(crate) async fn mark_connected_and_broadcast<F, E>(
    connected: &tokio::sync::RwLock<bool>,
    broadcast: F,
) where
    E: Display,
    F: FnOnce(&'static str) -> Result<(), E>,
{
    mark_connection_status_and_broadcast(connected, true, "connected", broadcast).await;
}

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
pub(crate) async fn cancel_connection_and_disconnect(
    coordinator: &crate::state::ConnectionCoordinator,
    client_slot: &crate::state::ClientSlot,
) -> Result<ConnectionResetOutcome, String> {
    cancel_connection_and_disconnect_with_timeout(coordinator, client_slot, CHAT_DISCONNECT_TIMEOUT)
        .await
}

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

pub(crate) struct ConnectionResetOutcome {
    pub generation: u64,
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

    #[test]
    fn group_delivery_receipt_matches_java_proto_contract() {
        let receipt = im_proto::ReceiveGroupMessage {
            msg_id: vec![70, 71],
            group_id: 7,
        };

        assert_eq!(receipt.encode_to_vec(), vec![10, 2, 70, 71, 16, 7]);
    }

    #[test]
    fn heartbeat_interval_is_below_java_server_timeout() {
        assert!(HEARTBEAT_INTERVAL < std::time::Duration::from_secs(60));
    }

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
