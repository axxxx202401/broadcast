//! IM HTTP 协议与业务客户端。
//!
//! 本 crate 负责组装项目使用的 HTTP 客户端、编解码 HTTP 消息体中的私有帧协议，
//! 并为 OpenChat 用户接口和 IM 业务接口提供面向应用的调用封装。通用密码算法、
//! 配置与错误类型由 `im-common` 提供，Protobuf 消息定义由 `im-proto` 提供。

/// 私有帧协议的请求构建、响应解析及大小限制。
pub mod client;
/// 共享 HTTP 客户端的构建、超时配置与响应体限流读取。
pub mod http_clients;
/// 基于 Protobuf 的 IM 业务接口客户端。
pub mod im_biz;
/// 基于 JSON 的 OpenChat 用户接口客户端。
pub mod openchat_user;
/// 第三方开奖历史 API 客户端。
pub mod lottery;
