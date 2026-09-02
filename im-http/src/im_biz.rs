//! im-biz 群列表协议客户端。
//!
//! 本模块调用 Java 兼容端点 `POST /group/groupContactList`：请求的 Protobuf
//! `GroupContactListReq` 在 wire field 1 直接携带 `ClientInfo`，经 AES 加密后封装为
//! 首字节为 `0xC1` 的长度帧，并通过 `X-One` 请求头发送。为兼容既有服务，请求头
//! `Content-Type` 固定为 `application/json; charset=utf-8`，但请求 body 实际是二进制
//! 加密帧，并非 JSON。
//!
//! 响应帧经校验后按帧标志执行可选变换：存在 `zipped` 标志时先解压，存在
//! `encrypted` 标志时再解密，随后解码 Protobuf。`common_result` 存在时要求
//! `err_code == 200`；缺失时当前实现继续处理响应中的群数据。Java 兼容的 `GroupBase`
//! Protobuf schema 覆盖 wire field 1–17；本模块只映射
//! [`GroupInfo`](crate::im_biz::GroupInfo) 明确列出的字段，不根据字段名推测额外业务事实。

use super::{
    client::{build_im_biz_request_body, parse_im_biz_response},
    http_clients::{read_response_body_limited, MAX_HTTP_RESPONSE_SIZE},
};
use im_common::aes::AesCipher;
use im_common::error::AppError;
use im_common::version_key::HeaderManager;
use im_proto::GroupContactListResp;
use prost::Message;

#[cfg(debug_assertions)]
use std::time::Instant;

/// 调用 im-biz 群列表端点的客户端。
///
/// 客户端持有 HTTP 连接池、请求/响应帧使用的 AES 密钥，以及生成 `X-One` 的头管理器。
/// 构造本身不发起网络请求；实际请求由 [`Self::fetch_group_list`] 发出。
pub struct ImBizClient {
    base_url: String,
    http: reqwest::Client,
    body_cipher: AesCipher,
    header_manager: HeaderManager,
}

/// 从 `GroupBase` 响应映射出的手写业务视图。
///
/// 该类型只保留当前调用方需要的映射结果；不会为 Java `GroupBase` wire field 1–17
/// 中未映射的字段补充或推断业务语义。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupInfo {
    /// Protobuf `GroupBase.group_id` 的原始值。
    pub group_id: i64,
    /// Protobuf `GroupBase.name` 的原始值。
    pub name: String,
    /// Protobuf `GroupBase.pic` 的原始值。
    pub pic: String,
    /// Protobuf `GroupBase.host_id` 的可选表示；wire 值为 `0` 时映射为 `None`。
    pub host_id: Option<i64>,
    /// Protobuf `GroupBase.member_count` 的原始值。
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

/// 解码群列表 Protobuf、检查业务码并映射公开模型。
///
/// 输入必须是 im-biz 响应帧完成按标志可选解压、解密后的 `GroupContactListResp`
/// 字节。函数先执行 Protobuf decode；若存在 `common_result`，要求
/// `err_code == 200`，否则返回业务错误；若缺失则继续处理 `groups`。最后把每个
/// `GroupBase` 映射为 [`GroupInfo`]，其中 `host_id == 0` 映射为 `None`。
/// Protobuf 无法解码，或存在非 200 业务码时返回对应错误。
fn decode_group_list_response(data: &[u8]) -> Result<Vec<GroupInfo>, AppError> {
    let response = GroupContactListResp::decode(data)
        .map_err(|error| AppError::ProtoParse(error.to_string()))?;

    if let Some(result) = response.common_result {
        if result.err_code != 200 {
            return Err(AppError::Business {
                code: result.err_code,
                message: result.err_msg,
            });
        }
    }

    Ok(response.groups.iter().map(GroupInfo::from).collect())
}

impl ImBizClient {
    /// 创建 im-biz 客户端。
    ///
    /// `base_url` 是 `/group/groupContactList` 的服务根地址；`body_aes_key` 用于构造和
    /// 解析 `0xC1` 二进制协议帧，`header_manager` 用于生成每次请求的 `X-One`。
    ///
    /// 此函数只初始化本地状态，不发起网络请求。AES 密钥不是恰好 16 字节时返回错误。
    pub fn new(
        http: reqwest::Client,
        base_url: String,
        body_aes_key: String,
        header_manager: HeaderManager,
    ) -> Result<Self, AppError> {
        Ok(Self {
            base_url,
            http,
            body_cipher: AesCipher::try_new(body_aes_key.as_bytes())?,
            header_manager,
        })
    }

    /// 从 `POST /group/groupContactList` 获取群列表。
    ///
    /// 为保持 Java 协议兼容，请求 Protobuf `GroupContactListReq` 的 wire field 1 直接
    /// 写入 `ClientInfo`，随后使用 AES 加密并封装为首字节 `0xC1` 的二进制长度帧。
    /// 请求携带动态生成的 `X-One`；虽然 `Content-Type` 是
    /// `application/json; charset=utf-8`，body 并不是 JSON，而是上述加密帧。
    ///
    /// 成功 HTTP 响应会依次经过大小限制、帧解析、按标志可选变换、Protobuf decode、
    /// 业务码检查和模型映射。可选变换在 `zipped` 时先解压，在 `encrypted` 时再解密。
    /// `common_result` 存在时要求 `err_code == 200`；缺失时当前实现继续处理群数据。
    /// Java 兼容的 `GroupBase` wire schema 覆盖 field 1–17，当前只映射 [`GroupInfo`]
    /// 中的字段，且 `host_id == 0` 映射为 `None`。
    ///
    /// 此方法会发起网络请求；debug 构建还会记录请求元数据、脱敏响应和解码失败日志。
    /// 构帧、`X-One` 生成、网络传输、响应过大、非成功 HTTP 状态、帧解析或可选变换、
    /// Protobuf 解码，或 `common_result` 中的非 200 业务码均会返回错误。
    pub async fn fetch_group_list(
        &self,
        client_info: &im_proto::ClientInfo,
    ) -> Result<Vec<GroupInfo>, Box<dyn std::error::Error + Send + Sync>> {
        const PATH: &str = "/group/groupContactList";
        #[cfg(debug_assertions)]
        let started_at = Instant::now();
        let group_req = im_proto::GroupContactListReq {
            client_info: Some(client_info.clone()),
        };
        let payload_bytes = group_req.encode_to_vec();

        let body = build_im_biz_request_body(&self.body_cipher, &payload_bytes)?;
        let x_one = self
            .header_manager
            .build_x_one()
            .map_err(|e| AppError::Http(e.to_string()))?;

        #[cfg(debug_assertions)]
        tracing::debug!(
            method = "POST",
            path = PATH,
            app_ver = client_info.app_ver,
            package_code = client_info.package_code,
            plat = client_info.plat,
            token_len = client_info.token.len(),
            sys_mac_len = client_info.sys_mac.len(),
            frame_byte_0 = format_args!("0x{:02X}", body[0]),
            frame_byte_1 = format_args!("0x{:02X}", body[1]),
            declared_body_len = u32::from_be_bytes(body[2..6].try_into().unwrap_or_default()),
            protobuf_len = payload_bytes.len(),
            wire_len = body.len(),
            x_one_len = x_one.len(),
            "im-biz request"
        );

        let resp = self
            .http
            .post(format!("{}{PATH}", self.base_url))
            .header("X-One", x_one)
            .header("Content-Type", "application/json; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(|error| AppError::Http(format!("POST {PATH} request failed: {error}")))?;

        let status = resp.status();
        #[cfg(debug_assertions)]
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<missing>")
            .to_string();
        let data = read_response_body_limited(resp, MAX_HTTP_RESPONSE_SIZE).await?;

        #[cfg(debug_assertions)]
        tracing::debug!(
            method = "POST",
            path = PATH,
            %status,
            %content_type,
            response_len = data.len(),
            elapsed_ms = started_at.elapsed().as_millis(),
            response = %super::openchat_user::sanitize_debug_json(&data),
            "im-biz raw response"
        );

        if !status.is_success() {
            return Err(AppError::Http(format!(
                "POST {PATH} -> HTTP {}: {}",
                status,
                super::openchat_user::sanitize_debug_json(&data)
            ))
            .into());
        }

        let decrypted = parse_im_biz_response(&self.body_cipher, &data).map_err(|error| {
            AppError::Http(format!("POST {PATH} response decode failed: {error}"))
        })?;
        let groups = decode_group_list_response(&decrypted).map_err(|error| {
            #[cfg(debug_assertions)]
            tracing::error!(
                method = "POST",
                path = PATH,
                decoded_len = decrypted.len(),
                schema = "GroupBase fields 1-17 (Java protobuf)",
                %error,
                "im-biz protobuf response decode failed"
            );
            error
        })?;

        #[cfg(debug_assertions)]
        tracing::debug!(
            method = "POST",
            path = PATH,
            decoded_len = decrypted.len(),
            group_count = groups.len(),
            "im-biz decoded response"
        );

        Ok(groups)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use im_common::error::AppError;
    use im_common::version_key::HeaderManager;
    use im_proto::{
        ClientInfo, CommonResult, CommonResultReq, GroupContactListReq, GroupContactListResp,
    };

    use super::{decode_group_list_response, ImBizClient};
    use prost::Message;

    #[test]
    fn group_list_rejects_protobuf_business_error() {
        // common_result 存在时，非 200 业务码必须保留服务端码和消息。
        let response = GroupContactListResp {
            common_result: Some(CommonResult {
                err_code: 4012,
                err_msg: "session expired".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let error = decode_group_list_response(&response.encode_to_vec()).unwrap_err();

        assert!(matches!(
            error,
            AppError::Business {
                code: 4012,
                ref message
            } if message == "session expired"
        ));
    }

    #[test]
    fn group_list_accepts_java_success_code_200() {
        // common_result 存在时，Java 协议约定 200 通过业务码检查。
        let response = GroupContactListResp {
            common_result: Some(CommonResult {
                err_code: 200,
                ..Default::default()
            }),
            group_count: 1,
            groups: vec![im_proto::GroupBase {
                group_id: 1,
                name: "group".to_string(),
                ..Default::default()
            }],
        };

        let groups = decode_group_list_response(&response.encode_to_vec()).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_id, 1);
    }

    #[test]
    fn group_list_decodes_current_java_group_base_fields() {
        // 手工 wire 覆盖 GroupBase field 1–8、14–17，并验证 varint、长度类型及多字节 tag
        // 能由当前 Java 兼容类型解码；不以 fixture 推断未出现字段的业务语义。
        let java_group = [
            0x08, 0x01, 0x10, 0x02, 0x1a, 0x01, b'g', 0x22, 0x00, 0x28, 0x00, 0x30, 0x01, 0x38,
            0x03, 0x40, 0x01, 0x72, 0x01, b'r', 0x78, 0x64, 0x80, 0x01, 0x01, 0x8a, 0x01, 0x01,
            b'n',
        ];
        let mut response = vec![0x1a, java_group.len() as u8];
        response.extend_from_slice(&java_group);

        let decoded = GroupContactListResp::decode(response.as_slice()).unwrap();
        let decoded_group = &decoded.groups[0];
        assert!(decoded_group.bf_join_friend);
        assert_eq!(decoded_group.remark, "r");
        assert_eq!(decoded_group.max_member_count, 100);
        assert!(decoded_group.bf_join_notice);
        assert_eq!(decoded_group.notice, "n");

        let groups = decode_group_list_response(&response).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_id, 1);
        assert_eq!(groups[0].host_id, Some(2));
        assert_eq!(groups[0].name, "g");
        assert_eq!(groups[0].member_count, 3);
    }

    #[test]
    fn group_contact_request_field_one_is_direct_client_info() {
        // 验证 field 1 直接编码 ClientInfo，与 Java wire 契约一致，不额外套消息层。
        let client_info = ClientInfo {
            token: "session-token".to_string(),
            app_ver: 680,
            package_code: 9803,
            language: 2,
            sys_mac: "device-id".to_string(),
            ..Default::default()
        };
        let common_wire = CommonResultReq {
            client_info: Some(client_info.clone()),
        }
        .encode_to_vec();
        let group_wire = GroupContactListReq {
            client_info: Some(client_info.clone()),
        }
        .encode_to_vec();

        assert_eq!(group_wire, common_wire);
        assert_eq!(
            CommonResultReq::decode(group_wire.as_slice())
                .unwrap()
                .client_info,
            Some(client_info.clone())
        );
        assert_eq!(
            GroupContactListReq::decode(common_wire.as_slice())
                .unwrap()
                .client_info,
            Some(client_info)
        );
    }

    #[test]
    fn invalid_body_key_returns_constructor_error() {
        // 无效 AES 密钥应在无网络副作用的构造阶段立即返回错误。
        let header_manager =
            HeaderManager::new("secret".to_string(), "1234567890abcdef".to_string());

        let error = ImBizClient::new(
            reqwest::Client::new(),
            "https://example.invalid".to_string(),
            "short".to_string(),
            header_manager,
        )
        .err()
        .unwrap();

        assert!(error.to_string().contains("16 bytes"));
    }

    #[tokio::test]
    async fn im_biz_request_uses_java_json_content_type() {
        // 兼容 Java 的 JSON Content-Type 声明；请求 body 仍由客户端发送二进制加密帧。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let count = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&request[..count]).into_owned()
        });
        let header_manager =
            HeaderManager::new("secret".to_string(), "1234567890abcdef".to_string());
        let client = ImBizClient::new(
            reqwest::Client::new(),
            format!("http://{address}"),
            "97b1f52761ffc7f8".to_string(),
            header_manager,
        )
        .unwrap();

        let error = client
            .fetch_group_list(&ClientInfo::default())
            .await
            .unwrap_err();
        let request = server.await.unwrap().to_ascii_lowercase();

        assert!(request.contains("\r\ncontent-type: application/json; charset=utf-8\r\n"));
        assert!(error.to_string().contains("POST /group/groupContactList"));
        assert!(error.to_string().contains("500"));
    }
}
