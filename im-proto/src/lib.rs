/// Generated protobuf types (produced by prost-build in `build.rs`).
/// All messages and enums from `proto/broadcast.proto` are available here.
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

// Re-export commonly used types at the crate root for convenience
pub use pb::{
    ClientInfo,
    CommonResult,
    CommonResultReq,
    UserBase,
    GroupBase,
    GroupMemberBase,
    LoginReq,
    LoginResp,
    UrlInfo,
    MessageType,
    Platform,
    GroupMessage,
    PushGroupMessage,
    LoginSessionMessage,
    GroupContactListResp,
    GroupContactListReq,
    ErrrMessage,
};
