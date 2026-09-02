use super::client::{build_gateway_request_body, parse_gateway_response};
use im_common::aes::AesCipher;
use im_common::error::AppError;
use im_common::version_key::VersionKeyManager;

pub struct OpenChatUserClient {
    base_url: String,
    http: reqwest::Client,
    body_cipher: AesCipher,
    version_manager: VersionKeyManager,
}

#[derive(Debug, serde::Deserialize)]
pub struct SendCodeResult {
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ValidateTokenResult {
    pub validate_token: Option<String>,
    pub success: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct LoginResult {
    pub uid: Option<i64>,
    pub token: Option<String>,
    pub is_not_last_device_mac: Option<bool>,
    pub is_login_out: i32,
    pub old_session_id: Option<String>,
}

impl OpenChatUserClient {
    pub fn new(base_url: String, body_aes_key: String, version_manager: VersionKeyManager) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
            body_cipher: AesCipher::new(body_aes_key.as_bytes()),
            version_manager,
        }
    }

    async fn post_encrypted(&self, path: &str, json_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
        let body = build_gateway_request_body(&self.body_cipher, json_bytes);
        let x_one = self
            .version_manager
            .build_x_one()
            .map_err(|e| AppError::Http(e.to_string()))?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .header("X-One", x_one)
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::Http(e.to_string()))?;

        let status = resp.status();
        let data = resp.bytes().await.map_err(|e| AppError::Http(e.to_string()))?;

        if !status.is_success() {
            return Err(AppError::Http(format!("HTTP {}: {}", status, String::from_utf8_lossy(&data))));
        }

        parse_gateway_response(&self.body_cipher, &data)
    }

    /// 发送短信验证码（带极验）
    pub async fn send_sms_captcha(
        &self,
        phone: &str,
        country_code: i32,
        gt4_dto: &serde_json::Value,
    ) -> Result<SendCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "phone": phone,
            "countryCode": country_code,
            "gt4DTO": gt4_dto
        });
        let json_bytes = serde_json::to_vec(&payload)?;
        let data = self.post_encrypted("/user/unauthorized/sendSmsCaptchaWithGt4", &json_bytes).await?;
        let result: SendCodeResult = serde_json::from_slice(&data)?;
        Ok(result)
    }

    /// 获取 validateToken
    pub async fn issued(
        &self,
        validate_scene: i32,
    ) -> Result<ValidateTokenResult, Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "validateScene": validate_scene
        });
        let json_bytes = serde_json::to_vec(&payload)?;
        let data = self.post_encrypted("/user/unauthorized/issued", &json_bytes).await?;
        let result: ValidateTokenResult = serde_json::from_slice(&data)?;
        Ok(result)
    }

    /// 验证验证码
    pub async fn verify(
        &self,
        validate_token: &str,
        second_mac: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut payload = serde_json::json!({
            "validateToken": validate_token
        });
        if let Some(mac) = second_mac {
            payload["secondMac"] = serde_json::json!(mac);
        }
        let json_bytes = serde_json::to_vec(&payload)?;
        let data = self
            .post_encrypted("/user/unauthorized/verify", &json_bytes)
            .await?;
        let _: serde_json::Value = serde_json::from_slice(&data)?;
        Ok(())
    }

    /// 登录
    pub async fn login(
        &self,
        phone: &str,
        country_code: i32,
        validate_token: &str,
    ) -> Result<LoginResult, Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "phone": phone,
            "countryCode": country_code,
            "loginType": 0,
            "validateToken": validate_token
        });
        let json_bytes = serde_json::to_vec(&payload)?;
        let data = self.post_encrypted("/sns/login/login", &json_bytes).await?;
        let result: LoginResult = serde_json::from_slice(&data)?;
        Ok(result)
    }
}
