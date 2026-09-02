use super::*;

pub struct ImBizClient {
    base_url: String,
    http: reqwest::Client,
    x_one_manager: Option<im_common::version_key::VersionKeyManager>,
}

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub group_id: i64,
    pub name: String,
    pub pic: String,
    pub host_id: Option<i64>,
    pub member_count: i64,
}

impl ImBizClient {
    pub fn new(
        base_url: String,
        x_one_manager: im_common::version_key::VersionKeyManager,
    ) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
            x_one_manager: Some(x_one_manager),
        }
    }

    /// 获取群列表
    pub async fn fetch_group_list(
        &self,
        client_info: &im_proto::ClientInfo,
    ) -> Result<Vec<GroupInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现 Protobuf + AES 加密请求
        let _ = client_info;
        todo!("Phase 2: implement protobuf HTTP request")
    }
}
