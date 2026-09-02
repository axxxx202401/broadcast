use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::client::ChatClient;

/// Attempts to reconnect to the server with exponential backoff.
///
/// Retries up to `max_retries` times; if `max_retries` is 0 the loop
/// runs until the client disconnects.
pub async fn reconnect_loop(client: &mut ChatClient, max_retries: u32) {
    let mut retries: u32 = 0;
    let mut backoff_ms: u64 = 1000;
    let max_backoff_ms: u64 = 30_000;

        loop {
        info!("Attempting reconnect (attempt {})", retries + 1);
        match client.connect().await {
            Ok(()) => {
                info!("Reconnected successfully");
                break;
            }
            Err(e) => {
                warn!("Reconnect attempt {} failed: {}", retries + 1, e);
                retries += 1;
                if max_retries > 0 && retries >= max_retries {
                    warn!("Max reconnect retries reached, giving up");
                    break;
                }
            }
        }
        // Exponential backoff with cap
        let backoff = Duration::from_millis(backoff_ms);
        sleep(backoff).await;
        backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
    }
}
