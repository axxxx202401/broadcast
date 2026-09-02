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
pub struct AuthSession {
    pub uid: i64,
    pub token: String,
    pub generation: u64,
}

pub struct ConnectionCoordinator {
    state: Mutex<ConnectionState>,
    status_publication: Mutex<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPhase {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
}

struct ConnectionState {
    generation: u64,
    next_attempt_id: u64,
    current_attempt_id: Option<u64>,
    phase: ConnectionPhase,
    cancellation: CancellationToken,
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

struct InFlightConnection {
    generation: u64,
    attempt_id: u64,
    cancellation: CancellationToken,
    finished: watch::Sender<bool>,
}

pub struct ConnectionPermit {
    generation: u64,
    attempt_id: u64,
    cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionAttemptKey {
    generation: u64,
    attempt_id: u64,
}

impl ConnectionAttemptKey {
    pub fn new(generation: u64, attempt_id: u64) -> Self {
        Self {
            generation,
            attempt_id,
        }
    }
}

#[derive(Debug)]
pub struct InstalledClient {
    key: ConnectionAttemptKey,
    pub client: ChatClient,
}

impl InstalledClient {
    pub fn new(key: ConnectionAttemptKey, client: ChatClient) -> Self {
        Self { key, client }
    }

    pub fn key(&self) -> ConnectionAttemptKey {
        self.key
    }
}

pub type ClientSlot = tokio::sync::Mutex<Option<InstalledClient>>;

impl ConnectionPermit {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn attempt_id(&self) -> u64 {
        self.attempt_id
    }

    #[cfg(test)]
    pub fn key(&self) -> ConnectionAttemptKey {
        ConnectionAttemptKey::new(self.generation, self.attempt_id)
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl ConnectionCoordinator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ConnectionState::default()),
            status_publication: Mutex::new(()),
        }
    }

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

    pub async fn phase(&self) -> ConnectionPhase {
        self.state.lock().await.phase
    }

    #[cfg(test)]
    pub async fn cancel_and_advance(&self) -> Result<u64, String> {
        Ok(self.cancel_and_advance_with_owner().await?.0)
    }

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
pub struct AppState {
    pub config: Arc<tokio::sync::RwLock<AppConfig>>,
    pub db: Arc<SqliteStore>,
    pub chat_client: Arc<ClientSlot>,
    pub auth_session: Arc<tokio::sync::RwLock<Option<AuthSession>>>,
    pub monitoring_groups: Arc<tokio::sync::RwLock<HashSet<i64>>>,
    pub group_ops: Arc<tokio::sync::Mutex<()>>,
    pub connection_coordinator: Arc<ConnectionCoordinator>,
    pub http: Arc<AppHttpClients>,
    pub connected: Arc<tokio::sync::RwLock<bool>>,
    pub shutdown: CancellationToken,
    pub app_handle: Option<AppHandle>,
}

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
    pub fn app_handle(&self) -> &AppHandle {
        self.app_handle.as_ref().expect("app_handle not set")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicBool, Arc};

    use im_common::config::AppConfig;
    use im_store::group::GroupRow;

    use super::{load_monitoring_groups, AuthSession, ConnectionCoordinator, ConnectionPhase};

    #[tokio::test]
    async fn cancellation_advances_generation_and_wakes_blocked_operation() {
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
