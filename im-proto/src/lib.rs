/// Generated protobuf types (produced by prost-build in `build.rs`).
/// All messages and enums from `proto/broadcast.proto` are available here.
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

// Re-export commonly used types at the crate root for convenience
pub use pb::{
    ClientInfo, CommonResult, CommonResultReq, ErrrMessage, GroupBase, GroupContactListReq,
    GroupContactListResp, GroupMemberBase, GroupMessage, KeyPairBase, LoginReq, LoginResp,
    LoginSessionMessage, MessageType, Platform, PushGroupMessage, PushLoginSuccessMessage,
    ReceiveGroupMessage, UrlInfo, UserBase,
};
