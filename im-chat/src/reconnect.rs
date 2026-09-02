use std::{future::Future, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    next: Duration,
    maximum: Duration,
}

impl ExponentialBackoff {
    pub fn new(initial: Duration, maximum: Duration) -> Self {
        Self {
            next: initial.min(maximum),
            maximum,
        }
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(30))
    }
}

impl Iterator for ExponentialBackoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next;
        self.next = self.next.saturating_mul(2).min(self.maximum);
        Some(current)
    }
}

/// Retries a complete connect-and-login action forever, until success or
/// cancellation. Both the action and sleeper are injected so tests never need
/// real network connections or production backoff delays.
pub async fn reconnect_loop<T, E, Connect, ConnectFuture, Sleep, SleepFuture>(
    cancellation: CancellationToken,
    backoff: ExponentialBackoff,
    mut connect_and_login: Connect,
    mut sleep: Sleep,
) -> Option<T>
where
    Connect: FnMut() -> ConnectFuture,
    ConnectFuture: Future<Output = Result<T, E>>,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: Future<Output = ()>,
    E: std::fmt::Display,
{
    for (attempt, delay) in backoff.enumerate() {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return None,
            _ = sleep(delay) => {}
        }
        info!("Attempting reconnect (attempt {})", attempt + 1);
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return None,
            result = connect_and_login() => result,
        };
        match result {
            Ok(connected) => return Some(connected),
            Err(error) => warn!("Reconnect attempt {} failed: {}", attempt + 1, error),
        }
    }
    unreachable!("exponential backoff is infinite")
}
