//! IM TCP 长连接客户端。
//!
//! 本 crate 负责建立和维护聊天服务器的 TCP 长连接，并处理帧编解码、传输变换、
//! 心跳与重连。协议基础类型和错误来自 `im-common`，登录及消息正文类型来自
//! `im-proto`；HTTP 请求与数据存储不属于本 crate 的职责。

pub mod client;
pub mod frame;
pub mod heartbeat;
pub mod reconnect;

pub use client::{ChatClient, ChatSender};
pub use frame::{decode_frame, encode_frame};

#[cfg(test)]
mod tests;
