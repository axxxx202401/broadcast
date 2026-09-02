use super::im_biz::ImBizClient;
use super::openchat_user::OpenChatUserClient;
use im_common::{config::AppConfig, error::AppError, version_key::HeaderManager};
use std::time::Duration;

pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_HTTP_RESPONSE_SIZE: usize = im_common::MAX_FRAME_BODY_SIZE + 6;

pub(crate) fn build_http_client(timeout: Duration) -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| AppError::Http(error.to_string()))
}

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

pub struct AppHttpClients {
    pub openchat_user: OpenChatUserClient,
    pub im_biz: ImBizClient,
}

impl AppHttpClients {
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
        let mut config = AppConfig::default();
        config.server.header_aes_key = "short".to_string();

        let error = AppHttpClients::new(&config).err().unwrap();

        assert!(error.to_string().contains("16 bytes"));
    }

    #[tokio::test]
    async fn chunked_response_is_rejected_as_soon_as_accumulated_limit_is_exceeded() {
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
