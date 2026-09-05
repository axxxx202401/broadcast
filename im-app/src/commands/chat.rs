//! 聊天连接与消息查询命令。
//!
//! 本模块把 Tauri 命令、认证会话、TCP 聊天客户端、连接状态机和 SQLite
//! 消息存储串联起来。连接流程按“取得认证会话 → 申请带 generation/attempt
//! 标识的连接许可 → TCP connect → login → 等待 1201 登录成功推送 → 安装客户端
//! → 发布 connected → 启动心跳”推进；任一阶段失败都会按当前门禁清理资源并发布
//! 可确认的终态。断线重连同样受 generation、attempt 和认证会话约束，旧连接不能
//! 覆盖新会话。
//!
//! 入站回调把帧送入有界队列；64 个帧槽约束 mpsc，32 MiB 正文字节许可继续覆盖
//! pending、投影排队及活动投影。工作协程立即处理 1201，保留 2205 的预留行为；2202
//! 在 Prost 完整解码前先执行常量额外内存的顶层 wire 预扫描，再按 25ms 或 100 条微批。
//! 每个 2202 帧只读取一次监控集合；批内监控消息经单次事务落库成功后才与未监控消息
//! 一起按群发送 2102。
//! 回执完成后，监控批次进入容量为 8 的独立投影队列；单一投影 worker 串行取批，
//! 每批以最多 8 路并发解密、恢复原顺序并通过一次前端 Channel 发送。数据库整批失败
//! 时监控消息不回执，未监控消息仍回执；Channel 缺失或发送失败只记录警告。连接取消
//! 会丢弃尚未投影的已提交批次，数据库历史仍保留。1201 解码失败会触发故障关闭并取消
//! 连接，2202 解码失败仅丢弃当前帧。

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
use futures::StreamExt;
use prost::Message;
use tauri::{Emitter, Manager, State};
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
/// 心跳间隔为 45 秒，短于服务端 60 秒超时窗口。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(45);
/// 入站帧队列最多容纳 64 项，满载时回调等待以形成背压。
const MESSAGE_QUEUE_CAPACITY: usize = 64;
/// 所有尚未完成处理的入站帧正文合计最多占用 32 MiB。
const MESSAGE_QUEUE_BYTE_BUDGET: usize = 32 * 1024 * 1024;
/// 单个已解码入站帧最大为 8 MiB，超限帧令回调返回错误。
const MAX_QUEUED_MESSAGE_SIZE: usize = 8 * 1024 * 1024;
/// 2202 微批从首条消息进入空批次起最多等待 25 毫秒。
const MESSAGE_BATCH_MAX_DELAY: Duration = Duration::from_millis(25);
/// 单个 2202 微批最多容纳 100 条群消息。
const MESSAGE_BATCH_MAX_MESSAGES: usize = 100;
/// 单个 2202 帧在 Prost 解码前允许的 `group_msg` 顶层字段数量上限。
///
/// 这是防止小字节输入膨胀为大量堆对象的结构预算，不代表业务层消息数量契约。
const MAX_GROUP_MESSAGES_PER_PUSH: usize = 10_000;
/// 单批监控消息正文解密最多同时执行 8 项。
const MESSAGE_DECRYPT_CONCURRENCY: usize = 8;
/// 主消息 worker 与投影 worker 之间最多排队 8 个已提交并已回执的消息批次。
const MESSAGE_PROJECTION_QUEUE_CAPACITY: usize = 8;

/// 暴露给前端的群消息。
///
/// 64 位标识以十进制字符串表示，避免 JavaScript 数值精度损失；二进制正文统一使用
/// 标准 Base64。2202 实时消息在批量写入 SQLite 并完成 2102 回执后才进入
/// Channel 批次；实时 DTO 的 `stored_at` 为 `None`，是因为发送前没有回读或携带
/// INSERT 时生成的写入时间，不表示消息尚未落库。历史查询 DTO 会携带已存的写入时间。
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
    /// 群组显示名称；实时协议未携带名称时可能为空。
    pub group_name: String,
    /// 标准 Base64 编码的原始消息正文。
    pub content_b64: String,
    /// 成功解密并按消息类型解析后的结构化正文。
    pub decoded_content: Option<crate::message_content::MediaContent>,
    /// 解密或正文解析失败的可展示原因；不影响原始消息入库。
    pub decode_error: Option<String>,
    /// 服务端记录的发送时间。
    pub send_time: i64,
    /// 消息正文的 MD5 摘要。
    pub content_md5: String,
    /// 数据库写入时间；实时推送未回读 INSERT 生成的值，故为 `None`，但消息已成功落库。
    /// 历史查询会返回已存的写入时间。
    pub stored_at: Option<i64>,
    /// 是否匹配当前账号的开奖规则；`1` 为匹配，`0` 为不匹配。
    pub matched: i32,
}

/// 前端可安全回传的消息分页游标。
///
/// `msg_id` 使用十进制字符串，避免超过 JavaScript 安全整数范围时发生精度损失。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageCursorDto {
    /// 本页最老消息的发送时间。
    pub send_time: i64,
    /// 本页最老消息 ID 的十进制字符串。
    pub msg_id: String,
}

/// 历史消息分页命令返回值。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePageDto {
    /// 当前页消息；存储层降序读取，前端索引按时间升序展示。
    pub messages: Vec<MessageDto>,
    /// 仍有更早消息时，指向当前页最老一条消息的游标。
    pub next_cursor: Option<MessageCursorDto>,
    /// 是否仍可请求更早一页。
    pub has_more: bool,
}

/// 当前前端页面登记的实时消息 Channel；页面重载后用新 Channel 原子替换旧值。
pub type MessageChannelSlot = tokio::sync::RwLock<Option<tauri::ipc::Channel<Vec<MessageDto>>>>;

/// 替换实时消息接收端；不向旧页面发送关闭通知。
async fn replace_message_channel(
    slot: &MessageChannelSlot,
    channel: tauri::ipc::Channel<Vec<MessageDto>>,
) {
    *slot.write().await = Some(channel);
}

/// 向当前登记的页面发送一个保持协议原顺序的已入库消息批次。
///
/// 先克隆 Channel 再释放读锁，避免 WebView 执行期间阻塞页面重载后的接收端替换。
async fn publish_realtime_message(
    slot: &MessageChannelSlot,
    messages: &[MessageDto],
) -> Result<(), String> {
    let channel = slot
        .read()
        .await
        .clone()
        .ok_or_else(|| "Realtime message channel is not registered".to_string())?;
    channel
        .send(messages.to_vec())
        .map_err(|error| error.to_string())
}

fn stored_message_parts(
    message: &im_proto::GroupMessage,
) -> (im_store::message::MessageRecord, MessageDto) {
    let dto = MessageDto {
        msg_id: message.msg_id.to_string(),
        group_id: message.group_id.to_string(),
        send_uid: message.send_uid.to_string(),
        msg_type: message.msg_type,
        group_name: message.group_name.clone(),
        content_b64: STANDARD.encode(&message.content),
        decoded_content: None,
        decode_error: None,
        send_time: message.send_time,
        content_md5: message.content_md5.clone(),
        stored_at: None,
        matched: 0,
    };
    // 提取明文文本：version == 0 时内容未加密，直接转为 UTF-8；否则暂时留空，
    // 由调用方在持有解密密钥后补充（persist_monitored_batch 会在入库后立即更新）。
    let content_text = if message.version == 0 {
        String::from_utf8_lossy(&message.content).to_string()
    } else {
        String::new()
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
        content_text,
    };
    (record, dto)
}

fn message_dto_from_row(row: im_store::message::MessageRow) -> MessageDto {
    MessageDto {
        msg_id: row.msg_id.to_string(),
        group_id: row.group_id.to_string(),
        send_uid: row.send_uid.to_string(),
        msg_type: row.msg_type,
        group_name: row.group_name,
        content_b64: STANDARD.encode(row.content),
        decoded_content: None,
        decode_error: None,
        send_time: row.send_time,
        content_md5: row.content_md5,
        stored_at: Some(row.stored_at),
        matched: row.matched,
    }
}

fn message_client_info(
    config: &im_common::config::AppConfig,
    token: String,
) -> im_proto::ClientInfo {
    im_proto::ClientInfo {
        session_id: String::new(),
        app_ver: config.device.app_ver,
        package_code: config.device.package_code,
        plat: im_proto::Platform::Android as i32,
        language: config.device.language,
        sys_mac: config.device.sys_mac.clone(),
        sys_model: config.device.sys_model.clone(),
        token,
        version: format!("{}-{}", config.device.app_ver, config.device.package_code),
    }
}

async fn enrich_message_dto(
    config: &tokio::sync::RwLock<im_common::config::AppConfig>,
    auth_session: &tokio::sync::RwLock<Option<crate::state::AuthSession>>,
    http: &im_http::http_clients::AppHttpClients,
    message_crypto: &crate::message_content::MessageCryptoState,
    message: &im_proto::GroupMessage,
    dto: &mut MessageDto,
) {
    let Some(session) = auth_session.read().await.clone() else {
        dto.decode_error = Some("尚未登录，无法解密消息".to_string());
        return;
    };
    let config = config.read().await;
    let client_info = message_client_info(&config, session.token);
    drop(config);
    match message_crypto
        .decode_group_message(&http.im_biz, &client_info, message)
        .await
    {
        Ok(decoded) => dto.decoded_content = Some(decoded.content),
        Err(error) => dto.decode_error = Some(error),
    }
}

/// 以有界乱序执行 future，并按输入位置恢复输出顺序。
async fn map_ordered_bounded<T, U, F, Fut>(items: Vec<T>, limit: usize, map: F) -> Vec<U>
where
    F: Fn(T) -> Fut,
    Fut: Future<Output = U>,
{
    assert!(limit > 0, "bounded map concurrency must be positive");
    let mut indexed = futures::stream::iter(items.into_iter().enumerate())
        .map(|(index, item)| {
            let future = map(item);
            async move { (index, future.await) }
        })
        .buffer_unordered(limit)
        .collect::<Vec<_>>()
        .await;
    indexed.sort_unstable_by_key(|(index, _)| *index);
    indexed.into_iter().map(|(_, output)| output).collect()
}

/// 从当前位置读取一个不超过 `u64` 的 protobuf varint。
fn read_protobuf_varint(input: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut value = 0_u64;
    for byte_index in 0..10 {
        let byte = *input
            .get(*cursor)
            .ok_or_else(|| "truncated protobuf varint".to_string())?;
        *cursor += 1;
        if byte_index == 9 && byte > 1 {
            return Err("protobuf varint exceeds u64".to_string());
        }
        value |= u64::from(byte & 0x7f) << (byte_index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("protobuf varint exceeds ten bytes".to_string())
}

/// 在 Prost 分配 `GroupMessage` 对象前扫描 2202 顶层 wire 结构。
///
/// 扫描只维护游标、计数器和固定深度的 group 字段栈，不解析嵌套消息，也不分配与输入
/// 规模相关的内存。它只统计顶层 field 1 且 wire type 为 length-delimited 的出现次数；
/// 其他字段按其 wire type 安全跳过。数量上限是客户端解码结构预算，不赋予协议字段
/// 额外业务含义。
fn count_group_messages_before_decode(input: &[u8]) -> Result<usize, String> {
    const MAX_PROTOBUF_FIELD_NUMBER: u64 = (1 << 29) - 1;
    const MAX_GROUP_NESTING: usize = 64;

    let mut cursor = 0;
    let mut group_message_count = 0_usize;
    let mut group_fields = [0_u64; MAX_GROUP_NESTING];
    let mut group_depth = 0_usize;
    while cursor < input.len() {
        let key = read_protobuf_varint(input, &mut cursor)?;
        let field_number = key >> 3;
        if field_number == 0 || field_number > MAX_PROTOBUF_FIELD_NUMBER {
            return Err(format!("invalid protobuf field number {field_number}"));
        }
        let wire_type = key & 0x07;
        match wire_type {
            0 => {
                read_protobuf_varint(input, &mut cursor)?;
            }
            1 => {
                cursor = cursor
                    .checked_add(8)
                    .filter(|end| *end <= input.len())
                    .ok_or_else(|| "truncated protobuf fixed64 field".to_string())?;
            }
            2 => {
                let length = read_protobuf_varint(input, &mut cursor)?;
                let length = usize::try_from(length)
                    .map_err(|_| "protobuf length exceeds usize".to_string())?;
                cursor = cursor
                    .checked_add(length)
                    .filter(|end| *end <= input.len())
                    .ok_or_else(|| "truncated or overflowing protobuf length".to_string())?;
                if group_depth == 0 && field_number == 1 {
                    group_message_count = group_message_count
                        .checked_add(1)
                        .ok_or_else(|| "group_msg count overflow".to_string())?;
                    if group_message_count > MAX_GROUP_MESSAGES_PER_PUSH {
                        return Err(format!(
                            "group_msg count {group_message_count} exceeds structural limit \
                             {MAX_GROUP_MESSAGES_PER_PUSH}"
                        ));
                    }
                }
            }
            3 => {
                if group_depth == MAX_GROUP_NESTING {
                    return Err(format!(
                        "protobuf group nesting exceeds structural limit {MAX_GROUP_NESTING}"
                    ));
                }
                group_fields[group_depth] = field_number;
                group_depth += 1;
            }
            4 => {
                if group_depth == 0 {
                    return Err("unexpected protobuf end-group at top level".to_string());
                }
                group_depth -= 1;
                if group_fields[group_depth] != field_number {
                    return Err("mismatched protobuf end-group field number".to_string());
                }
            }
            5 => {
                cursor = cursor
                    .checked_add(4)
                    .filter(|end| *end <= input.len())
                    .ok_or_else(|| "truncated protobuf fixed32 field".to_string())?;
            }
            invalid => {
                return Err(format!(
                    "invalid or unsupported protobuf wire type {invalid}"
                ))
            }
        }
    }
    if group_depth != 0 {
        return Err("truncated protobuf group".to_string());
    }
    Ok(group_message_count)
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
    http: Arc<im_http::http_clients::AppHttpClients>,
    message_crypto: Arc<crate::message_content::MessageCryptoState>,
    message_channel: Arc<MessageChannelSlot>,
    connected: Arc<tokio::sync::RwLock<bool>>,
    shutdown: CancellationToken,
    app_handle: tauri::AppHandle,
}

impl ConnectionContext {
    /// 从当前认证会话取得活动账号数据库，并克隆连接任务所需的共享状态。
    ///
    /// 未登录或活动库与会话 UID 不一致时返回错误。返回的 `db` 绑定本次连接；
    /// generation 失效后不得继续接收或写入。
    async fn from_state(state: &AppState) -> Result<Self, String> {
        let session = authenticated_session_for_connect(&state.auth_session).await?;
        let db = state
            .account_db
            .require(session.uid)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            config: state.config.clone(),
            db,
            chat_client: state.chat_client.clone(),
            auth_session: state.auth_session.clone(),
            monitoring_groups: state.monitoring_groups.clone(),
            coordinator: state.connection_coordinator.clone(),
            http: state.http.clone(),
            message_crypto: state.message_crypto.clone(),
            message_channel: state.message_channel.clone(),
            connected: state.connected.clone(),
            shutdown: state.shutdown.clone(),
            app_handle: state.app_handle().clone(),
        })
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
    /// 正文占用的端到端字节许可；2202 解码后由其派生消息共享所有权。
    queue_byte_permit: Option<Arc<tokio::sync::OwnedSemaphorePermit>>,
}

/// 校验单帧大小、取得端到端正文总字节许可后送入有界队列。
///
/// 帧槽或 32 MiB 字节预算不足时等待而不丢帧；等待期间若连接取消，或 receiver 已关闭，
/// 则返回 TCP 帧错误。正文超出 8 MiB 时不会取得许可。2202 的许可由其派生消息及投影
/// 批次共享，直到该帧相关处理全部完成、取消或丢弃；其他帧处理结束即自动释放。
async fn enqueue_incoming_frame(
    sender: &mpsc::Sender<IncomingFrame>,
    mut frame: IncomingFrame,
    cancellation: &CancellationToken,
    byte_budget: &Arc<tokio::sync::Semaphore>,
) -> Result<(), im_common::error::AppError> {
    if frame.content.len() > MAX_QUEUED_MESSAGE_SIZE {
        return Err(im_common::error::AppError::TcpFrame(format!(
            "decoded message size {} exceeds queue limit {}",
            frame.content.len(),
            MAX_QUEUED_MESSAGE_SIZE
        )));
    }
    let message_id = frame.message_id;
    let queued_bytes = u32::try_from(frame.content.len()).map_err(|_| {
        im_common::error::AppError::TcpFrame(format!(
            "decoded message size {} cannot be represented by queue budget",
            frame.content.len()
        ))
    })?;
    let permit = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(im_common::error::AppError::TcpFrame(format!(
            "connection cancelled before message {} could reserve queue bytes",
            message_id
        ))),
        permit = byte_budget.clone().acquire_many_owned(queued_bytes) => permit.map_err(|_| {
            im_common::error::AppError::TcpFrame(
                "message queue byte budget closed".to_string()
            )
        })?,
    };
    frame.queue_byte_permit = Some(Arc::new(permit));
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
    /// 取得处理一个 PushGroupMessage 帧时使用的监控群组快照。
    async fn monitoring_snapshot(&self) -> std::collections::HashSet<i64>;
    /// 在一次事务中写入当前微批的全部监控消息；整批失败时返回 `false`。
    async fn persist_monitored_batch(&self, messages: &[im_proto::GroupMessage]) -> bool;
    /// 解密并向前端发送一个监控消息批次；失败只由实现记录，不改变落库或回执结果。
    async fn publish_monitored_batch(&self, messages: Vec<im_proto::GroupMessage>);
    /// 按群发送包含完整消息 ID 列表的 2102 接收回执。
    async fn acknowledge_group_messages(
        &self,
        group_id: i64,
        msg_ids: Vec<i64>,
    ) -> Result<(), im_common::error::AppError>;
}

/// 使用真实应用状态执行监控查询、持久化、Channel 推送和回执副作用。
struct ConnectionMessageEffects {
    context: ConnectionContext,
    sender: Arc<tokio::sync::OnceCell<im_chat::ChatSender>>,
    cancellation: CancellationToken,
}

#[async_trait::async_trait]
impl MessageEffects for ConnectionMessageEffects {
    async fn monitoring_snapshot(&self) -> std::collections::HashSet<i64> {
        self.context.monitoring_groups.read().await.clone()
    }

    async fn persist_monitored_batch(&self, messages: &[im_proto::GroupMessage]) -> bool {
        let session = self.context.auth_session.read().await.clone();
        let mut records: Vec<_> = messages
            .iter()
            .map(|message| stored_message_parts(message).0)
            .collect();

        tracing::info!(
            message_count = records.len(),
            "persist_monitored_batch: inserting {} messages",
            records.len()
        );

        if let Err(error) = self.context.db.messages.insert_batch(&records).await {
            tracing::error!(
                message_count = records.len(),
                "Failed to insert message batch: {error}"
            );
            return false;
        }

        tracing::info!(
            message_count = records.len(),
            "persist_monitored_batch: inserted, now processing..."
        );

        // 新消息入库后：对加密消息尝试解密并回填 content_text，同时检查匹配开奖配置。
        if let Some(session) = session {
            let config = self.context.db.lottery_config.get(session.uid).await.ok();
            let has_config = config
                .as_ref()
                .map(|c| !c.current_issues.is_empty())
                .unwrap_or(false);
            tracing::info!(
                uid = session.uid,
                has_config = has_config,
                issue_count = config.as_ref().map(|c| c.current_issues.len()).unwrap_or(0),
                "persist_monitored_batch: lottery config check"
            );
            if has_config || records.iter().any(|r| r.content_text.is_empty()) {
                // 需要解密密钥；尝试获取群相对密钥来解密。
                let config_guard = self.context.config.read().await;
                let client_info = message_client_info(&config_guard, session.token.clone());
                drop(config_guard);
                let mut decrypted_count = 0u32;
                for record in &mut records {
                    if record.content_text.is_empty() {
                        if let Some(raw) = &record.raw_proto {
                            if let Ok(msg) = im_proto::GroupMessage::decode(raw.as_slice()) {
                                if msg.version > 0 {
                                    // 通过 message_crypto 解密；如果失败（如密钥未就绪）则跳过。
                                    match self
                                        .context
                                        .message_crypto
                                        .decode_group_message(
                                            &self.context.http.im_biz,
                                            &client_info,
                                            &msg,
                                        )
                                        .await
                                    {
                                        Ok(d) => {
                                            let text = match d.content {
                                                crate::message_content::MediaContent::Text {
                                                    text,
                                                } => text,
                                                _ => String::new(),
                                            };
                                            tracing::debug!(
                                                msg_id = record.msg_id,
                                                group_id = record.group_id,
                                                version = msg.version,
                                                text_len = text.len(),
                                                "persist_monitored_batch: decrypted message"
                                            );
                                            record.content_text = text;
                                            sqlx::query("UPDATE messages SET content_text = ? WHERE msg_id = ?")
                                                .bind(&record.content_text)
                                                .bind(record.msg_id)
                                                .execute(&self.context.db.pool)
                                                .await
                                                .ok();
                                            decrypted_count += 1;
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                msg_id = record.msg_id,
                                                group_id = record.group_id,
                                                error = %e,
                                                "persist_monitored_batch: decryption failed"
                                            );
                                        }
                                    }
                                } else {
                                    // version == 0：明文消息，content_text 已在 stored_message_parts 中填充。
                                    tracing::debug!(
                                        msg_id = record.msg_id,
                                        group_id = record.group_id,
                                        text_len = record.content_text.len(),
                                        "persist_monitored_batch: plaintext message (version=0)"
                                    );
                                }
                            } else {
                                tracing::debug!(
                                    msg_id = record.msg_id,
                                    "persist_monitored_batch: failed to re-decode raw_proto"
                                );
                            }
                        }
                    }
                }
                tracing::info!(
                    record_count = records.len(),
                    decrypted = decrypted_count,
                    "persist_monitored_batch: decryption pass complete"
                );
            }
            // 检查匹配开奖配置。
            if let Some(config) = config {
                if !config.current_issues.is_empty() {
                    let mut updated = 0usize;
                    for record in &records {
                        let text = &record.content_text;
                        let is_matched = text.contains("开奖")
                            && config
                                .current_issues
                                .iter()
                                .any(|issue| text.contains(&issue.to_string()));
                        if is_matched {
                            tracing::info!(
                                uid = session.uid,
                                msg_id = record.msg_id,
                                issue = config
                                    .current_issues
                                    .iter()
                                    .map(|i| i.to_string())
                                    .collect::<Vec<_>>()
                                    .join(","),
                                "persist_monitored_batch: MATCHED lottery message"
                            );
                            sqlx::query("UPDATE messages SET matched = 1 WHERE msg_id = ?")
                                .bind(record.msg_id)
                                .execute(&self.context.db.pool)
                                .await
                                .ok();
                            updated += 1;
                        } else if !text.is_empty() {
                            tracing::debug!(
                                uid = session.uid,
                                msg_id = record.msg_id,
                                text = %text,
                                "persist_monitored_batch: no match (no '开奖' or issue)"
                            );
                        } else {
                            tracing::debug!(
                                uid = session.uid,
                                msg_id = record.msg_id,
                                version = record.content.len(),
                                "persist_monitored_batch: empty content_text (no decryption available)"
                            );
                        }
                    }
                    if updated > 0 {
                        tracing::info!(
                            uid = session.uid,
                            updated = updated,
                            "Matched new messages against lottery config"
                        );
                    } else {
                        tracing::info!(
                            uid = session.uid,
                            record_count = records.len(),
                            "No lottery matches found in this batch"
                        );
                    }
                }
            }
        }
        true
    }

    async fn publish_monitored_batch(&self, messages: Vec<im_proto::GroupMessage>) {
        let context = self.context.clone();
        let mut dtos = map_ordered_bounded(messages, MESSAGE_DECRYPT_CONCURRENCY, move |message| {
            let context = context.clone();
            async move {
                let (_, mut dto) = stored_message_parts(&message);
                enrich_message_dto(
                    &context.config,
                    &context.auth_session,
                    &context.http,
                    &context.message_crypto,
                    &message,
                    &mut dto,
                )
                .await;
                dto
            }
        })
        .await;

        // 从数据库读取最新的 matched 值，因为 persist_monitored_batch 可能已更新。
        // stored_message_parts 中 matched 硬编码为 0，此处修正为 DB 中的实际值。
        let db_matched: std::collections::HashMap<i64, i32> = {
            let ids: Vec<String> = dtos.iter().map(|d| d.msg_id.clone()).collect();
            let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT msg_id, matched FROM messages WHERE msg_id IN ({})",
                placeholders
            );
            let mut q = sqlx::query_as::<_, (i64, i32)>(&sql);
            for id_str in &ids {
                q = q.bind(id_str.parse::<i64>().unwrap_or(0));
            }
            q.fetch_all(&self.context.db.pool)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect()
        };
        tracing::debug!(
            fetched = db_matched.len(),
            total = dtos.len(),
            "publish_monitored_batch: loaded matched from DB"
        );
        for dto in &mut dtos {
            if let Ok(msg_id) = dto.msg_id.parse::<i64>() {
                if let Some(&matched) = db_matched.get(&msg_id) {
                    dto.matched = matched;
                }
            }
        }

        match publish_realtime_message(&self.context.message_channel, &dtos).await {
            Ok(()) => {
                #[cfg(debug_assertions)]
                tracing::info!(
                    message_count = dtos.len(),
                    "Persisted message batch sent through frontend Channel"
                );
                #[cfg(not(debug_assertions))]
                tracing::debug!(
                    message_count = dtos.len(),
                    "Persisted message batch sent through frontend Channel"
                );
            }
            Err(error) => tracing::warn!(
                message_count = dtos.len(),
                "Failed to send persisted message batch through frontend Channel: {error}"
            ),
        }
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
    server_user_key_pair: im_proto::KeyPairBase,
    _message_worker: tokio::task::JoinHandle<()>,
}

#[tauri::command]
/// 登记当前页面用于接收已入库实时消息的 Channel。
///
/// 后注册者替换旧页面的 Channel，适配开发热重载和 WebView 重新加载；该命令不读取
/// 历史消息，调用方仍须通过 `get_messages` 完成初始快照。
pub async fn register_message_channel(
    state: State<'_, AppState>,
    on_message: tauri::ipc::Channel<Vec<MessageDto>>,
) -> Result<(), String> {
    replace_message_channel(&state.message_channel, on_message).await;
    tracing::info!("Frontend realtime message Channel registered");
    Ok(())
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
    let context = ConnectionContext::from_state(state).await?;
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
        server_user_key_pair,
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
                start_user_key_pair_sync(
                    context.clone(),
                    auth_session.clone(),
                    server_user_key_pair,
                    generation_cancellation.clone(),
                );
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
/// 先创建容量为 64、正文预算为 32 MiB 的 mpsc 接收缓冲和消息 worker，再安装入站
/// handler 与断线回调。
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
    let frame_byte_budget = Arc::new(tokio::sync::Semaphore::new(MESSAGE_QUEUE_BYTE_BUDGET));
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
        let byte_budget = frame_byte_budget.clone();
        async move {
            if cancellation.is_cancelled() {
                return Ok(());
            }
            enqueue_incoming_frame(
                &sender,
                IncomingFrame {
                    message_id,
                    content,
                    queue_byte_permit: None,
                },
                &cancellation,
                &byte_budget,
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

    let login_success = match network_result {
        Ok(login_success) => login_success,
        Err(error) => {
            connection_cancellation.cancel();
            disconnect_local_client(&mut chat_client).await;
            let _ = tokio::time::timeout(CHAT_DISCONNECT_TIMEOUT, message_worker).await;
            return Err(error);
        }
    };
    let server_user_key_pair = login_user_key_metadata(login_success)?;

    Ok(EstablishedConnection {
        client: chat_client,
        installed,
        connection_lost,
        connection_cancellation,
        server_user_key_pair,
        _message_worker: message_worker,
    })
}

/// 提取 1201 中服务端公布的当前 App 公钥和版本。
///
/// `user_key_pair` 字段是后续消息加解密所依赖的服务器公钥元数据；缺失时登录失败，
/// 避免使用空密钥对继续建立连接，导致后续所有消息解密静默失败。
fn login_user_key_metadata(
    login_success: im_proto::PushLoginSuccessMessage,
) -> Result<im_proto::KeyPairBase, String> {
    login_success
        .user_key_pair
        .ok_or_else(|| "服务端未返回 App 公钥元数据（user_key_pair）".to_string())
}

/// 在连接已发布为在线后恢复或登记当前账号的本地 App 密钥对。
///
/// 任务受当前认证 generation 的取消信号约束。任何失败只记录不含密钥内容的警告；
/// 它不会撤销 TCP 连接，后续消息仍会入库和回执，并在 DTO 中携带解密错误。
fn start_user_key_pair_sync(
    context: ConnectionContext,
    auth_session: crate::state::AuthSession,
    server_key_pair: im_proto::KeyPairBase,
    cancellation: CancellationToken,
) {
    tokio::spawn(async move {
        let config = context.config.read().await;
        let client_info = message_client_info(&config, auth_session.token);
        drop(config);
        let http = context.http.clone();
        let synchronization = crate::message_content::synchronize_user_key_pair(
            &context.message_crypto,
            &context.db.key_pairs,
            auth_session.uid,
            &server_key_pair,
            move |public_key| async move {
                http.im_biz
                    .update_user_key_pair(&client_info, &public_key)
                    .await
                    .map_err(|error| format!("登记用户 App 公钥失败：{error}"))
            },
        );
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {}
            result = synchronization => {
                match result {
                    Ok(()) => {
                        tracing::info!(
                            uid = auth_session.uid,
                            "User App key pair is ready for group message decryption"
                        );
                        if let Err(error) = context.app_handle.emit("message_keys_ready", ()) {
                            tracing::warn!("Failed to emit message_keys_ready: {error}");
                        }
                    }
                    Err(error) => tracing::warn!(
                        uid = auth_session.uid,
                        %error,
                        "User App key pair synchronization failed; chat remains connected"
                    ),
                }
            }
        }
    });
}

/// 驱动当前连接的串行消息处理循环。
async fn run_message_worker(
    receiver: mpsc::Receiver<IncomingFrame>,
    effects: Arc<dyn MessageEffects>,
    cancellation: CancellationToken,
    login_sender: tokio::sync::oneshot::Sender<im_proto::PushLoginSuccessMessage>,
) {
    run_message_worker_with_effects(receiver, effects, cancellation, login_sender).await;
}

/// 微批中一条群消息及其所在 2202 帧读取到的监控判定。
struct PendingGroupMessage {
    message: im_proto::GroupMessage,
    monitored: bool,
    /// 与来源帧共享的正文预算许可；同帧拆分到多个批次时由最后一个引用归还。
    frame_byte_permit: Option<Arc<tokio::sync::OwnedSemaphorePermit>>,
}

/// 等待或正在执行正文解密与 Channel 发送的监控消息批次。
struct ProjectionMessageBatch {
    messages: Vec<im_proto::GroupMessage>,
    /// 保持来源帧字节许可直至本批投影完成、取消或被队列丢弃。
    frame_byte_permits: Vec<Arc<tokio::sync::OwnedSemaphorePermit>>,
}

/// 提交一个 2202 微批，并依次执行事务写入、分群回执和投影排队。
///
/// 未监控消息不入库但始终进入回执；监控消息仅在整批事务成功后进入回执。取消发生在
/// 事务完成前时不会开始回执；回执全部成功后，监控消息才进入容量受限的投影队列。
/// 队列已满时这里等待形成背压，取消或投影 worker 退出会停止当前批次排队。
async fn flush_group_message_batch(
    pending: &mut Vec<PendingGroupMessage>,
    effects: &dyn MessageEffects,
    cancellation: &CancellationToken,
    projection_sender: &mpsc::Sender<ProjectionMessageBatch>,
) -> bool {
    if pending.is_empty() {
        return true;
    }
    let batch = std::mem::take(pending);
    let monitored_messages = batch
        .iter()
        .filter(|item| item.monitored)
        .map(|item| item.message.clone())
        .collect::<Vec<_>>();
    let monitored_frame_permits = batch
        .iter()
        .filter(|item| item.monitored)
        .filter_map(|item| item.frame_byte_permit.clone())
        .collect::<Vec<_>>();
    let monitored_persisted = if monitored_messages.is_empty() {
        true
    } else {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return false,
            persisted = effects.persist_monitored_batch(&monitored_messages) => persisted,
        }
    };

    let mut receipts = std::collections::BTreeMap::<i64, Vec<i64>>::new();
    for item in &batch {
        if !item.monitored || monitored_persisted {
            receipts
                .entry(item.message.group_id)
                .or_default()
                .push(item.message.msg_id);
        }
    }
    for (group_id, msg_ids) in receipts {
        let receipt_result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return false,
            result = effects.acknowledge_group_messages(group_id, msg_ids) => result,
        };
        if let Err(error) = receipt_result {
            tracing::error!(
                group_id,
                "Failed to acknowledge received group messages: {error}"
            );
            cancellation.cancel();
            return false;
        }
    }
    if monitored_persisted && !monitored_messages.is_empty() {
        let projection_result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return false,
            result = projection_sender.send(ProjectionMessageBatch {
                messages: monitored_messages,
                frame_byte_permits: monitored_frame_permits,
            }) => result,
        };
        if projection_result.is_err() {
            tracing::error!("Message projection queue closed before batch could be delivered");
            cancellation.cancel();
            return false;
        }
    }
    true
}

/// 串行消费已提交批次，并将每批交给副作用层执行有界并发解密及单次 Channel 发送。
///
/// 连接取消时优先停止当前投影并关闭 receiver，尚在队列中的批次随 receiver drop 丢弃；
/// 正常输入结束时则在 sender 全部释放后处理完已有批次再返回。
async fn run_message_projection_worker(
    mut receiver: mpsc::Receiver<ProjectionMessageBatch>,
    effects: Arc<dyn MessageEffects>,
    cancellation: CancellationToken,
) {
    loop {
        let batch = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            batch = receiver.recv() => match batch {
                Some(batch) => batch,
                None => break,
            },
        };
        let ProjectionMessageBatch {
            messages,
            frame_byte_permits,
        } = batch;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            _ = effects.publish_monitored_batch(messages) => {}
        }
        drop(frame_byte_permits);
    }
    receiver.close();
}

/// 按入队顺序处理聊天推送及其副作用。
///
/// 1201 立即解码并完成登录 oneshot，失败时取消连接。2202 先验证顶层 wire 结构预算，
/// 再由 Prost 解码；每帧只读取一次监控集合，并按“最多 100 条或首条等待 25ms”微批。
/// 2205 仍仅记录预留日志。取消分支优先并丢弃未提交批次。事务与回执留在本 worker；
/// 已回执监控批次进入独立有界队列，由单一投影 worker 完成解密和 Channel，避免单批
/// 慢投影直接阻塞后续收帧。
async fn run_message_worker_with_effects(
    mut receiver: mpsc::Receiver<IncomingFrame>,
    effects: Arc<dyn MessageEffects>,
    cancellation: CancellationToken,
    login_sender: tokio::sync::oneshot::Sender<im_proto::PushLoginSuccessMessage>,
) {
    let (projection_sender, projection_receiver) = mpsc::channel(MESSAGE_PROJECTION_QUEUE_CAPACITY);
    let projection_effects = effects.clone();
    let projection_cancellation = cancellation.clone();
    let projection_worker = tokio::spawn(async move {
        run_message_projection_worker(
            projection_receiver,
            projection_effects,
            projection_cancellation,
        )
        .await;
    });
    let mut login_sender = Some(login_sender);
    let mut pending = Vec::<PendingGroupMessage>::with_capacity(MESSAGE_BATCH_MAX_MESSAGES);
    let mut batch_deadline = None::<tokio::time::Instant>;
    'message_loop: loop {
        let frame = if let Some(deadline) = batch_deadline {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                _ = tokio::time::sleep_until(deadline) => {
                    if !flush_group_message_batch(
                        &mut pending,
                        effects.as_ref(),
                        &cancellation,
                        &projection_sender,
                    ).await {
                        break;
                    }
                    batch_deadline = None;
                    continue;
                }
                frame = receiver.recv() => frame,
            }
        } else {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                frame = receiver.recv() => frame,
            }
        };
        let Some(mut frame) = frame else {
            if !flush_group_message_batch(
                &mut pending,
                effects.as_ref(),
                &cancellation,
                &projection_sender,
            )
            .await
            {
                break;
            }
            break;
        };
        match frame.message_id {
            im_chat::heartbeat::PUSH_LOGIN_SUCCESS => {
                let login_success =
                    match im_proto::PushLoginSuccessMessage::decode(frame.content.as_slice()) {
                        Ok(message) => message,
                        Err(error) => {
                            tracing::warn!("Failed to decode PushLoginSuccessMessage: {error}");
                            cancellation.cancel();
                            break;
                        }
                    };
                if let Some(sender) = login_sender.take() {
                    let _ = sender.send(login_success);
                }
            }
            im_chat::heartbeat::PUSH_GROUP_MESSAGE => {
                if let Err(error) = count_group_messages_before_decode(frame.content.as_slice()) {
                    tracing::warn!(
                        "Failed to validate PushGroupMessage wire structure before decode: {error}"
                    );
                    continue;
                }
                let push = match im_proto::PushGroupMessage::decode(frame.content.as_slice()) {
                    Ok(push) => push,
                    Err(error) => {
                        tracing::warn!("Failed to decode PushGroupMessage: {error}");
                        continue;
                    }
                };
                let monitoring = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break,
                    snapshot = effects.monitoring_snapshot() => snapshot,
                };
                let frame_byte_permit = frame.queue_byte_permit.take();
                for group_message in push.group_msg {
                    if pending.is_empty() {
                        batch_deadline =
                            Some(tokio::time::Instant::now() + MESSAGE_BATCH_MAX_DELAY);
                    }
                    let monitored = monitoring.contains(&group_message.group_id);
                    pending.push(PendingGroupMessage {
                        message: group_message,
                        monitored,
                        frame_byte_permit: frame_byte_permit.clone(),
                    });
                    if pending.len() == MESSAGE_BATCH_MAX_MESSAGES {
                        if !flush_group_message_batch(
                            &mut pending,
                            effects.as_ref(),
                            &cancellation,
                            &projection_sender,
                        )
                        .await
                        {
                            break 'message_loop;
                        }
                        batch_deadline = None;
                    }
                }
            }
            im_chat::heartbeat::PUSH_RECALL_GROUP_MESSAGE => {
                tracing::info!("Received group-message recall push (2205); handling reserved");
            }
            message_id => tracing::debug!("Ignoring unsupported chat message {message_id}"),
        }
    }
    // 关闭接收端会丢弃仍排队及之后发送的帧；已完成的事务或回执不会被取消回滚。
    receiver.close();
    drop(projection_sender);
    if let Err(error) = projection_worker.await {
        tracing::warn!("Message projection worker failed to join: {error}");
    }
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
        server_user_key_pair,
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
                start_user_key_pair_sync(
                    context.clone(),
                    auth_session.clone(),
                    server_user_key_pair,
                    generation_cancellation.clone(),
                );
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
/// 分页查询指定群或全部群组的已存储消息。
///
/// `group_id` 为 `Some` 时必须是 64 位十进制整数并只查询该群；为 `None` 时跨群读取
/// 最近消息。必须已登录且活动库属于当前会话 UID。`before_send_time` 与字符串
/// `before_msg_id` 必须同时提供或同时省略；`limit` 省略时为 200，且不得超过 200。
/// 查询后以最多 8 路并发解密并恢复存储顺序；单条解密失败只写入 DTO 的
/// `decode_error`，不令整页失败。
pub async fn get_messages(
    state: State<'_, AppState>,
    group_id: Option<String>,
    limit: Option<usize>,
    before_send_time: Option<i64>,
    before_msg_id: Option<String>,
    matched_only: Option<bool>,
) -> Result<MessagePageDto, String> {
    let (limit, cursor) = validate_message_page(limit, before_send_time, before_msg_id.as_deref())?;
    let matched_only = matched_only.unwrap_or(false);
    let session = authenticated_session_for_connect(&state.auth_session).await?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|error| error.to_string())?;
    tracing::info!(
        uid = session.uid,
        group_id = group_id.as_deref().unwrap_or("all"),
        matched_only,
        "get_messages: loading messages"
    );
    let page = match group_id {
        Some(group_id) => {
            let group_id = super::parse_i64_id(&group_id, "group_id")?;
            db.messages
                .get_by_group(group_id, limit, cursor, matched_only)
                .await
        }
        None => db.messages.get_recent(limit, cursor, matched_only).await,
    }
    .map_err(|error| error.to_string())?;
    tracing::info!(
        uid = session.uid,
        page_size = page.messages.len(),
        has_more = page.has_more,
        matched_only,
        "get_messages: loaded {} messages",
        page.messages.len()
    );

    let messages = map_ordered_bounded(page.messages, MESSAGE_DECRYPT_CONCURRENCY, |row| async {
        let message = row
            .raw_proto
            .as_deref()
            .and_then(|bytes| im_proto::GroupMessage::decode(bytes).ok());
        let mut dto = message_dto_from_row(row);
        if let Some(message) = message {
            enrich_message_dto(
                &state.config,
                &state.auth_session,
                &state.http,
                &state.message_crypto,
                &message,
                &mut dto,
            )
            .await;
        } else {
            dto.decode_error = Some("消息缺少可解码的原始协议数据".to_string());
        }
        dto
    })
    .await;
    tracing::info!(
        uid = session.uid,
        returned = messages.len(),
        matched_count = messages.iter().filter(|m| m.matched != 0).count(),
        "get_messages: built DTOs, sending to frontend"
    );
    Ok(MessagePageDto {
        messages,
        next_cursor: page.next_cursor.map(|cursor| MessageCursorDto {
            send_time: cursor.send_time,
            msg_id: cursor.msg_id.to_string(),
        }),
        has_more: page.has_more,
    })
}

const MAX_ATTACHMENT_DOWNLOAD_SIZE: usize = 256 * 1024 * 1024;

/// 已解密到本地缓存的附件定位信息。
#[derive(serde::Serialize)]
pub struct AttachmentDownloadDto {
    /// 可交给 Tauri `convertFileSrc` 的绝对路径。
    pub path: String,
    /// 消息协议携带或按媒体类型推导的 MIME。
    pub mime_type: String,
}

#[tauri::command]
/// 下载并解密一条媒体消息的主附件或缩略图，返回本地缓存绝对路径。
///
/// 命令从 SQLite 的 `raw_proto` 恢复完整群消息，再按需获取群密钥、解开
/// `attachment_key` 和正文 Protobuf，最后对 OSS 密文执行 PC 分块或整文件 AES 解密。
/// 只允许 HTTP(S) URL，下载上限为 256 MiB；明文写入 Tauri 应用缓存目录，不覆盖
/// 该消息与附件类型之外的路径。
pub async fn download_message_attachment(
    state: State<'_, AppState>,
    msg_id: String,
    thumbnail: bool,
) -> Result<AttachmentDownloadDto, String> {
    let msg_id = super::parse_i64_id(&msg_id, "msg_id")?;
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "尚未登录".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|error| error.to_string())?;
    let row = db
        .messages
        .get_by_id(msg_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("消息 {msg_id} 不存在"))?;
    let raw_proto = row
        .raw_proto
        .ok_or_else(|| format!("消息 {msg_id} 缺少原始协议数据"))?;
    let message = im_proto::GroupMessage::decode(raw_proto.as_slice())
        .map_err(|error| format!("消息 {msg_id} 协议解析失败：{error}"))?;
    let config = state.config.read().await;
    let client_info = message_client_info(&config, session.token);
    drop(config);
    let decoded = state
        .message_crypto
        .decode_group_message(&state.http.im_biz, &client_info, &message)
        .await?;
    let descriptor = decoded
        .content
        .attachment(thumbnail)
        .ok_or_else(|| "该消息没有可下载附件".to_string())?;
    let file_key = decoded
        .file_key
        .ok_or_else(|| "消息缺少附件解密密钥".to_string())?;
    let url =
        reqwest::Url::parse(&descriptor.url).map_err(|error| format!("附件 URL 无效：{error}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("附件 URL 必须是无认证信息的 HTTP(S) 地址".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("创建附件下载客户端失败：{error}"))?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| format!("附件下载失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("附件下载失败：{error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ATTACHMENT_DOWNLOAD_SIZE as u64)
    {
        return Err("附件超过 256 MiB 下载上限".to_string());
    }
    let mut ciphertext = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取附件失败：{error}"))?
    {
        let next_len = ciphertext
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "附件长度溢出".to_string())?;
        if next_len > MAX_ATTACHMENT_DOWNLOAD_SIZE {
            return Err("附件超过 256 MiB 下载上限".to_string());
        }
        ciphertext.extend_from_slice(&chunk);
    }
    let plaintext = crate::message_content::decrypt_attachment_bytes(&file_key, &ciphertext)?;

    let source_name = if descriptor.name.contains('.') {
        descriptor.name
    } else {
        url.path_segments()
            .and_then(|mut segments| segments.rfind(|part| !part.is_empty()))
            .filter(|name| !name.is_empty())
            .unwrap_or(&descriptor.name)
            .to_string()
    };
    let cache_dir = state
        .app_handle()
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法取得应用缓存目录：{error}"))?
        .join("media");
    tokio::fs::create_dir_all(&cache_dir)
        .await
        .map_err(|error| format!("创建附件缓存目录失败：{error}"))?;
    let cache_path = cache_dir.join(safe_attachment_filename(msg_id, thumbnail, &source_name));
    tokio::fs::write(&cache_path, plaintext)
        .await
        .map_err(|error| format!("写入附件缓存失败：{error}"))?;
    Ok(AttachmentDownloadDto {
        path: cache_path.to_string_lossy().into_owned(),
        mime_type: descriptor.mime_type,
    })
}

fn safe_attachment_filename(msg_id: i64, thumbnail: bool, source_name: &str) -> String {
    let basename = std::path::Path::new(source_name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment");
    let sanitized: String = basename
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = if sanitized.trim_matches(['.', '_']).is_empty() {
        "attachment"
    } else {
        sanitized.as_str()
    };
    format!(
        "{msg_id}-{}-{sanitized}",
        if thumbnail { "thumbnail" } else { "main" }
    )
}

/// 校验页长和成对游标，并把字符串消息 ID 转为存储层复合键。
fn validate_message_page(
    limit: Option<usize>,
    before_send_time: Option<i64>,
    before_msg_id: Option<&str>,
) -> Result<(usize, Option<im_store::message::MessageCursor>), String> {
    let limit = limit.unwrap_or(im_store::message::MAX_MESSAGE_PAGE_LIMIT);
    if !(1..=im_store::message::MAX_MESSAGE_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "limit must be between 1 and {}",
            im_store::message::MAX_MESSAGE_PAGE_LIMIT
        ));
    }
    let cursor = match (before_send_time, before_msg_id) {
        (None, None) => None,
        (Some(send_time), Some(msg_id)) => Some(im_store::message::MessageCursor {
            send_time,
            msg_id: super::parse_i64_id(msg_id, "before_msg_id")?,
        }),
        _ => {
            return Err("before_send_time and before_msg_id must be provided together".to_string())
        }
    };
    Ok((limit, cursor))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

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
        login_user_key_metadata, mark_connected_and_broadcast, mark_disconnected_and_broadcast,
        message_dto_from_row, publish_realtime_message, replace_message_channel,
        retry_automatic_connection, run_cancellable_with_timeout, run_message_worker_with_effects,
        stored_message_parts, validate_message_page, ConnectionAttemptGuard, IncomingFrame,
        MessageCursorDto, MessageDto, MessageEffects, MessagePageDto, HEARTBEAT_INTERVAL,
        MAX_QUEUED_MESSAGE_SIZE, MESSAGE_BATCH_MAX_MESSAGES, MESSAGE_DECRYPT_CONCURRENCY,
        MESSAGE_PROJECTION_QUEUE_CAPACITY, MESSAGE_QUEUE_BYTE_BUDGET, MESSAGE_QUEUE_CAPACITY,
    };

    fn installed_client(client: im_chat::ChatClient) -> InstalledClient {
        InstalledClient::new(crate::state::ConnectionAttemptKey::new(0, 1), client)
    }

    fn full_monitored_batch_frame(batch_id: i64) -> IncomingFrame {
        IncomingFrame {
            message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
            content: im_proto::PushGroupMessage {
                group_msg: (0..MESSAGE_BATCH_MAX_MESSAGES as i64)
                    .map(|offset| im_proto::GroupMessage {
                        msg_id: batch_id * 1_000 + offset,
                        group_id: 7,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }
            .encode_to_vec(),
            queue_byte_permit: None,
        }
    }

    #[tokio::test]
    async fn newest_registered_channel_receives_realtime_message_batch() {
        let slot = tokio::sync::RwLock::new(None);
        let first_payloads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let first_payloads_for_channel = first_payloads.clone();
        let first = tauri::ipc::Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body {
                first_payloads_for_channel.lock().unwrap().push(json);
            }
            Ok(())
        });
        replace_message_channel(&slot, first).await;

        let latest_payloads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let latest_payloads_for_channel = latest_payloads.clone();
        let latest = tauri::ipc::Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body {
                latest_payloads_for_channel.lock().unwrap().push(json);
            }
            Ok(())
        });
        replace_message_channel(&slot, latest).await;

        publish_realtime_message(
            &slot,
            &[MessageDto {
                msg_id: "80".to_string(),
                group_id: "8".to_string(),
                send_uid: "42".to_string(),
                msg_type: 0,
                group_name: "群 8".to_string(),
                content_b64: String::new(),
                decoded_content: None,
                decode_error: None,
                send_time: 20,
                content_md5: String::new(),
                stored_at: None,
                matched: 0,
            }],
        )
        .await
        .unwrap();

        assert!(first_payloads.lock().unwrap().is_empty());
        let payloads = latest_payloads.lock().unwrap();
        assert_eq!(payloads.len(), 1);
        assert!(payloads[0].starts_with('['));
        assert!(payloads[0].contains(r#""msg_id":"80""#));
    }

    // 分页边界：limit 省略时取 200；游标必须成对出现，消息 ID 按十进制字符串安全解析。
    #[test]
    fn message_page_validates_limit_and_paired_cursor() {
        assert_eq!(
            validate_message_page(None, None, None).unwrap(),
            (200, None)
        );
        assert!(validate_message_page(Some(0), None, None).is_err());
        assert!(validate_message_page(Some(201), None, None).is_err());
        assert!(validate_message_page(Some(10), Some(100), None).is_err());
        assert!(validate_message_page(Some(10), None, Some("9")).is_err());
        assert!(validate_message_page(Some(10), Some(100), Some("9223372036854775808")).is_err());
        assert_eq!(
            validate_message_page(Some(10), Some(100), Some("9")).unwrap(),
            (
                10,
                Some(im_store::message::MessageCursor {
                    send_time: 100,
                    msg_id: 9,
                })
            )
        );
    }

    #[test]
    fn message_page_serializes_camel_case_cursor_without_losing_large_id() {
        let page = MessagePageDto {
            messages: Vec::new(),
            next_cursor: Some(MessageCursorDto {
                send_time: 100,
                msg_id: "9007199254740993".to_string(),
            }),
            has_more: true,
        };

        assert_eq!(
            serde_json::to_value(page).unwrap(),
            serde_json::json!({
                "messages": [],
                "nextCursor": {
                    "sendTime": 100,
                    "msgId": "9007199254740993"
                },
                "hasMore": true
            })
        );
    }

    #[test]
    fn login_success_with_key_pair_returns_metadata() {
        let login = im_proto::PushLoginSuccessMessage {
            user_key_pair: Some(im_proto::KeyPairBase {
                public_key: "server-public-key".to_string(),
                key_version: 1,
                ..Default::default()
            }),
            ..Default::default()
        };

        let metadata = login_user_key_metadata(login).expect("should return metadata");

        assert_eq!(metadata.public_key, "server-public-key");
        assert_eq!(metadata.key_version, 1);
        assert!(metadata.private_key.is_empty());
    }

    #[test]
    fn login_success_without_key_pair_returns_error() {
        let login = im_proto::PushLoginSuccessMessage::default();

        let result = login_user_key_metadata(login);

        assert!(result.is_err());
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
        let budget = Arc::new(tokio::sync::Semaphore::new(MESSAGE_QUEUE_BYTE_BUDGET));
        let oversized = enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 2202,
                content: vec![0; MAX_QUEUED_MESSAGE_SIZE + 1],
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap_err();
        assert!(oversized.to_string().contains("exceeds queue limit"));
    }

    // 接收缓冲基线：帧数上限为 64，字节预算独立限制整条未完成链路的正文。
    #[test]
    fn receive_buffer_uses_planned_frame_and_byte_limits() {
        assert_eq!(MESSAGE_QUEUE_CAPACITY, 64);
        assert_eq!(super::MESSAGE_QUEUE_BYTE_BUDGET, 32 * 1024 * 1024);
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

    // 解码前结构预算：恰好 10,000 个 field 1 可通过，第 10,001 个立即拒绝。
    #[test]
    fn push_group_message_prescan_accepts_limit_and_rejects_one_over_limit() {
        let mut payload = [0x0a, 0x00].repeat(super::MAX_GROUP_MESSAGES_PER_PUSH);
        assert_eq!(
            super::count_group_messages_before_decode(&payload).unwrap(),
            super::MAX_GROUP_MESSAGES_PER_PUSH
        );
        assert_eq!(
            im_proto::PushGroupMessage::decode(payload.as_slice())
                .unwrap()
                .group_msg
                .len(),
            super::MAX_GROUP_MESSAGES_PER_PUSH
        );

        payload.extend_from_slice(&[0x0a, 0x00]);
        assert!(super::count_group_messages_before_decode(&payload)
            .unwrap_err()
            .contains("exceeds structural limit"));
    }

    // 未知字段：预扫描按 wire type 跳过 varint、fixed64、length-delimited 和 fixed32。
    #[test]
    fn push_group_message_prescan_skips_unknown_fields_by_wire_type() {
        let payload = [
            0x10, 0x96, 0x01, // field 2, varint
            0x19, 1, 2, 3, 4, 5, 6, 7, 8, // field 3, fixed64
            0x22, 0x03, b'a', b'b', b'c', // field 4, length-delimited
            0x2d, 1, 2, 3, 4, // field 5, fixed32
            0x33, 0x0a, 0x00, 0x34, // field 6 group，内部 field 1 不计入顶层
            0x0a, 0x00, // field 1, empty GroupMessage
        ];

        assert_eq!(
            super::count_group_messages_before_decode(&payload).unwrap(),
            1
        );
    }

    // 畸形 wire：截断/溢出的 varint、越界长度、截断定长字段和非法 wire type 均拒绝。
    #[test]
    fn push_group_message_prescan_rejects_malformed_wire_values() {
        let mut overflowing_length = vec![0x0a];
        overflowing_length.extend(std::iter::repeat_n(0xff, 9));
        overflowing_length.push(0x01);
        let malformed = [
            vec![0x80],
            vec![0x10, 0x80],
            vec![0x0a, 0x80],
            vec![0x0a, 0x05, 0x00],
            overflowing_length,
            vec![
                0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02,
            ],
            vec![0x19, 1, 2],
            vec![0x2d, 1, 2],
            vec![0x0e],
            vec![0x00],
            vec![0x0b],
            vec![0x0b, 0x14],
        ];

        for payload in malformed {
            assert!(
                super::count_group_messages_before_decode(&payload).is_err(),
                "malformed payload unexpectedly passed: {payload:?}"
            );
        }
    }

    // worker 门禁：超过结构预算的 2202 在监控快照及 Prost 完整解码前被丢弃。
    #[tokio::test]
    async fn message_worker_rejects_oversized_group_message_structure_before_dispatch() {
        let effects = Arc::new(FakeMessageEffects::default());
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let payload = [0x0a, 0x00].repeat(super::MAX_GROUP_MESSAGES_PER_PUSH.saturating_add(1));
        sender
            .send(IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
                content: payload,
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        drop(sender);

        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        assert_eq!(effects.snapshot_calls.load(Ordering::SeqCst), 0);
        assert_eq!(effects.persist_calls.load(Ordering::SeqCst), 0);
        assert!(effects.acknowledged.lock().await.is_empty());
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
        let budget = Arc::new(tokio::sync::Semaphore::new(MESSAGE_QUEUE_BYTE_BUDGET));
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 3203,
                content: Vec::new(),
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap();

        let pending_sender = sender.clone();
        let pending_cancellation = cancellation.clone();
        let pending_budget = budget.clone();
        let (enqueue_started, enqueue_is_running) = tokio::sync::oneshot::channel();
        let mut pending = tokio::spawn(async move {
            let _ = enqueue_started.send(());
            enqueue_incoming_frame(
                &pending_sender,
                IncomingFrame {
                    message_id: 3204,
                    content: Vec::new(),
                    queue_byte_permit: None,
                },
                &pending_cancellation,
                &pending_budget,
            )
            .await
        });
        enqueue_is_running.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut pending)
                .await
                .is_err(),
            "second frame must remain blocked while the only queue slot is occupied"
        );

        assert_eq!(receiver.recv().await.unwrap().message_id, 3203);
        pending.await.unwrap().unwrap();
        assert_eq!(receiver.recv().await.unwrap().message_id, 3204);
    }

    // 字节背压：总预算耗尽时后续入队等待，前一帧释放许可后才继续。
    #[tokio::test]
    async fn message_queue_byte_budget_blocks_until_queued_bytes_are_released() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
        let budget = Arc::new(tokio::sync::Semaphore::new(5));
        let cancellation = tokio_util::sync::CancellationToken::new();
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 3203,
                content: vec![0; 5],
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap();

        let (enqueue_started, enqueue_is_running) = tokio::sync::oneshot::channel();
        let mut pending = tokio::spawn({
            let sender = sender.clone();
            let cancellation = cancellation.clone();
            let budget = budget.clone();
            async move {
                let _ = enqueue_started.send(());
                enqueue_incoming_frame(
                    &sender,
                    IncomingFrame {
                        message_id: 3204,
                        content: vec![0],
                        queue_byte_permit: None,
                    },
                    &cancellation,
                    &budget,
                )
                .await
            }
        });
        enqueue_is_running.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut pending)
                .await
                .is_err(),
            "second frame must remain blocked while all byte permits are held"
        );

        drop(receiver.recv().await.unwrap());
        pending.await.unwrap().unwrap();
        drop(receiver.recv().await.unwrap());
        assert_eq!(budget.available_permits(), 5);
    }

    // 字节许可取消：等待预算期间取消必须返回错误，且已取得的许可仍可在帧丢弃后全量归还。
    #[tokio::test]
    async fn cancelling_byte_budget_wait_releases_every_permit() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
        let budget = Arc::new(tokio::sync::Semaphore::new(2));
        let cancellation = tokio_util::sync::CancellationToken::new();
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 3203,
                content: vec![0; 2],
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap();
        let (wait_started, wait_is_running) = tokio::sync::oneshot::channel();
        let mut waiting = tokio::spawn({
            let sender = sender.clone();
            let cancellation = cancellation.clone();
            let budget = budget.clone();
            async move {
                let _ = wait_started.send(());
                enqueue_incoming_frame(
                    &sender,
                    IncomingFrame {
                        message_id: 3204,
                        content: vec![0],
                        queue_byte_permit: None,
                    },
                    &cancellation,
                    &budget,
                )
                .await
            }
        });
        wait_is_running.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err(),
            "enqueue must still be waiting for byte permits before cancellation"
        );

        cancellation.cancel();
        assert!(waiting.await.unwrap().is_err());
        drop(receiver.recv().await.unwrap());
        assert_eq!(budget.available_permits(), 2);
    }

    // 发送失败：取得字节许可后若 receiver 已关闭，错误路径必须归还全部许可。
    #[tokio::test]
    async fn closed_message_queue_releases_reserved_bytes() {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let budget = Arc::new(tokio::sync::Semaphore::new(2));
        let cancellation = tokio_util::sync::CancellationToken::new();
        drop(receiver);

        let result = enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 3203,
                content: vec![0; 2],
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(budget.available_permits(), 2);
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

    struct FakeMessageEffects {
        monitored: HashSet<i64>,
        persisted: tokio::sync::Mutex<Vec<i64>>,
        persisted_batches: tokio::sync::Mutex<Vec<Vec<i64>>>,
        published: tokio::sync::Mutex<Vec<Vec<i64>>>,
        acknowledged: tokio::sync::Mutex<Vec<(i64, Vec<i64>)>>,
        persist_calls: AtomicUsize,
        snapshot_calls: AtomicUsize,
        persist_succeeds: AtomicBool,
    }

    impl Default for FakeMessageEffects {
        fn default() -> Self {
            Self {
                monitored: HashSet::new(),
                persisted: tokio::sync::Mutex::new(Vec::new()),
                persisted_batches: tokio::sync::Mutex::new(Vec::new()),
                published: tokio::sync::Mutex::new(Vec::new()),
                acknowledged: tokio::sync::Mutex::new(Vec::new()),
                persist_calls: AtomicUsize::new(0),
                snapshot_calls: AtomicUsize::new(0),
                persist_succeeds: AtomicBool::new(true),
            }
        }
    }

    #[async_trait::async_trait]
    impl MessageEffects for FakeMessageEffects {
        async fn monitoring_snapshot(&self) -> HashSet<i64> {
            self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
            self.monitored.clone()
        }

        async fn persist_monitored_batch(&self, messages: &[im_proto::GroupMessage]) -> bool {
            self.persist_calls.fetch_add(1, Ordering::SeqCst);
            if !self.persist_succeeds.load(Ordering::SeqCst) {
                return false;
            }
            self.persisted_batches
                .lock()
                .await
                .push(messages.iter().map(|message| message.msg_id).collect());
            self.persisted
                .lock()
                .await
                .extend(messages.iter().map(|message| message.msg_id));
            true
        }

        async fn publish_monitored_batch(&self, messages: Vec<im_proto::GroupMessage>) {
            self.published
                .lock()
                .await
                .push(messages.into_iter().map(|message| message.msg_id).collect());
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
        async fn monitoring_snapshot(&self) -> HashSet<i64> {
            self.monitored.read().await.clone()
        }

        async fn persist_monitored_batch(&self, messages: &[im_proto::GroupMessage]) -> bool {
            self.persisted
                .lock()
                .await
                .extend(messages.iter().map(|message| message.msg_id));
            true
        }

        async fn publish_monitored_batch(&self, _messages: Vec<im_proto::GroupMessage>) {}

        async fn acknowledge_group_messages(
            &self,
            group_id: i64,
            msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            self.acknowledged.lock().await.push((group_id, msg_ids));
            Ok(())
        }
    }

    struct BlockingPersistEffects {
        persist_started: tokio::sync::Notify,
        acknowledged: AtomicUsize,
        published: AtomicUsize,
    }

    struct BlockingProjectionEffects {
        snapshot_calls: AtomicUsize,
        persist_calls: AtomicUsize,
        receipt_calls: AtomicUsize,
        projection_calls: AtomicUsize,
        projection_started: tokio::sync::Notify,
        projection_release: tokio::sync::Semaphore,
    }

    struct ReceiptFailureEffects {
        persisted: AtomicUsize,
        acknowledged_groups: tokio::sync::Mutex<Vec<i64>>,
        projected: AtomicUsize,
    }

    struct ProjectionCancellationEffects {
        calls: AtomicUsize,
        started: tokio::sync::Notify,
        future_cancelled: AtomicBool,
    }

    struct ProjectionCancellationGuard<'a>(&'a AtomicBool);

    impl Drop for ProjectionCancellationGuard<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl MessageEffects for ProjectionCancellationEffects {
        async fn monitoring_snapshot(&self) -> HashSet<i64> {
            HashSet::new()
        }

        async fn persist_monitored_batch(&self, _messages: &[im_proto::GroupMessage]) -> bool {
            true
        }

        async fn publish_monitored_batch(&self, _messages: Vec<im_proto::GroupMessage>) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let _guard = ProjectionCancellationGuard(&self.future_cancelled);
            self.started.notify_one();
            std::future::pending().await
        }

        async fn acknowledge_group_messages(
            &self,
            _group_id: i64,
            _msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl MessageEffects for ReceiptFailureEffects {
        async fn monitoring_snapshot(&self) -> HashSet<i64> {
            [7, 8, 9].into_iter().collect()
        }

        async fn persist_monitored_batch(&self, _messages: &[im_proto::GroupMessage]) -> bool {
            self.persisted.fetch_add(1, Ordering::SeqCst);
            true
        }

        async fn publish_monitored_batch(&self, _messages: Vec<im_proto::GroupMessage>) {
            self.projected.fetch_add(1, Ordering::SeqCst);
        }

        async fn acknowledge_group_messages(
            &self,
            group_id: i64,
            _msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            self.acknowledged_groups.lock().await.push(group_id);
            if group_id == 8 {
                return Err(im_common::error::AppError::TcpFrame(
                    "injected receipt failure".to_string(),
                ));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl MessageEffects for BlockingProjectionEffects {
        async fn monitoring_snapshot(&self) -> HashSet<i64> {
            self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
            [7].into_iter().collect()
        }

        async fn persist_monitored_batch(&self, _messages: &[im_proto::GroupMessage]) -> bool {
            self.persist_calls.fetch_add(1, Ordering::SeqCst);
            true
        }

        async fn publish_monitored_batch(&self, _messages: Vec<im_proto::GroupMessage>) {
            self.projection_calls.fetch_add(1, Ordering::SeqCst);
            self.projection_started.notify_one();
            self.projection_release.acquire().await.unwrap().forget();
        }

        async fn acknowledge_group_messages(
            &self,
            _group_id: i64,
            _msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            self.receipt_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl MessageEffects for BlockingPersistEffects {
        async fn monitoring_snapshot(&self) -> HashSet<i64> {
            [7].into_iter().collect()
        }

        async fn persist_monitored_batch(&self, _messages: &[im_proto::GroupMessage]) -> bool {
            self.persist_started.notify_one();
            std::future::pending().await
        }

        async fn publish_monitored_batch(&self, _messages: Vec<im_proto::GroupMessage>) {
            self.published.fetch_add(1, Ordering::SeqCst);
        }

        async fn acknowledge_group_messages(
            &self,
            _group_id: i64,
            _msg_ids: Vec<i64>,
        ) -> Result<(), im_common::error::AppError> {
            self.acknowledged.fetch_add(1, Ordering::SeqCst);
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
                queue_byte_permit: None,
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
            ..Default::default()
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
                queue_byte_permit: None,
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
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        sender
            .send(IncomingFrame {
                message_id: 2205,
                content: b"reserved recall payload".to_vec(),
                queue_byte_permit: None,
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

    // 微批上限：同一帧中的 100 条监控消息应一次提交并立即 flush。
    #[tokio::test]
    async fn message_worker_flushes_one_hundred_messages_in_one_batch() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        let push = im_proto::PushGroupMessage {
            group_msg: (1..=100)
                .map(|msg_id| im_proto::GroupMessage {
                    msg_id,
                    group_id: 7,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: push.encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_millis(200), async {
            while effects.persist_calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("100 messages must flush while the input queue remains open");

        assert_eq!(effects.persist_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.persisted.lock().await.len(), 100);
        cancellation.cancel();
        drop(sender);
        worker.await.unwrap();
    }

    // 可重复负载：单帧 10,000 条消息必须按阈值形成恰好 100 个事务批次，并保持
    // 持久化、投影与按群回执的 ID 集合及协议顺序完全一致。只输出观测值，不设耗时门槛。
    #[tokio::test]
    async fn message_worker_preserves_ten_thousand_message_burst_without_loss_or_duplicates() {
        const MESSAGE_COUNT: usize = 10_000;
        const EXPECTED_BATCH_COUNT: usize = 100;
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7, 8].into_iter().collect(),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let messages = (0..MESSAGE_COUNT)
            .map(|index| im_proto::GroupMessage {
                msg_id: index as i64 + 1,
                group_id: if index % 2 == 0 { 7 } else { 8 },
                send_time: index as i64,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let expected_ids = messages
            .iter()
            .map(|message| message.msg_id)
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();

        sender
            .send(IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
                content: im_proto::PushGroupMessage {
                    group_msg: messages,
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        drop(sender);
        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        let persisted_batches = effects.persisted_batches.lock().await;
        let published_batches = effects.published.lock().await;
        let receipts = effects.acknowledged.lock().await;
        let persisted_ids = persisted_batches
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let published_ids = published_batches
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let receipt_ids = receipts
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect::<Vec<_>>();
        let peak_batch = persisted_batches.iter().map(Vec::len).max().unwrap_or(0);

        assert_eq!(persisted_batches.len(), EXPECTED_BATCH_COUNT);
        assert_eq!(peak_batch, 100);
        assert!(persisted_batches
            .iter()
            .all(|batch| batch.len() <= MESSAGE_BATCH_MAX_MESSAGES));
        assert_eq!(published_batches.len(), persisted_batches.len());
        assert_eq!(persisted_ids, expected_ids);
        assert_eq!(published_ids, expected_ids);
        assert_eq!(
            persisted_ids.iter().copied().collect::<HashSet<_>>().len(),
            MESSAGE_COUNT
        );
        assert_eq!(
            published_ids.iter().copied().collect::<HashSet<_>>().len(),
            MESSAGE_COUNT
        );
        assert_eq!(
            receipt_ids.iter().copied().collect::<HashSet<_>>(),
            expected_ids.iter().copied().collect::<HashSet<_>>()
        );
        assert_eq!(receipt_ids.len(), MESSAGE_COUNT);
        assert_eq!(receipts.len(), persisted_batches.len() * 2);
        for (batch_index, receipts_for_batch) in receipts.chunks_exact(2).enumerate() {
            assert_eq!(receipts_for_batch[0].0, 7);
            assert_eq!(receipts_for_batch[1].0, 8);
            let mut merged = receipts_for_batch
                .iter()
                .flat_map(|(_, ids)| ids.iter().copied())
                .collect::<Vec<_>>();
            merged.sort_unstable();
            assert_eq!(merged, persisted_batches[batch_index]);
        }

        eprintln!(
            "10k message worker load: elapsed={:?}, peak_batch={}, observed_projection_queue_capacity={}",
            started.elapsed(),
            peak_batch,
            MESSAGE_PROJECTION_QUEUE_CAPACITY
        );
    }

    // 时间上限：不足 100 条时从首条进入空批次起等待 25ms 后提交。
    #[tokio::test(start_paused = true)]
    async fn message_worker_flushes_partial_batch_after_twenty_five_milliseconds() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: im_proto::PushGroupMessage {
                    group_msg: vec![im_proto::GroupMessage {
                        msg_id: 70,
                        group_id: 7,
                        ..Default::default()
                    }],
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_millis(24)).await;
        tokio::task::yield_now().await;
        assert_eq!(effects.persist_calls.load(Ordering::SeqCst), 0);
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(effects.persist_calls.load(Ordering::SeqCst), 1);

        cancellation.cancel();
        drop(sender);
        worker.await.unwrap();
    }

    // 协议优先级：待 flush 的 2202 不阻塞随后到达的 1201 登录确认。
    #[tokio::test(start_paused = true)]
    async fn login_success_is_processed_while_partial_message_batch_waits() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let (login_sender, mut login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: im_proto::PushGroupMessage {
                    group_msg: vec![im_proto::GroupMessage {
                        msg_id: 70,
                        group_id: 7,
                        ..Default::default()
                    }],
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        tokio::task::yield_now().await;
        sender
            .send(IncomingFrame {
                message_id: 1201,
                content: im_proto::PushLoginSuccessMessage::default().encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        tokio::task::yield_now().await;

        assert!(login_receiver.try_recv().is_ok());
        assert_eq!(effects.persist_calls.load(Ordering::SeqCst), 0);

        cancellation.cancel();
        drop(sender);
        worker.await.unwrap();
    }

    // 帧级快照：同一 PushGroupMessage 的多条消息只读取一次监控集合。
    #[tokio::test]
    async fn message_worker_reads_monitoring_snapshot_once_per_push_frame() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: im_proto::PushGroupMessage {
                    group_msg: vec![
                        im_proto::GroupMessage {
                            msg_id: 70,
                            group_id: 7,
                            ..Default::default()
                        },
                        im_proto::GroupMessage {
                            msg_id: 71,
                            group_id: 7,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        drop(sender);

        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        assert_eq!(effects.snapshot_calls.load(Ordering::SeqCst), 1);
    }

    // 事务失败：监控消息不回执、不推前端，未监控消息仍按群回执。
    #[tokio::test]
    async fn failed_monitored_batch_only_acknowledges_unmonitored_messages() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            persist_succeeds: AtomicBool::new(false),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: im_proto::PushGroupMessage {
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
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        drop(sender);

        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        assert!(effects.persisted.lock().await.is_empty());
        assert_eq!(*effects.acknowledged.lock().await, [(8, vec![80])]);
        assert!(effects.published.lock().await.is_empty());
    }

    // Channel 批次：成功事务和回执后仅发布一次，并保持监控消息的协议顺序。
    #[tokio::test]
    async fn successful_batch_publishes_once_in_original_order() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            ..Default::default()
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: im_proto::PushGroupMessage {
                    group_msg: [3, 1, 2]
                        .into_iter()
                        .map(|msg_id| im_proto::GroupMessage {
                            msg_id,
                            group_id: 7,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        drop(sender);

        run_message_worker_with_effects(receiver, effects.clone(), cancellation, login_sender)
            .await;

        assert_eq!(*effects.published.lock().await, [vec![3, 1, 2]]);
        assert_eq!(*effects.acknowledged.lock().await, [(7, vec![3, 1, 2])]);
    }

    // 取消边界：监控批次事务尚未完成时退出，不发送回执或 Channel。
    #[tokio::test]
    async fn cancellation_during_batch_insert_does_not_acknowledge_uncommitted_messages() {
        let effects = Arc::new(BlockingPersistEffects {
            persist_started: tokio::sync::Notify::new(),
            acknowledged: AtomicUsize::new(0),
            published: AtomicUsize::new(0),
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        sender
            .send(IncomingFrame {
                message_id: 2202,
                content: im_proto::PushGroupMessage {
                    group_msg: (1..=100)
                        .map(|msg_id| im_proto::GroupMessage {
                            msg_id,
                            group_id: 7,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        effects.persist_started.notified().await;

        cancellation.cancel();
        worker.await.unwrap();

        assert_eq!(effects.acknowledged.load(Ordering::SeqCst), 0);
        assert_eq!(effects.published.load(Ordering::SeqCst), 0);
    }

    // 投影隔离：阻塞的首批投影不妨碍主 worker 继续提交和回执，直到容量 8 的投影队列填满。
    #[tokio::test]
    async fn blocked_projection_allows_main_worker_to_fill_bounded_projection_queue() {
        let effects = Arc::new(BlockingProjectionEffects {
            snapshot_calls: AtomicUsize::new(0),
            persist_calls: AtomicUsize::new(0),
            receipt_calls: AtomicUsize::new(0),
            projection_calls: AtomicUsize::new(0),
            projection_started: tokio::sync::Notify::new(),
            projection_release: tokio::sync::Semaphore::new(0),
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        for batch_id in 0..(MESSAGE_PROJECTION_QUEUE_CAPACITY + 3) {
            sender
                .send(full_monitored_batch_frame(batch_id as i64))
                .await
                .unwrap();
        }
        effects.projection_started.notified().await;

        let progressed = tokio::time::timeout(Duration::from_millis(200), async {
            while effects.persist_calls.load(Ordering::SeqCst)
                < MESSAGE_PROJECTION_QUEUE_CAPACITY + 2
            {
                tokio::task::yield_now().await;
            }
        })
        .await;

        cancellation.cancel();
        drop(sender);
        worker.await.unwrap();
        assert!(
            progressed.is_ok(),
            "blocked projection must not stop database and receipt processing before the queue fills"
        );
        let processed_before_backpressure = MESSAGE_PROJECTION_QUEUE_CAPACITY + 2;
        assert_eq!(
            effects.snapshot_calls.load(Ordering::SeqCst),
            processed_before_backpressure
        );
        assert_eq!(
            effects.persist_calls.load(Ordering::SeqCst),
            processed_before_backpressure
        );
        assert_eq!(
            effects.receipt_calls.load(Ordering::SeqCst),
            processed_before_backpressure
        );
        assert_eq!(effects.projection_calls.load(Ordering::SeqCst), 1);
    }

    // 端到端字节预算：已完成事务和回执但仍阻塞投影的帧必须继续占用其正文许可。
    #[tokio::test]
    async fn blocked_projection_keeps_frame_bytes_reserved_until_projection_finishes() {
        let effects = Arc::new(BlockingProjectionEffects {
            snapshot_calls: AtomicUsize::new(0),
            persist_calls: AtomicUsize::new(0),
            receipt_calls: AtomicUsize::new(0),
            projection_calls: AtomicUsize::new(0),
            projection_started: tokio::sync::Notify::new(),
            projection_release: tokio::sync::Semaphore::new(0),
        });
        let payload = im_proto::PushGroupMessage {
            group_msg: vec![im_proto::GroupMessage {
                msg_id: 70,
                group_id: 7,
                content: vec![1, 2, 3],
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec();
        let second_frame_bytes = 2;
        let total_budget = payload.len() + second_frame_bytes - 1;
        let budget = Arc::new(tokio::sync::Semaphore::new(total_budget));
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
                content: payload,
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap();
        effects.projection_started.notified().await;
        assert_eq!(effects.persist_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.receipt_calls.load(Ordering::SeqCst), 1);

        let (enqueue_entered, enqueue_is_running) = tokio::sync::oneshot::channel();
        let mut second_enqueue = tokio::spawn({
            let sender = sender.clone();
            let cancellation = cancellation.clone();
            let budget = budget.clone();
            async move {
                let _ = enqueue_entered.send(());
                enqueue_incoming_frame(
                    &sender,
                    IncomingFrame {
                        message_id: 9999,
                        content: vec![0; second_frame_bytes],
                        queue_byte_permit: None,
                    },
                    &cancellation,
                    &budget,
                )
                .await
            }
        });
        enqueue_is_running.await.unwrap();
        let blocked = tokio::time::timeout(Duration::from_millis(20), &mut second_enqueue).await;
        let was_blocked = blocked.is_err();

        effects.projection_release.add_permits(1);
        if was_blocked {
            second_enqueue.await.unwrap().unwrap();
        } else {
            blocked.unwrap().unwrap().unwrap();
        }
        tokio::time::timeout(Duration::from_millis(200), async {
            while budget.available_permits() != total_budget {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("projection completion and later frame consumption must release all bytes");
        cancellation.cancel();
        drop(sender);
        worker.await.unwrap();

        assert!(
            was_blocked,
            "the second enqueue must wait while projection retains the first frame permit"
        );
    }

    // 投影取消：正在运行的投影 future 被 drop，排队批次不再发布，二者许可均被归还。
    #[tokio::test]
    async fn cancelling_projection_worker_drops_active_and_queued_batches() {
        let effects = Arc::new(ProjectionCancellationEffects {
            calls: AtomicUsize::new(0),
            started: tokio::sync::Notify::new(),
            future_cancelled: AtomicBool::new(false),
        });
        let byte_budget = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = Arc::new(byte_budget.clone().acquire_owned().await.unwrap());
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        for msg_id in [70, 71] {
            sender
                .send(super::ProjectionMessageBatch {
                    messages: vec![im_proto::GroupMessage {
                        msg_id,
                        group_id: 7,
                        ..Default::default()
                    }],
                    frame_byte_permits: vec![permit.clone()],
                })
                .await
                .unwrap();
        }
        drop(permit);
        drop(sender);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let projection_worker = tokio::spawn(super::run_message_projection_worker(
            receiver,
            effects.clone(),
            cancellation.clone(),
        ));
        effects.started.notified().await;

        cancellation.cancel();
        projection_worker.await.unwrap();

        assert!(effects.future_cancelled.load(Ordering::SeqCst));
        assert_eq!(effects.calls.load(Ordering::SeqCst), 1);
        assert_eq!(byte_budget.available_permits(), 1);
    }

    // 跨批共享：同一帧拆成两个投影批次时，首批完成不能提前归还该帧许可。
    #[tokio::test]
    async fn frame_permit_is_released_after_its_last_projection_batch_finishes() {
        let effects = Arc::new(BlockingProjectionEffects {
            snapshot_calls: AtomicUsize::new(0),
            persist_calls: AtomicUsize::new(0),
            receipt_calls: AtomicUsize::new(0),
            projection_calls: AtomicUsize::new(0),
            projection_started: tokio::sync::Notify::new(),
            projection_release: tokio::sync::Semaphore::new(0),
        });
        let payload = im_proto::PushGroupMessage {
            group_msg: (0..150)
                .map(|msg_id| im_proto::GroupMessage {
                    msg_id,
                    group_id: 7,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
        .encode_to_vec();
        let total_bytes = payload.len();
        let byte_budget = Arc::new(tokio::sync::Semaphore::new(total_bytes));
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        ));
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
                content: payload,
                queue_byte_permit: None,
            },
            &cancellation,
            &byte_budget,
        )
        .await
        .unwrap();
        drop(sender);
        effects.projection_started.notified().await;

        effects.projection_release.add_permits(1);
        while effects.projection_calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        assert_eq!(byte_budget.available_permits(), 0);

        effects.projection_release.add_permits(1);
        worker.await.unwrap();
        assert_eq!(byte_budget.available_permits(), total_bytes);
    }

    // 回执失败：按 group_id 顺序发送时中途失败会取消连接，不再回执后续群，也不提交投影。
    #[tokio::test]
    async fn receipt_failure_cancels_connection_and_skips_remaining_receipts_and_projection() {
        let effects = Arc::new(ReceiptFailureEffects {
            persisted: AtomicUsize::new(0),
            acknowledged_groups: tokio::sync::Mutex::new(Vec::new()),
            projected: AtomicUsize::new(0),
        });
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        sender
            .send(IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
                content: im_proto::PushGroupMessage {
                    group_msg: [9, 7, 8]
                        .into_iter()
                        .map(|group_id| im_proto::GroupMessage {
                            msg_id: group_id * 10,
                            group_id,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            })
            .await
            .unwrap();
        drop(sender);

        run_message_worker_with_effects(
            receiver,
            effects.clone(),
            cancellation.clone(),
            login_sender,
        )
        .await;

        assert_eq!(effects.persisted.load(Ordering::SeqCst), 1);
        assert_eq!(*effects.acknowledged_groups.lock().await, [7, 8]);
        assert_eq!(effects.projected.load(Ordering::SeqCst), 0);
        assert!(cancellation.is_cancelled());
    }

    // 无派生消息的帧：真实字节许可在 worker 完成本帧处理后归还。
    #[tokio::test]
    async fn message_worker_releases_real_byte_permit_after_frame_handling() {
        let effects = Arc::new(FakeMessageEffects::default());
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let budget = Arc::new(tokio::sync::Semaphore::new(2));
        let cancellation = tokio_util::sync::CancellationToken::new();
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 9999,
                content: vec![0; 2],
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap();
        assert_eq!(budget.available_permits(), 0);
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(run_message_worker_with_effects(
            receiver,
            effects,
            cancellation.clone(),
            login_sender,
        ));

        tokio::time::timeout(Duration::from_millis(200), async {
            while budget.available_permits() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker must release bytes after receiving the frame");

        cancellation.cancel();
        drop(sender);
        worker.await.unwrap();
    }

    // worker 取消：尚在 mpsc 中的真实许可随 receiver 关闭和排队帧 drop 一并归还。
    #[tokio::test]
    async fn cancelled_message_worker_drops_queued_real_byte_permit() {
        let effects = Arc::new(FakeMessageEffects::default());
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let budget = Arc::new(tokio::sync::Semaphore::new(2));
        let cancellation = tokio_util::sync::CancellationToken::new();
        enqueue_incoming_frame(
            &sender,
            IncomingFrame {
                message_id: 9999,
                content: vec![0; 2],
                queue_byte_permit: None,
            },
            &cancellation,
            &budget,
        )
        .await
        .unwrap();
        let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();
        cancellation.cancel();

        run_message_worker_with_effects(receiver, effects, cancellation, login_sender).await;

        assert_eq!(budget.available_permits(), 2);
    }

    // 非投影帧：1201、2205 与 2202 解码失败均在本帧处理结束后及时归还真实许可。
    #[tokio::test]
    async fn non_projected_frames_release_real_byte_permits_after_handling() {
        let frames = [
            IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_LOGIN_SUCCESS,
                content: im_proto::PushLoginSuccessMessage {
                    login_time: 1,
                    ..Default::default()
                }
                .encode_to_vec(),
                queue_byte_permit: None,
            },
            IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_RECALL_GROUP_MESSAGE,
                content: vec![1],
                queue_byte_permit: None,
            },
            IncomingFrame {
                message_id: im_chat::heartbeat::PUSH_GROUP_MESSAGE,
                content: vec![0xff],
                queue_byte_permit: None,
            },
        ];

        for frame in frames {
            let total_bytes = frame.content.len();
            let budget = Arc::new(tokio::sync::Semaphore::new(total_bytes));
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            let cancellation = tokio_util::sync::CancellationToken::new();
            enqueue_incoming_frame(&sender, frame, &cancellation, &budget)
                .await
                .unwrap();
            drop(sender);
            let (login_sender, _login_receiver) = tokio::sync::oneshot::channel();

            run_message_worker_with_effects(
                receiver,
                Arc::new(FakeMessageEffects::default()),
                cancellation,
                login_sender,
            )
            .await;

            assert_eq!(budget.available_permits(), total_bytes);
        }
    }

    // 有界并发映射：同时运行数不超过 8，结果仍按输入位置排列。
    #[tokio::test]
    async fn ordered_bounded_map_limits_concurrency_and_preserves_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let output = super::map_ordered_bounded((0..24).collect(), MESSAGE_DECRYPT_CONCURRENCY, {
            let active = active.clone();
            let maximum = maximum.clone();
            move |value| {
                let active = active.clone();
                let maximum = maximum.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    value
                }
            }
        })
        .await;

        assert_eq!(maximum.load(Ordering::SeqCst), MESSAGE_DECRYPT_CONCURRENCY);
        assert_eq!(output, (0..24).collect::<Vec<_>>());
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
                queue_byte_permit: None,
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
                    queue_byte_permit: None,
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

    // 取消先于收帧生效：此场景下排队的 2202 被丢弃，不产生持久化副作用。
    #[tokio::test]
    async fn cancelled_message_worker_discards_queued_frames() {
        let effects = Arc::new(FakeMessageEffects {
            monitored: [7].into_iter().collect(),
            ..Default::default()
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
                queue_byte_permit: None,
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
        let (wait_started, wait_is_running) = tokio::sync::oneshot::channel();
        let mut worker = tokio::spawn(async move {
            let _ = wait_started.send(());
            cancellation.cancelled().await;
            worker_coordinator
                .finish_connect(generation, attempt_id)
                .await;
        });
        wait_is_running.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut worker)
                .await
                .is_err(),
            "connection worker must still be waiting before explicit cancellation"
        );

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
            matched: 0,
            group_name: "测试群".to_string(),
            content_text: record.content_text.clone(),
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

    #[test]
    fn attachment_cache_filename_removes_path_components() {
        assert_eq!(
            super::safe_attachment_filename(42, false, "../../报告 final.pdf"),
            "42-main-报告_final.pdf"
        );
        assert_eq!(
            super::safe_attachment_filename(42, true, ""),
            "42-thumbnail-attachment"
        );
    }
}
