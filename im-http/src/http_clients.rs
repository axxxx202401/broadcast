use super::im_biz::ImBizClient;
use super::openchat_user::OpenChatUserClient;
use im_common::{config::AppConfig, error::AppError, version_key::HeaderManager};
use std::time::Duration;

/// 所有应用 HTTP 请求的总超时时间，覆盖建立连接、等待响应及读取响应体。
pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// HTTP 响应体上限：私有帧允许的最大内容加六字节帧头。
pub const MAX_HTTP_RESPONSE_SIZE: usize = im_common::MAX_FRAME_BODY_SIZE + 6;

/// 构建带总请求超时的共享 HTTP 客户端。
///
/// 客户端配置失败时转换为 [`AppError::Http`]。
pub(crate) fn build_http_client(timeout: Duration) -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| AppError::Http(error.to_string()))
}

/// 在限定内存占用的前提下读取完整 HTTP 响应体。
///
/// 若服务端提供 `Content-Length`，会在读取前预检并立即拒绝超限声明；对于分块传输、
/// 缺失或不可信的长度声明，则逐 chunk 累加并在追加前检查上限。两层检查共同避免把
/// 无界响应完整聚合到内存而导致 OOM。长度计算溢出、响应读取失败或实际累计长度超过
/// `limit` 时返回 [`AppError::Http`]。
pub(crate) async fn read_response_body_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, AppError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(AppError::Http(format!(
            "HTTP response Content-Length exceeds limit {limit}"
        )));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| AppError::Http(error.to_string()))?
    {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| AppError::Http("HTTP response length overflow".to_string()))?;
        if next_len > limit {
            return Err(AppError::Http(format!(
                "HTTP response body length {next_len} exceeds limit {limit}"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// 应用共用的 HTTP 业务客户端集合。
///
/// 两个业务客户端共享连接池与超时配置，但分别持有自己的请求头管理器及协议封装。
pub struct AppHttpClients {
    /// OpenChat 用户接口客户端，负责登录、验证及用户资料等 JSON API。
    pub openchat_user: OpenChatUserClient,
    /// IM 业务接口客户端，负责群列表等 Protobuf 业务请求。
    pub im_biz: ImBizClient,
}

impl AppHttpClients {
    /// 根据应用配置创建 OpenChat 用户客户端与 im-biz 客户端。
    ///
    /// HTTP 客户端使用 [`HTTP_REQUEST_TIMEOUT`]；请求头密钥或正文 AES 密钥无效，
    /// 以及底层 HTTP 客户端构建失败时，返回对应配置或传输错误。
    pub fn new(config: &AppConfig) -> Result<Self, AppError> {
        let http = build_http_client(HTTP_REQUEST_TIMEOUT)?;
        let openchat_headers = HeaderManager::try_new(
            config.server.version_secret_name.clone(),
            config.server.header_aes_key.clone(),
        )?;
        let im_biz_headers = HeaderManager::try_new(
            config.server.version_secret_name.clone(),
            config.server.header_aes_key.clone(),
        )?;
        Ok(Self {
            openchat_user: OpenChatUserClient::new(
                http.clone(),
                config.server.openchat_user_url.clone(),
                config.server.body_aes_key.clone(),
                openchat_headers,
                config.device.clone(),
            )?,
            im_biz: ImBizClient::new(
                http,
                config.server.im_biz_url.clone(),
                config.server.body_aes_key.clone(),
                im_biz_headers,
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use im_common::config::AppConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{build_http_client, read_response_body_limited, AppHttpClients};

    #[test]
    fn invalid_header_key_returns_configuration_error() {
        // 构造客户端集合时应立即暴露无效请求头密钥，而不是推迟到首次请求。
        let mut config = AppConfig::default();
        config.server.header_aes_key = "short".to_string();

        let error = AppHttpClients::new(&config).err().unwrap();

        assert!(error.to_string().contains("16 bytes"));
    }

    #[tokio::test]
    async fn chunked_response_is_rejected_as_soon_as_accumulated_limit_is_exceeded() {
        // 无 Content-Length 的 chunked 响应依靠累计值检查，并在越界 chunk 到达时拒绝。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n12345\r\n5\r\n67890\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();

        let error = read_response_body_limited(response, 8).await.unwrap_err();

        assert!(error.to_string().contains("exceeds"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn configured_client_enforces_total_request_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let client = build_http_client(Duration::from_millis(20)).unwrap();

        let error = client
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap_err();

        assert!(error.is_timeout());
        server.abort();
    }
}
