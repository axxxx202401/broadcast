use std::{future::Future, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// 产生带最大值封顶的指数退避时长。
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    next: Duration,
    maximum: Duration,
}

impl ExponentialBackoff {
    /// 以 `initial` 创建退避序列，并将首个时长立即限制在 `maximum` 内。
    pub fn new(initial: Duration, maximum: Duration) -> Self {
        Self {
            next: initial.min(maximum),
            maximum,
        }
    }
}

impl Default for ExponentialBackoff {
    /// 创建从 1 秒开始、以 30 秒封顶的默认退避序列。
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(30))
    }
}

impl Iterator for ExponentialBackoff {
    type Item = Duration;

    /// 返回当前退避时长，并将下一项翻倍后限制在最大值内。
    ///
    /// 迭代器不会耗尽；默认序列为 1、2、4、8、16、30、30……秒。
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next;
        self.next = self.next.saturating_mul(2).min(self.maximum);
        Some(current)
    }
}

/// 持续重试完整的“连接并登录”动作，直到成功或取消。
///
/// 每次尝试前（包括第一次）都先等待对应退避时长；默认等待序列从 1 秒开始并在
/// 30 秒封顶。动作成功时返回 `Some`，等待或动作期间取消时返回 `None`，动作
/// 返回错误时记录告警并继续下一次重试，因此未取消且始终失败时会无限重试。
///
/// 连接动作与等待函数均由调用方注入，使测试无需真实网络连接或实际退避等待。
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
