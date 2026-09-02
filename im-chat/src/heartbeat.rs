use std::{future::Future, time::Duration};

use tokio::time::{interval_at, Instant};
use tokio_util::sync::CancellationToken;
use tracing::debug;

/// 心跳消息 ID。
pub const HEARTBEAT_MSG_ID: u16 = 1000;
/// 登录服务消息 ID。
pub const LOGIN_SERVER_MSG_ID: u16 = 1100;
/// 登录成功推送消息 ID。
pub const PUSH_LOGIN_SUCCESS: u16 = 1201;
/// 群消息推送消息 ID。
pub const PUSH_GROUP_MESSAGE: u16 = 2202;
/// 群消息撤回推送消息 ID。
pub const PUSH_RECALL_GROUP_MESSAGE: u16 = 2205;

/// 构造心跳的消息 ID 与空应用正文。
pub fn heartbeat_message() -> (u16, &'static [u8]) {
    (HEARTBEAT_MSG_ID, &[])
}

/// 按指定周期发送心跳，直到当前连接代次被取消或发送失败。
///
/// 使用 [`interval_at`] 将首次截止时间设为当前时刻之后的一个完整 `period`，
/// 避免普通 interval 立即产生第一次 tick。周期由上层传入，本函数不选择默认值。
///
/// 等待 tick 时收到取消信号会返回 `Ok(())`，且 `biased` 分支使同时就绪时优先
/// 处理取消；一次发送返回错误时立即将该错误返回。已经进入
/// `send_heartbeat().await` 的发送过程不由此取消分支中断。
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
