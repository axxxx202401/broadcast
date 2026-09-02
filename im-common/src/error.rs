//! 跨 crate 的统一错误模型。
//!
//! 本模块将密码学、传输、协议、I/O、HTTP、业务、存储和配置等失败归一为
//! [`AppError`]，并通过 [`AppResult`] 统一可失败操作的返回类型。

use thiserror::Error;

/// 应用基础能力及其调用方共享的错误类型。
#[derive(Error, Debug)]
pub enum AppError {
    /// AES 解密失败，字符串包含底层失败原因。
    #[error("AES decryption failed: {0}")]
    AesDecrypt(String),
    /// AES 加密失败，字符串包含底层失败原因。
    #[error("AES encryption failed: {0}")]
    AesEncrypt(String),
    /// TCP 帧格式、长度或传输变换不符合约束。
    #[error("TCP frame malformed: {0}")]
    TcpFrame(String),
    /// protobuf 或其他协议数据无法解析。
    #[error("Proto parse error: {0}")]
    ProtoParse(String),
    /// 文件、网络流等标准 I/O 操作失败。
    ///
    /// `From<std::io::Error>` 允许调用方使用 `?` 自动转换错误。
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// HTTP 请求、响应或状态处理失败。
    #[error("HTTP error: {0}")]
    Http(String),
    /// 服务端返回业务拒绝；保留业务错误码及说明。
    #[error("Business error {code}: {message}")]
    Business { code: i32, message: String },
    /// 数据库存取或事务操作失败。
    #[error("Database error: {0}")]
    Db(String),
    /// 配置值缺失、格式错误或不满足组件约束。
    #[error("Configuration error: {0}")]
    Config(String),
    /// 登录流程未能完成。
    #[error("Login failed: {0}")]
    Login(String),
    /// 聊天客户端已连接，拒绝重复建立连接。
    #[error("Chat client is already connected")]
    AlreadyConnected,
}

/// 以 [`AppError`] 为错误类型的应用通用结果。
pub type AppResult<T> = Result<T, AppError>;
