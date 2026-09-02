use super::client::{build_im_biz_request_body, parse_im_biz_response};
use im_common::aes::AesCipher;
use im_common::error::AppError;
use im_common::version_key::VersionKeyManager;
use im_proto::GroupContactListResp;
use prost::Message;

pub struct ImBizClient {
    base_url: String,
    http: reqwest::Client,
    body_cipher: AesCipher,
    x_one_manager: VersionKeyManager,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupInfo {
    pub group_id: i64,
    pub name: String,
    pub pic: String,
    pub host_id: Option<i64>,
    pub member_count: i64,
}

impl From<&im_proto::GroupBase> for GroupInfo {
    fn from(group: &im_proto::GroupBase) -> Self {
        Self {
            group_id: group.group_id,
            name: group.name.clone(),
            pic: group.pic.clone(),
            host_id: if group.host_id != 0 {
                Some(group.host_id)
            } else {
                None
            },
            member_count: group.member_count,
        }
    }
}

impl ImBizClient {
    pub fn new(
        base_url: String,
        body_aes_key: String,
        x_one_manager: VersionKeyManager,
    ) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
            body_cipher: AesCipher::new(body_aes_key.as_bytes()),
            x_one_manager,
        }
    }

    /// 获取群列表
    pub async fn fetch_group_list(
        &self,
        client_info: &im_proto::ClientInfo,
    ) -> Result<Vec<GroupInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // Build CommonResultReq with ClientInfo
        let req = im_proto::CommonResultReq {
            client_info: Some(client_info.clone()),
        };
        let _req_bytes = req.encode_to_vec();

        // Build GroupContactListReq wrapping CommonResultReq
        let group_req = im_proto::GroupContactListReq {
            common_result_req: Some(req),
        };
        let payload_bytes = group_req.encode_to_vec();

        let body = build_im_biz_request_body(&self.body_cipher, &payload_bytes);
        let x_one = self
            .x_one_manager
            .build_x_one()
            .map_err(|e| AppError::Http(e.to_string()))?;

        let resp = self
            .http
            .post(format!("{}/group/groupContactList", self.base_url))
            .header("X-One", x_one)
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::Http(e.to_string()))?;

        let status = resp.status();
        let data = resp.bytes().await.map_err(|e| AppError::Http(e.to_string()))?;

        if !status.is_success() {
            return Err(AppError::Http(format!(
                "HTTP {}: {}",
                status,
                String::from_utf8_lossy(&data)
            ))
            .into());
        }

        let decrypted = parse_im_biz_response(&self.body_cipher, &data)?;
        let resp_msg: GroupContactListResp = GroupContactListResp::decode(decrypted.as_slice())?;

        let groups: Vec<GroupInfo> = resp_msg
            .groups
            .iter()
            .map(GroupInfo::from)
            .collect();

        Ok(groups)
    }
}
