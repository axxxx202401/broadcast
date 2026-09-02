//! 即时通信各 crate 共享的基础能力。
//!
//! 本 crate 集中提供 AES 加解密、客户端配置、统一错误、TCP 头部解析和版本请求头
//! 等跨模块能力。各实现保留在对应公开模块中，常用的 [`AppError`] 则从 crate 根
//! 重新导出，供调用方使用统一导入路径。

/// AES 加解密工具。
pub mod aes;
/// 客户端、服务端和设备配置。
pub mod config;
/// 跨 crate 使用的统一错误类型。
pub mod error;
/// TCP 帧头部的编码与解析。
pub mod tcp_head;
/// X-One、X-Ten 等版本请求头的生成逻辑。
pub mod version_key;

/// 编码后单个传输帧正文的最大长度：8 MiB。
///
/// 该限制约束线上帧中声明和实际承载的正文长度。
pub const MAX_FRAME_BODY_SIZE: usize = 8 * 1024 * 1024;
/// 应用原文或解压后正文的最大长度：32 MiB。
///
/// 该限制独立于 [`MAX_FRAME_BODY_SIZE`]，用于在压缩前及解压过程中控制内存增长。
pub const MAX_DECOMPRESSED_BODY_SIZE: usize = 32 * 1024 * 1024;

#[cfg(test)]
mod tests;

/// 统一应用错误的根级重新导出。
pub use error::AppError;
