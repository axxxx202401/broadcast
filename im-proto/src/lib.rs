//! 即时通信 protobuf 协议类型。
//!
//! 本 crate 的类型由构建脚本根据 `proto/broadcast.proto` 生成。生成实现集中在
//! [`pb`] 模块中；crate 根仅重新导出调用方常用的协议类型，为应用代码提供稳定、
//! 简洁的导入路径。

/// `prost-build` 生成代码的边界。
///
/// 该模块直接包含构建产物，不应在源码中手工维护其中的消息和枚举定义。
#[allow(missing_docs)]
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

/// 调用方常用的 protobuf 消息与枚举。
///
/// 根级重新导出将调用方与生成文件名及内部布局隔离。
pub use pb::{
    AudioObj, ClientInfo, CommonResult, CommonResultReq, ErrrMessage, FileObj, GetKeyPairReq,
    GetKeyPairResp, GroupBase, GroupContactListReq, GroupContactListResp, GroupMemberBase,
    GroupMessage, ImageObj, KeyPairBase, KeyPairType, LoginReq, LoginResp, LoginSessionMessage,
    MessageType, Platform, PushGroupMessage, PushLoginSuccessMessage, ReceiveGroupMessage, TextObj,
    UpdateKeyPairReq, UpdateKeyPairResp, UrlInfo, UserBase, VideoObj,
};
