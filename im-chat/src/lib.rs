//! IM TCP 长连接客户端。
//!
//! 本 crate 负责建立和维护聊天服务器的 TCP 长连接，并处理帧编解码、传输变换、
//! 心跳与重连。协议基础类型和错误来自 `im-common`，登录及消息正文类型来自
//! `im-proto`；HTTP 请求与数据存储不属于本 crate 的职责。

pub mod client;
/// TCP 帧线格式、长度限制及 Java 兼容的加密、压缩编解码。
pub mod frame;
/// 心跳与聊天协议消息 ID，以及可取消的周期发送循环。
pub mod heartbeat;
/// 指数退避序列与可取消的连接、登录重试循环。
pub mod reconnect;

/// 聊天客户端及其可克隆发送句柄的根级重新导出。
pub use client::{ChatClient, ChatSender};
/// 常用帧编码、解码函数的根级重新导出。
pub use frame::{decode_frame, encode_frame};

#[cfg(test)]
mod tests;
