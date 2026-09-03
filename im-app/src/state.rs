//! 应用共享状态与聊天连接协调器。
//!
//! 本模块集中保存认证会话、已安装客户端和连接阶段，并用代际与尝试编号拒绝过期异步结果。
//! 这些锁只保护各自注明的内存对象；跨锁流程依靠固定持锁顺序和当前性复核，而非事务原子性。

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use im_chat::ChatClient;
use im_common::config::AppConfig;
use im_http::http_clients::AppHttpClients;
use im_store::SqliteStore;
use tauri::AppHandle;
use tokio::sync::{watch, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
/// 当前认证凭据及其所属连接代际。
pub struct AuthSession {
    /// 后端用户标识。
    pub uid: i64,
    /// 调用 HTTP 与聊天服务所需的认证令牌。
    pub token: String,
    /// 会话当前绑定的连接代际；显式断开后可由协调器重新标记。
    pub generation: u64,
}

/// 串行协调连接状态迁移和状态发布。
///
/// `generation` 区分登录、登出或显式取消前后的生命周期，`attempt_id` 区分同一代际内的
/// 多次连接尝试。二者共同标识一次尝试，但不会为外部网络操作提供事务原子性或公平性保证。
pub struct ConnectionCoordinator {
    /// 保护连接状态机、当前尝试和本代取消令牌。
    state: Mutex<ConnectionState>,
    /// 串行执行状态复核及其发布闭包，防止并发状态通知相互穿插。
    status_publication: Mutex<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 协调器记录的聊天连接阶段。
pub enum ConnectionPhase {
    /// 协调器逻辑上不再认可任何进行中的连接尝试。
    ///
    /// 取消流程可先进入此阶段；旧异步任务、客户端或底层物理连接可能短暂仍存在，随后由任务
    /// 自行观察取消或由调用方继续清理。
    Idle,
    /// 首次连接尝试正在执行。
    Connecting,
    /// 当前尝试已安装可用客户端。
    Connected,
    /// 已连接客户端自然掉线后正在重连。
    Reconnecting,
}

/// `ConnectionCoordinator::state` 互斥锁保护的可变状态。
struct ConnectionState {
    /// 当前认证/连接生命周期代际；显式取消或认证切换时递增。
    generation: u64,
    /// 最近分配的尝试编号，在同一代际重试时仍单调递增。
    next_attempt_id: u64,
    /// 当前或最近一次阶段所有者；部分进入 `Idle` 的路径会保留该编号供后续状态校验。
    current_attempt_id: Option<u64>,
    /// 当前连接阶段。
    phase: ConnectionPhase,
    /// 当前代际共享的取消令牌；取消后会替换为新令牌。
    cancellation: CancellationToken,
    /// 尚需报告结束的首次连接操作。
    in_flight: Option<InFlightConnection>,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            generation: 0,
            next_attempt_id: 0,
            current_attempt_id: None,
            phase: ConnectionPhase::Idle,
            cancellation: CancellationToken::new(),
            in_flight: None,
        }
    }
}

/// 正在执行的首次连接及其完成通知。
struct InFlightConnection {
    /// 操作开始时的代际。
    generation: u64,
    /// 操作开始时分配的尝试编号。
    attempt_id: u64,
    /// 供取消流程协作式通知该操作停止的令牌；任务须自行观察通知后退出。
    cancellation: CancellationToken,
    /// 连接任务结束时置为 `true`，供取消流程限时等待。
    finished: watch::Sender<bool>,
}

/// `begin_connect` 发放给单次连接任务的只读凭据。
///
/// 凭据存活到该连接任务结束；后续安装或收尾必须同时匹配其中的代际与尝试编号。
pub struct ConnectionPermit {
    /// 连接开始时的代际。
    generation: u64,
    /// 本次连接的尝试编号。
    attempt_id: u64,
    /// 与本代连接生命周期关联的取消令牌。
    cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 已安装客户端的所有者键，由代际与尝试编号共同组成。
pub struct ConnectionAttemptKey {
    /// 所属连接代际。
    generation: u64,
    /// 所属连接尝试编号。
    attempt_id: u64,
}

impl ConnectionAttemptKey {
    /// 根据代际与尝试编号构造所有者键。
    pub fn new(generation: u64, attempt_id: u64) -> Self {
        Self {
            generation,
            attempt_id,
        }
    }
}

#[derive(Debug)]
/// 连同所有者键保存的聊天客户端。
pub struct InstalledClient {
    /// 标识安装该客户端的连接尝试。
    key: ConnectionAttemptKey,
    /// 供聊天命令与后台任务使用的客户端。
    pub client: ChatClient,
}

impl InstalledClient {
    /// 将客户端归属到指定连接尝试。
    pub fn new(key: ConnectionAttemptKey, client: ChatClient) -> Self {
        Self { key, client }
    }

    /// 返回安装该客户端的连接尝试键。
    pub fn key(&self) -> ConnectionAttemptKey {
        self.key
    }
}

/// 保护可选已安装聊天客户端的互斥槽位。
///
/// 槽位从连接成功安装持续到调用方的断开流程取走客户端；锁只保护槽位内容，不表示底层物理
/// 连接已同步关闭，也不保护客户端内部状态。
pub type ClientSlot = tokio::sync::Mutex<Option<InstalledClient>>;

impl ConnectionPermit {
    /// 返回该许可所属代际。
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 返回该许可所属尝试编号。
    pub fn attempt_id(&self) -> u64 {
        self.attempt_id
    }

    #[cfg(test)]
    /// 在测试中取得与许可对应的客户端所有者键。
    pub fn key(&self) -> ConnectionAttemptKey {
        ConnectionAttemptKey::new(self.generation, self.attempt_id)
    }

    /// 克隆供连接任务监听的取消令牌。
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl ConnectionCoordinator {
    /// 创建处于空闲第 0 代的连接协调器。
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ConnectionState::default()),
            status_publication: Mutex::new(()),
        }
    }

    /// 在指定代际开始一次首次连接并返回许可。
    ///
    /// 仅当前代际且阶段为 `Idle` 时成功；其他阶段返回重复连接或进行中错误，编号溢出也返回
    /// 错误。成功后登记 in-flight 操作并进入 `Connecting`，尚未执行网络连接或安装客户端。
    pub async fn begin_connect(&self, generation: u64) -> Result<ConnectionPermit, String> {
        let mut state = self.state.lock().await;
        if generation != state.generation {
            return Err("Authentication generation is stale".to_string());
        }
        match state.phase {
            ConnectionPhase::Idle => {}
            ConnectionPhase::Connecting => {
                return Err("A connection operation is already in progress".to_string());
            }
            ConnectionPhase::Connected => return Err("AlreadyConnected".to_string()),
            ConnectionPhase::Reconnecting => return Err("Connecting".to_string()),
        }
        state.next_attempt_id = state
            .next_attempt_id
            .checked_add(1)
            .ok_or_else(|| "Connection attempt id overflow".to_string())?;
        let attempt_id = state.next_attempt_id;
        let cancellation = state.cancellation.clone();
        let (finished, _) = watch::channel(false);
        state.in_flight = Some(InFlightConnection {
            generation,
            attempt_id,
            cancellation: cancellation.clone(),
            finished,
        });
        state.phase = ConnectionPhase::Connecting;
        state.current_attempt_id = Some(attempt_id);
        Ok(ConnectionPermit {
            generation,
            attempt_id,
            cancellation,
        })
    }

    /// 报告指定首次连接任务已结束。
    ///
    /// 只有 in-flight 的代际和尝试编号都匹配时才发送完成通知；若该尝试仍拥有
    /// `Connecting` 阶段，还会回到逻辑空闲、发出协作式取消通知并轮换令牌。过期调用不产生
    /// 副作用；此阶段变化本身不证明底层物理连接已经关闭。
    pub async fn finish_connect(&self, generation: u64, attempt_id: u64) {
        let mut state = self.state.lock().await;
        if state.in_flight.as_ref().is_some_and(|operation| {
            operation.generation == generation && operation.attempt_id == attempt_id
        }) {
            if let Some(operation) = state.in_flight.take() {
                let _ = operation.finished.send(true);
            }
            if state.phase == ConnectionPhase::Connecting
                && state.current_attempt_id == Some(attempt_id)
            {
                state.phase = ConnectionPhase::Idle;
                state.current_attempt_id = None;
                state.cancellation.cancel();
                state.cancellation = CancellationToken::new();
            }
        }
    }

    /// 仅在指定尝试仍是当前首次连接时记录失败。
    ///
    /// 匹配时通知等待者、清除 in-flight 与当前尝试、回到逻辑空闲并发出协作式取消通知后
    /// 轮换令牌；过期失败结果被忽略。此方法不清除认证会话、修改客户端槽位或同步关闭物理
    /// 连接。
    pub async fn fail_connect_if_current(&self, generation: u64, attempt_id: u64) {
        let mut state = self.state.lock().await;
        let is_current = state.generation == generation
            && state.phase == ConnectionPhase::Connecting
            && state.current_attempt_id == Some(attempt_id)
            && state.in_flight.as_ref().is_some_and(|operation| {
                operation.generation == generation && operation.attempt_id == attempt_id
            });
        if is_current {
            if let Some(operation) = state.in_flight.take() {
                let _ = operation.finished.send(true);
            }
            state.phase = ConnectionPhase::Idle;
            state.current_attempt_id = None;
            state.current_attempt_id = None;
            state.cancellation.cancel();
            state.cancellation = CancellationToken::new();
        }
    }

    /// 在当前代际仍为已连接状态时串行执行发布闭包。
    ///
    /// 返回值表示闭包是否实际执行。发布锁覆盖当前性检查和整个异步闭包，因此同类状态发布按
    /// 获锁顺序执行，但不承诺调度公平性，也不让闭包中的外部副作用具备事务原子性。
    pub async fn publish_connected_if_current<F, Fut>(&self, generation: u64, publish: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _publication = self.status_publication.lock().await;
        let should_publish = {
            let state = self.state.lock().await;
            state.generation == generation && state.phase == ConnectionPhase::Connected
        };
        if !should_publish {
            return false;
        }
        publish().await;
        true
    }

    /// 在代际和尝试编号仍拥有连接或重连阶段时串行发布“连接中”状态。
    ///
    /// 不匹配时返回 `false` 且不调用闭包；匹配时持有发布锁直至闭包结束。
    pub async fn publish_connecting_if_current<F, Fut>(
        &self,
        generation: u64,
        attempt_id: u64,
        publish: F,
    ) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _publication = self.status_publication.lock().await;
        let should_publish = {
            let state = self.state.lock().await;
            state.generation == generation
                && state.current_attempt_id == Some(attempt_id)
                && matches!(
                    state.phase,
                    ConnectionPhase::Connecting | ConnectionPhase::Reconnecting
                )
        };
        if !should_publish {
            return false;
        }
        publish().await;
        true
    }

    /// 在当前代际尚非已连接状态时串行发布“已断开”状态。
    ///
    /// 此方法只筛选并发布状态，不改变连接阶段或客户端槽位。
    pub async fn publish_disconnected_if_current<F, Fut>(&self, generation: u64, publish: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _publication = self.status_publication.lock().await;
        let should_publish = {
            let state = self.state.lock().await;
            state.generation == generation && state.phase != ConnectionPhase::Connected
        };
        if !should_publish {
            return false;
        }
        publish().await;
        true
    }

    #[cfg(test)]
    /// 测试辅助：若代际仍当前且阶段非空闲，则先切为逻辑空闲、发出协作式取消通知并轮换
    /// 令牌，再串行发布断开；它不等待底层物理连接关闭。
    pub async fn disconnect_and_publish_if_current<F, Fut>(
        &self,
        generation: u64,
        publish: F,
    ) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _publication = self.status_publication.lock().await;
        {
            let mut state = self.state.lock().await;
            if state.generation != generation || state.phase == ConnectionPhase::Idle {
                return false;
            }
            state.phase = ConnectionPhase::Idle;
            state.cancellation.cancel();
            state.cancellation = CancellationToken::new();
        }
        publish().await;
        true
    }

    /// 将当前已连接尝试切换为重连阶段，并串行执行状态发布闭包。
    ///
    /// 代际、尝试编号或阶段不匹配时返回 `false` 且无副作用；成功不会递增代际，也不会取消
    /// 当前代际令牌，使自然掉线后的重连仍可沿用原尝试所有权。
    pub async fn begin_reconnect_and_publish_if_current<F, Fut>(
        &self,
        generation: u64,
        attempt_id: u64,
        publish: F,
    ) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _publication = self.status_publication.lock().await;
        {
            let mut state = self.state.lock().await;
            if state.generation != generation
                || state.current_attempt_id != Some(attempt_id)
                || state.phase != ConnectionPhase::Connected
            {
                return false;
            }
            state.phase = ConnectionPhase::Reconnecting;
        }
        publish().await;
        true
    }

    /// 返回锁内记录的当前连接阶段快照。
    pub async fn phase(&self) -> ConnectionPhase {
        self.state.lock().await.phase
    }

    #[cfg(test)]
    /// 测试辅助：按协作式取消语义推进当前生命周期，并返回递增后的代际。
    pub async fn cancel_and_advance(&self) -> Result<u64, String> {
        Ok(self.cancel_and_advance_with_owner().await?.0)
    }

    /// 取消当前生命周期、递增代际，并返回此前的尝试所有者。
    ///
    /// 方法在发布锁内先递增 `generation`、切为逻辑空闲、通过当前令牌发出协作式取消通知并
    /// 替换新令牌；若存在 in-flight 操作，还会通知其停止并在释放锁后最多等待一秒完成信号，
    /// 随后仅在记录仍匹配时清除它。等待超时不会作为错误返回，任务可能继续运行，直到自行
    /// 观察取消或结束。代际溢出返回错误；该流程分阶段持锁，不保证与外部任务副作用原子提交，
    /// 也不表示旧物理连接已在返回前关闭。
    pub async fn cancel_and_advance_with_owner(
        &self,
    ) -> Result<(u64, Option<ConnectionAttemptKey>), String> {
        let (next_generation, cancelled_owner, cancelled_generation, mut finished) = {
            let _publication = self.status_publication.lock().await;
            let mut state = self.state.lock().await;
            let cancelled_owner = state
                .current_attempt_id
                .map(|attempt_id| ConnectionAttemptKey::new(state.generation, attempt_id));
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or_else(|| "Connection generation overflow".to_string())?;
            state.phase = ConnectionPhase::Idle;
            state.current_attempt_id = None;
            state.cancellation.cancel();
            state.cancellation = CancellationToken::new();
            let next_generation = state.generation;
            if let Some(operation) = state.in_flight.as_ref() {
                operation.cancellation.cancel();
                (
                    next_generation,
                    cancelled_owner,
                    Some((operation.generation, operation.attempt_id)),
                    Some(operation.finished.subscribe()),
                )
            } else {
                (next_generation, cancelled_owner, None, None)
            }
        };

        if let Some(receiver) = finished.as_mut() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                if !*receiver.borrow() {
                    let _ = receiver.wait_for(|done| *done).await;
                }
            })
            .await;
        }

        if let Some((cancelled_generation, cancelled_attempt_id)) = cancelled_generation {
            let mut state = self.state.lock().await;
            if state.in_flight.as_ref().is_some_and(|operation| {
                operation.generation == cancelled_generation
                    && operation.attempt_id == cancelled_attempt_id
            }) {
                state.in_flight = None;
            }
        }
        Ok((next_generation, cancelled_owner))
    }

    /// 仅当预期代际和尝试编号仍为当前所有者时取消并递增代际。
    ///
    /// 不匹配返回 `Ok(None)` 且不修改状态；匹配时执行与无条件取消相同的协作式通知、令牌
    /// 轮换和最长一秒等待，并返回新代际。等待超时后任务可能继续到自行观察取消或结束，不
    /// 表示物理连接已经关闭；代际溢出是唯一由本方法直接返回的错误。
    pub async fn cancel_and_advance_if_current(
        &self,
        expected_generation: u64,
        expected_attempt_id: u64,
    ) -> Result<Option<u64>, String> {
        let (next_generation, cancelled_generation, mut finished) = {
            let _publication = self.status_publication.lock().await;
            let mut state = self.state.lock().await;
            if state.generation != expected_generation
                || state.current_attempt_id != Some(expected_attempt_id)
            {
                return Ok(None);
            }
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or_else(|| "Connection generation overflow".to_string())?;
            state.phase = ConnectionPhase::Idle;
            state.current_attempt_id = None;
            state.cancellation.cancel();
            state.cancellation = CancellationToken::new();
            let next_generation = state.generation;
            if let Some(operation) = state.in_flight.as_ref() {
                operation.cancellation.cancel();
                (
                    next_generation,
                    Some((operation.generation, operation.attempt_id)),
                    Some(operation.finished.subscribe()),
                )
            } else {
                (next_generation, None, None)
            }
        };
        if let Some(receiver) = finished.as_mut() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                if !*receiver.borrow() {
                    let _ = receiver.wait_for(|done| *done).await;
                }
            })
            .await;
        }
        if let Some((cancelled_generation, cancelled_attempt_id)) = cancelled_generation {
            let mut state = self.state.lock().await;
            if state.in_flight.as_ref().is_some_and(|operation| {
                operation.generation == cancelled_generation
                    && operation.attempt_id == cancelled_attempt_id
            }) {
                state.in_flight = None;
            }
        }
        Ok(Some(next_generation))
    }

    /// 在取消并递增连接代际的同时清除认证会话。
    ///
    /// 发布锁内依次取得会话写锁和状态锁，确认代际可递增后清空会话、发出协作式取消通知并
    /// 记录旧所有者；随后最多等待一秒让 in-flight 操作报告结束。超时后任务可能继续到自行
    /// 观察取消或结束。代际溢出时返回错误且不会清空会话；本方法不清理客户端槽位、监控群组，
    /// 也不保证物理连接已关闭。
    pub async fn cancel_and_advance_clearing_auth(
        &self,
        auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
    ) -> Result<(u64, Option<ConnectionAttemptKey>), String> {
        let (next_generation, cancelled_owner, cancelled_generation, mut finished) = {
            let _publication = self.status_publication.lock().await;
            let mut stored_session = auth_session.write().await;
            let mut state = self.state.lock().await;
            let cancelled_owner = state
                .current_attempt_id
                .map(|attempt_id| ConnectionAttemptKey::new(state.generation, attempt_id));
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or_else(|| "Connection generation overflow".to_string())?;
            *stored_session = None;
            state.phase = ConnectionPhase::Idle;
            state.current_attempt_id = None;
            state.cancellation.cancel();
            state.cancellation = CancellationToken::new();
            let next_generation = state.generation;
            if let Some(operation) = state.in_flight.as_ref() {
                operation.cancellation.cancel();
                (
                    next_generation,
                    cancelled_owner,
                    Some((operation.generation, operation.attempt_id)),
                    Some(operation.finished.subscribe()),
                )
            } else {
                (next_generation, cancelled_owner, None, None)
            }
        };

        if let Some(receiver) = finished.as_mut() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                if !*receiver.borrow() {
                    let _ = receiver.wait_for(|done| *done).await;
                }
            })
            .await;
        }
        if let Some((cancelled_generation, cancelled_attempt_id)) = cancelled_generation {
            let mut state = self.state.lock().await;
            if state.in_flight.as_ref().is_some_and(|operation| {
                operation.generation == cancelled_generation
                    && operation.attempt_id == cancelled_attempt_id
            }) {
                state.in_flight = None;
            }
        }
        Ok((next_generation, cancelled_owner))
    }

    /// 仅为仍当前的首次连接安装客户端。
    ///
    /// 先核对认证会话，再在状态锁下核对许可的代际、尝试编号、阶段、取消状态与 in-flight
    /// 记录，最后取得客户端槽位锁。成功时设置安装标志、写入带所有者键的客户端并进入
    /// `Connected`。会话变化、过期操作或槽位已占用会连同原客户端返回错误；槽位冲突还会
    /// 结束当前首次连接、切为逻辑空闲并发出协作式取消通知。
    pub async fn install_if_current(
        &self,
        permit: &ConnectionPermit,
        expected_session: &AuthSession,
        auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
        client_slot: &ClientSlot,
        installed: &std::sync::atomic::AtomicBool,
        client: ChatClient,
    ) -> Result<(), (String, ChatClient)> {
        let session_matches = auth_session.read().await.as_ref() == Some(expected_session);
        if !session_matches {
            return Err((
                "Authentication session changed during connect".to_string(),
                client,
            ));
        }

        let mut state = self.state.lock().await;
        let operation_is_current = state.generation == permit.generation
            && state.phase == ConnectionPhase::Connecting
            && state.current_attempt_id == Some(permit.attempt_id)
            && !permit.cancellation.is_cancelled()
            && state.in_flight.as_ref().is_some_and(|operation| {
                operation.generation == permit.generation
                    && operation.attempt_id == permit.attempt_id
            });
        if !operation_is_current {
            return Err(("Connection operation is stale".to_string(), client));
        }
        let mut slot = client_slot.lock().await;
        if slot.is_some() {
            if let Some(operation) = state.in_flight.take() {
                let _ = operation.finished.send(true);
            }
            state.phase = ConnectionPhase::Idle;
            state.current_attempt_id = None;
            state.cancellation.cancel();
            state.cancellation = CancellationToken::new();
            return Err(("Chat client was installed concurrently".to_string(), client));
        }
        installed.store(true, std::sync::atomic::Ordering::Release);
        *slot = Some(InstalledClient::new(
            ConnectionAttemptKey::new(permit.generation, permit.attempt_id),
            client,
        ));
        state.phase = ConnectionPhase::Connected;
        Ok(())
    }

    /// 仅为仍当前的自然重连安装替代客户端。
    ///
    /// 认证会话、代际、尝试编号与 `Reconnecting` 阶段均匹配且代际令牌未取消时才安装；
    /// 成功后进入 `Connected`。失败会连同未安装的客户端返回错误，不会覆盖已有槽位。
    pub async fn install_reconnected_if_current(
        &self,
        attempt: ConnectionAttemptKey,
        expected_session: &AuthSession,
        auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
        client_slot: &ClientSlot,
        installed: &std::sync::atomic::AtomicBool,
        client: ChatClient,
    ) -> Result<(), (String, ChatClient)> {
        let session_matches = auth_session.read().await.as_ref() == Some(expected_session);
        if !session_matches {
            return Err((
                "Authentication session changed during reconnect".to_string(),
                client,
            ));
        }
        let mut state = self.state.lock().await;
        let operation_is_current = state.generation == attempt.generation
            && state.phase == ConnectionPhase::Reconnecting
            && state.current_attempt_id == Some(attempt.attempt_id)
            && !state.cancellation.is_cancelled();
        if !operation_is_current {
            return Err(("Reconnect operation is stale".to_string(), client));
        }
        let mut slot = client_slot.lock().await;
        if slot.is_some() {
            return Err(("Chat client was installed concurrently".to_string(), client));
        }
        installed.store(true, std::sync::atomic::Ordering::Release);
        *slot = Some(InstalledClient::new(attempt, client));
        state.phase = ConnectionPhase::Connected;
        Ok(())
    }

    /// 在代际仍当前时发布登录会话及恢复的监控群组快照。
    ///
    /// 方法依次持有会话写锁、监控集合写锁和状态锁完成复核与赋值。代际已变化时返回错误，
    /// 两个共享对象均不修改；它不改变连接阶段或代际。
    pub async fn publish_login_if_current(
        &self,
        generation: u64,
        auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
        monitoring_groups: &tokio::sync::RwLock<HashSet<i64>>,
        session: AuthSession,
        restored_monitoring: HashSet<i64>,
    ) -> Result<(), String> {
        let mut stored_session = auth_session.write().await;
        let mut stored_monitoring = monitoring_groups.write().await;
        let state = self.state.lock().await;
        if state.generation != generation {
            return Err("Connection generation changed before login publication".to_string());
        }
        *stored_monitoring = restored_monitoring;
        *stored_session = Some(session);
        Ok(())
    }

    /// 在登录准备工作执行期间保持“当前代际且尚无会话”的门禁。
    ///
    /// 取得会话读锁和状态锁后先验证条件，再在持锁期间等待 `prepare`。条件不满足时不执行
    /// future 并返回错误；future 自身的错误原样返回。持锁可阻止相关写入，但不代表外部准备
    /// 操作与其他系统副作用是原子的。
    pub async fn prepare_login_if_current<T, F>(
        &self,
        generation: u64,
        auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
        prepare: F,
    ) -> Result<T, String>
    where
        F: Future<Output = Result<T, String>>,
    {
        let stored_session = auth_session.read().await;
        let state = self.state.lock().await;
        if state.generation != generation {
            return Err("Connection generation changed before login preparation".to_string());
        }
        if stored_session.is_some() {
            return Err("Authentication transition is no longer active".to_string());
        }

        prepare.await
    }

    /// 若代际仍当前，将已有认证会话重新标记到该代际。
    ///
    /// 没有会话或代际已变化时不做修改，也不返回错误；常用于显式断开后保留登录态供后续连接。
    pub async fn retag_session_if_current(
        &self,
        generation: u64,
        auth_session: &tokio::sync::RwLock<Option<AuthSession>>,
    ) {
        let mut stored_session = auth_session.write().await;
        let state = self.state.lock().await;
        if state.generation == generation {
            if let Some(session) = stored_session.as_mut() {
                session.generation = generation;
            }
        }
    }
}

#[derive(Clone)]
/// 注入 Tauri 命令并由后台任务克隆共享的应用状态。
///
/// 各 `Arc` 克隆指向同一底层资源；锁的作用域仅限对应字段。`shutdown` 则贯穿整个应用
/// 生命周期，用于在退出时通知连接和消息任务停止。业务 SQLite 不在启动时打开，
/// 只通过 [`account_db`](Self::account_db) 在认证成功后按 UID 激活。
pub struct AppState {
    /// 保护运行期配置快照；连接任务读取克隆后使用。
    pub config: Arc<tokio::sync::RwLock<AppConfig>>,
    /// 应用数据根目录及账号文件路径模型。
    #[allow(dead_code)]
    pub paths: crate::account::AppPaths,
    /// 非敏感账号索引；不保存 Token 或密码。
    pub account_index: Arc<crate::account::AccountIndexStore>,
    /// 系统凭据库；Token 与密码只经此接口读写。
    pub credentials: Arc<dyn crate::account::CredentialStore>,
    /// 当前活动账号的 SQLite 管理器；未登录时没有打开的业务库。
    pub account_db: Arc<crate::account::AccountDatabaseManager>,
    /// 旧单库一次性迁移器；仅在登录成功且已知 UID 后调用。
    pub legacy_migrator: Arc<crate::account::LegacyDatabaseMigrator>,
    /// 尚未完成二次验证的短期登录秘密缓存；仅驻留内存，不落盘。
    pub pending_login: Arc<crate::account::PendingLoginCache>,
    /// 保护当前已安装聊天客户端及其尝试所有者。
    pub chat_client: Arc<ClientSlot>,
    /// 保护从登录成功到登出之间的可选认证会话。
    pub auth_session: Arc<tokio::sync::RwLock<Option<AuthSession>>>,
    /// 保护当前内存中的受监控群组 ID 快照。
    pub monitoring_groups: Arc<tokio::sync::RwLock<HashSet<i64>>>,
    /// 串行化需要共同更新数据库与监控群组快照的群组操作。
    pub group_ops: Arc<tokio::sync::Mutex<()>>,
    /// 协调连接代际、尝试所有权、阶段迁移及状态发布。
    pub connection_coordinator: Arc<ConnectionCoordinator>,
    /// 复用认证与群组接口所需的 HTTP 客户端集合。
    pub http: Arc<AppHttpClients>,
    /// 仅驻留内存的用户私钥与派生群消息密钥。
    pub message_crypto: Arc<crate::message_content::MessageCryptoState>,
    /// 当前监控页面登记的实时消息批量 Channel；页面重载会替换旧接收端。
    pub message_channel: Arc<crate::commands::chat::MessageChannelSlot>,
    /// 保护供命令和前端状态通知读取的连接布尔快照。
    pub connected: Arc<tokio::sync::RwLock<bool>>,
    /// 应用级取消令牌；从 setup 创建到退出请求触发取消。
    pub shutdown: CancellationToken,
    /// setup 注入的 Tauri 句柄，用于事件发布；测试状态可不提供。
    pub app_handle: Option<AppHandle>,
}

/// 从 SQLite 读取已标记监控的群组，并恢复为内存 ID 集合。
///
/// 数据库查询失败时原样返回 `sqlx::Error`，不会产生部分集合。
/// 启动路径不再调用本函数；登录成功后由群组同步重建监控集合。
#[cfg_attr(not(test), allow(dead_code))]
pub async fn load_monitoring_groups(db: &SqliteStore) -> Result<HashSet<i64>, sqlx::Error> {
    Ok(db
        .groups
        .list_monitored()
        .await?
        .into_iter()
        .map(|group| group.group_id)
        .collect())
}

impl AppState {
    /// 返回 setup 保存的 Tauri 应用句柄。
    ///
    /// 未注入句柄时会 panic；生产状态总是在 Tauri setup 中设置该字段。
    pub fn app_handle(&self) -> &AppHandle {
        self.app_handle.as_ref().expect("app_handle not set")
    }
}

#[cfg(test)]
/// 构造仅具备账号基础设施、尚未打开业务库的测试用 [`AppState`]。
///
/// 返回的 [`tempfile::TempDir`] 必须由调用方持有到测试结束，避免数据根目录被提前删除。
/// 凭据走内存替身；该状态不注入 Tauri 句柄，也不预置认证会话或监控群组。
pub(crate) async fn test_state_with_account_foundation() -> (AppState, tempfile::TempDir) {
    test_state_with_credentials(Arc::new(
        crate::account::credentials::MemoryCredentialStore::default(),
    ))
    .await
}

#[cfg(test)]
/// 使用指定凭据仓储构造账号基础设施测试状态。
///
/// 调用方必须持有返回的临时目录，直到测试结束。该状态不注入 Tauri 句柄。
pub(crate) async fn test_state_with_credentials(
    credentials: Arc<dyn crate::account::CredentialStore>,
) -> (AppState, tempfile::TempDir) {
    use crate::account::{
        AccountDatabaseManager, AccountIndexStore, AppPaths, LegacyDatabaseMigrator,
        PendingLoginCache,
    };

    let temp = tempfile::tempdir().expect("创建账号测试临时目录");
    let paths = AppPaths::new(temp.path().to_path_buf());
    let config = AppConfig::default();
    let http = Arc::new(
        im_http::http_clients::AppHttpClients::new(&config)
            .expect("测试 HTTP 客户端应能用默认配置创建"),
    );

    let state = AppState {
        config: Arc::new(tokio::sync::RwLock::new(config)),
        paths: paths.clone(),
        account_index: Arc::new(AccountIndexStore::new(paths.index_file())),
        credentials,
        account_db: Arc::new(AccountDatabaseManager::new(paths.clone())),
        legacy_migrator: Arc::new(LegacyDatabaseMigrator::new(paths)),
        pending_login: Arc::new(PendingLoginCache::default()),
        chat_client: Arc::new(tokio::sync::Mutex::new(None)),
        auth_session: Arc::new(tokio::sync::RwLock::new(None)),
        monitoring_groups: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
        group_ops: Arc::new(tokio::sync::Mutex::new(())),
        connection_coordinator: Arc::new(ConnectionCoordinator::new()),
        http,
        message_crypto: Arc::new(crate::message_content::MessageCryptoState::default()),
        message_channel: Arc::new(tokio::sync::RwLock::new(None)),
        connected: Arc::new(tokio::sync::RwLock::new(false)),
        shutdown: CancellationToken::new(),
        app_handle: None,
    };
    (state, temp)
}

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicBool, Arc};

    use im_common::config::AppConfig;
    use im_store::group::GroupRow;

    use super::{
        load_monitoring_groups, test_state_with_account_foundation, AuthSession,
        ConnectionCoordinator, ConnectionPhase,
    };
    use crate::account::AccountError;

    /// 未登录时不得打开业务 SQLite，内存监控集合也必须为空。
    #[tokio::test]
    async fn app_state_has_no_business_database_before_login() {
        let (state, _temp) = test_state_with_account_foundation().await;
        let error = match state.account_db.active().await {
            Err(error) => error,
            Ok(_) => panic!("未登录时不应持有活动账号数据库"),
        };
        assert!(matches!(error, AccountError::NoActiveDatabase));
        assert!(state.monitoring_groups.read().await.is_empty());
    }

    #[tokio::test]
    async fn cancellation_advances_generation_and_wakes_blocked_operation() {
        // 验证显式取消先推进代际，并用协作式令牌唤醒等待中的 worker。
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let operation = coordinator.begin_connect(0).await.unwrap();
        let worker_coordinator = coordinator.clone();
        let generation = operation.generation();
        let attempt_id = operation.attempt_id();
        let cancellation = operation.cancellation_token();
        let worker = tokio::spawn(async move {
            cancellation.cancelled().await;
            worker_coordinator
                .finish_connect(generation, attempt_id)
                .await;
        });

        let next_generation = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            coordinator.cancel_and_advance(),
        )
        .await
        .expect("cancellation must not wait for a blocked network operation")
        .unwrap();

        assert_eq!(next_generation, 1);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn stale_finish_cannot_clear_new_attempt_in_same_generation() {
        // 验证同代旧尝试的迟到收尾不能清除较新的当前尝试或取消其令牌。
        let coordinator = ConnectionCoordinator::new();
        let old = coordinator.begin_connect(0).await.unwrap();
        coordinator
            .fail_connect_if_current(old.generation(), old.attempt_id())
            .await;
        let current = coordinator.begin_connect(0).await.unwrap();

        coordinator
            .finish_connect(old.generation(), old.attempt_id())
            .await;

        assert!(current.attempt_id() > old.attempt_id());
        assert_eq!(coordinator.phase().await, ConnectionPhase::Connecting);
        assert!(!current.cancellation_token().is_cancelled());
        coordinator
            .finish_connect(current.generation(), current.attempt_id())
            .await;
    }

    #[tokio::test]
    async fn disconnect_before_install_atomically_rejects_stale_client() {
        // 模拟断开先于客户端安装，确认旧许可被门禁拒绝且后续同代重试仍可获得新令牌。
        let coordinator = ConnectionCoordinator::new();
        let permit = coordinator.begin_connect(0).await.unwrap();
        let session = AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        };
        let auth_session = tokio::sync::RwLock::new(Some(session.clone()));
        let slot = tokio::sync::Mutex::new(None);
        let installed = AtomicBool::new(false);
        let statuses = std::sync::Mutex::new(Vec::new());

        assert!(
            coordinator
                .disconnect_and_publish_if_current(permit.generation(), || async {
                    statuses.lock().unwrap().push("disconnected");
                })
                .await
        );
        let error = coordinator
            .install_if_current(
                &permit,
                &session,
                &auth_session,
                &slot,
                &installed,
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap_err()
            .0;

        assert!(error.contains("stale"));
        assert_eq!(coordinator.phase().await, ConnectionPhase::Idle);
        assert!(slot.lock().await.is_none());
        assert_eq!(*statuses.lock().unwrap(), ["disconnected"]);
        coordinator
            .finish_connect(permit.generation(), permit.attempt_id())
            .await;
        let retry = coordinator
            .begin_connect(permit.generation())
            .await
            .unwrap();
        assert!(!retry.cancellation_token().is_cancelled());
        coordinator
            .finish_connect(retry.generation(), retry.attempt_id())
            .await;
    }

    #[tokio::test]
    async fn installed_connection_publishes_connected_before_racing_disconnect() {
        // 制造连接与断开发布竞争，验证发布锁保持可观察顺序并最终落到空闲阶段。
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let permit = coordinator.begin_connect(0).await.unwrap();
        let generation = permit.generation();
        let session = AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation,
        };
        let auth_session = tokio::sync::RwLock::new(Some(session.clone()));
        let slot = tokio::sync::Mutex::new(None);
        let installed = AtomicBool::new(false);
        coordinator
            .install_if_current(
                &permit,
                &session,
                &auth_session,
                &slot,
                &installed,
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();

        let statuses = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let (connected_entered_tx, connected_entered_rx) = tokio::sync::oneshot::channel();
        let (release_connected_tx, release_connected_rx) = tokio::sync::oneshot::channel();
        let connected_coordinator = coordinator.clone();
        let connected_statuses = statuses.clone();
        let connected_task = tokio::spawn(async move {
            connected_coordinator
                .publish_connected_if_current(generation, || async move {
                    connected_entered_tx.send(()).unwrap();
                    release_connected_rx.await.unwrap();
                    connected_statuses.lock().await.push("connected");
                })
                .await
        });
        connected_entered_rx.await.unwrap();

        let disconnected_coordinator = coordinator.clone();
        let disconnected_statuses = statuses.clone();
        let disconnected_task = tokio::spawn(async move {
            disconnected_coordinator
                .disconnect_and_publish_if_current(generation, || async move {
                    disconnected_statuses.lock().await.push("disconnected");
                })
                .await
        });
        tokio::task::yield_now().await;
        release_connected_tx.send(()).unwrap();

        assert!(connected_task.await.unwrap());
        assert!(disconnected_task.await.unwrap());
        assert_eq!(*statuses.lock().await, ["connected", "disconnected"]);
        assert_eq!(coordinator.phase().await, ConnectionPhase::Idle);
    }

    #[tokio::test]
    async fn stale_generation_status_waits_then_cannot_overwrite_new_connected_status() {
        // 验证旧代断开通知即使排队等待，也不能覆盖新代已经发布的连接状态。
        let coordinator = Arc::new(ConnectionCoordinator::new());
        let old = coordinator.begin_connect(0).await.unwrap();
        coordinator
            .finish_connect(old.generation(), old.attempt_id())
            .await;
        assert_eq!(coordinator.cancel_and_advance().await.unwrap(), 1);

        let current = coordinator.begin_connect(1).await.unwrap();
        let session = AuthSession {
            uid: 42,
            token: "new-token".to_string(),
            generation: 1,
        };
        let auth_session = tokio::sync::RwLock::new(Some(session.clone()));
        let slot = tokio::sync::Mutex::new(None);
        coordinator
            .install_if_current(
                &current,
                &session,
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();
        let statuses = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let current_coordinator = coordinator.clone();
        let current_statuses = statuses.clone();
        let current_publish = tokio::spawn(async move {
            current_coordinator
                .publish_connected_if_current(1, || async move {
                    entered_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                    current_statuses.lock().await.push("connected");
                })
                .await
        });
        entered_rx.await.unwrap();

        let stale_coordinator = coordinator.clone();
        let stale_statuses = statuses.clone();
        let stale_publish = tokio::spawn(async move {
            stale_coordinator
                .publish_disconnected_if_current(0, || async move {
                    stale_statuses.lock().await.push("disconnected");
                })
                .await
        });
        tokio::task::yield_now().await;
        release_tx.send(()).unwrap();

        assert!(current_publish.await.unwrap());
        assert!(!stale_publish.await.unwrap());
        assert_eq!(*statuses.lock().await, ["connected"]);
    }

    #[tokio::test]
    async fn natural_loss_preserves_generation_token_and_allows_reconnect_install() {
        // 模拟自然掉线，确认不推进代际或取消令牌，并允许原所有者安装重连客户端。
        let coordinator = ConnectionCoordinator::new();
        let permit = coordinator.begin_connect(0).await.unwrap();
        let cancellation = permit.cancellation_token();
        let session = AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        };
        let auth_session = tokio::sync::RwLock::new(Some(session.clone()));
        let slot = tokio::sync::Mutex::new(None);
        let installed = AtomicBool::new(false);
        coordinator
            .install_if_current(
                &permit,
                &session,
                &auth_session,
                &slot,
                &installed,
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();

        assert!(
            coordinator
                .begin_reconnect_and_publish_if_current(0, permit.attempt_id(), || async {})
                .await
        );
        assert!(!cancellation.is_cancelled());
        slot.lock().await.take();
        coordinator
            .install_reconnected_if_current(
                permit.key(),
                &session,
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();

        assert_eq!(coordinator.phase().await, ConnectionPhase::Connected);
        assert!(slot.lock().await.is_some());
    }

    #[tokio::test]
    async fn connecting_publication_is_generation_and_attempt_safe_during_reconnect() {
        // 验证重连中的状态发布同时受代际和尝试编号约束，过期代际无法发布。
        let coordinator = ConnectionCoordinator::new();
        let permit = coordinator.begin_connect(0).await.unwrap();
        let session = AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        };
        let auth_session = tokio::sync::RwLock::new(Some(session.clone()));
        let slot = tokio::sync::Mutex::new(None);
        coordinator
            .install_if_current(
                &permit,
                &session,
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();
        coordinator
            .begin_reconnect_and_publish_if_current(0, permit.attempt_id(), || async {})
            .await;
        let published = Arc::new(AtomicBool::new(false));
        let observed = published.clone();

        assert!(
            coordinator
                .publish_connecting_if_current(0, permit.attempt_id(), || async move {
                    observed.store(true, std::sync::atomic::Ordering::SeqCst);
                })
                .await
        );
        assert!(published.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            !coordinator
                .publish_connecting_if_current(1, permit.attempt_id(), || async {})
                .await
        );
    }

    #[tokio::test]
    async fn explicit_cancel_during_backoff_rejects_stale_reconnect_install() {
        // 模拟重连退避期间显式取消，确认代际推进后旧重连结果不能重新安装客户端。
        let coordinator = ConnectionCoordinator::new();
        let permit = coordinator.begin_connect(0).await.unwrap();
        let cancellation = permit.cancellation_token();
        let session = AuthSession {
            uid: 42,
            token: "token".to_string(),
            generation: 0,
        };
        let auth_session = tokio::sync::RwLock::new(Some(session.clone()));
        let slot = tokio::sync::Mutex::new(None);
        let installed = AtomicBool::new(false);
        coordinator
            .install_if_current(
                &permit,
                &session,
                &auth_session,
                &slot,
                &installed,
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap();
        coordinator
            .begin_reconnect_and_publish_if_current(0, permit.attempt_id(), || async {})
            .await;
        slot.lock().await.take();

        assert_eq!(coordinator.cancel_and_advance().await.unwrap(), 1);
        assert!(cancellation.is_cancelled());
        let error = coordinator
            .install_reconnected_if_current(
                permit.key(),
                &session,
                &auth_session,
                &slot,
                &AtomicBool::new(false),
                im_chat::ChatClient::new(AppConfig::default()),
            )
            .await
            .unwrap_err()
            .0;

        assert!(error.contains("stale"));
        assert!(slot.lock().await.is_none());
    }

    #[tokio::test]
    async fn restores_monitored_group_ids_from_sqlite() {
        // 以混合监控标记写入数据库，验证启动恢复只重建启用监控的群组 ID 集合。
        let store = im_store::SqliteStore::new(":memory:").await.unwrap();
        for (group_id, monitored) in [(10, 1), (20, 0), (30, 1)] {
            store
                .groups
                .insert_or_update(&GroupRow {
                    group_id,
                    name: group_id.to_string(),
                    pic: String::new(),
                    host_id: None,
                    member_count: 0,
                    created_at: 0,
                    monitored,
                    updated_at: 0,
                })
                .await
                .unwrap();
        }

        let restored = load_monitoring_groups(&store).await.unwrap();

        assert_eq!(restored, [10, 30].into_iter().collect());
    }
}
