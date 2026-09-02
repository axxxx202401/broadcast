use std::{future::Future, time::Duration};

use tokio::time::{interval_at, Instant};
use tokio_util::sync::CancellationToken;
use tracing::debug;

/// Message IDs
pub const HEARTBEAT_MSG_ID: u16 = 1000;
pub const LOGIN_SERVER_MSG_ID: u16 = 1100;
pub const PUSH_LOGIN_SUCCESS: u16 = 1201;
pub const PUSH_GROUP_MESSAGE: u16 = 2202;
pub const PUSH_RECALL_GROUP_MESSAGE: u16 = 2205;

pub fn heartbeat_message() -> (u16, &'static [u8]) {
    (HEARTBEAT_MSG_ID, &[])
}

/// Sends a periodic heartbeat until its connection generation is cancelled.
///
/// The first deadline is one complete period in the future. This intentionally
/// avoids `tokio::time::interval`'s immediate first tick.
pub async fn heartbeat_loop<F, Fut, E>(
    period: Duration,
    cancellation: CancellationToken,
    mut send_heartbeat: F,
) -> Result<(), E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let mut ticker = interval_at(Instant::now() + period, period);
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(()),
            _ = ticker.tick() => {}
        }
        debug!("Sending heartbeat");
        send_heartbeat().await?;
    }
}
