use super::*;

pub struct OpenChatUserClient {
    base_url: String,
    http: reqwest::Client,
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
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    /// 发送短信验证码（带极验）
    pub async fn send_sms_captcha(
        &self,
        phone: &str,
        country_code: i32,
        gt4_dto: &serde_json::Value,
    ) -> Result<SendCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现加密请求
        let _ = (&self.base_url, phone, country_code, gt4_dto);
        todo!("Phase 2: implement encrypted HTTP request")
    }

    /// 获取 validateToken
    pub async fn issued(
        &self,
        validate_scene: i32,
    ) -> Result<ValidateTokenResult, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现加密请求
        let _ = validate_scene;
        todo!("Phase 2: implement encrypted HTTP request")
    }

    /// 验证验证码
    pub async fn verify(
        &self,
        validate_token: &str,
        second_mac: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现加密请求
        let _ = (validate_token, second_mac);
        todo!("Phase 2: implement encrypted HTTP request")
    }

    /// 登录
    pub async fn login(
        &self,
        phone: &str,
        country_code: i32,
        validate_token: &str,
    ) -> Result<LoginResult, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Phase 2 - 实现加密请求
        let _ = (phone, country_code, validate_token);
        todo!("Phase 2: implement encrypted HTTP request")
    }
}
