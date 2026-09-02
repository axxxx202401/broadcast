use tokio::time::{interval, Duration};
use tracing::{info, debug};

use crate::client::ChatClient;
use crate::frame::encode_frame;

/// Message IDs
pub const HEARTBEAT_MSG_ID: u16 = 1000;
pub const LOGIN_SERVER_MSG_ID: u16 = 1100;
pub const PUSH_LOGIN_SUCCESS: u16 = 1201;
pub const PUSH_GROUP_MESSAGE: u16 = 2202;
pub const PUSH_RECALL_GROUP_MESSAGE: u16 = 2205;

/// Sends a periodic heartbeat to keep the TCP connection alive.
///
/// If `send_heartbeat` returns true the client should re-transmit the
/// heartbeat; if it returns false the task exits cleanly.
pub async fn heartbeat_loop(client: &ChatClient, interval_secs: u64) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    loop {
        ticker.tick().await;
        debug!("Sending heartbeat");
        let frame = encode_frame(HEARTBEAT_MSG_ID, &[], true, false);
        if let Err(e) = client.send(HEARTBEAT_MSG_ID, &frame[8..]).await {
            info!("Heartbeat send failed: {}", e);
            break;
        }
    }
}
