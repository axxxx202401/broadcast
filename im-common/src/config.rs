use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub device: DeviceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub openchat_user_url: String,
    pub im_biz_url: String,
    pub im_chat_host: String,
    pub im_chat_port: u16,
    pub version_secret_name: String,
    pub body_aes_key: String,
    pub header_aes_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub app_ver: i32,
    pub package_code: i32,
    pub plat: i32,
    pub language: i32,
    pub sys_mac: String,
    pub sys_model: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            device: DeviceConfig::new(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            openchat_user_url: "https://test-ochat-user1.68chat.co".to_string(),
            im_biz_url: "https://test-biz-b.68chat.co".to_string(),
            im_chat_host: "35.220.159.225".to_string(),
            im_chat_port: 9500,
            version_secret_name: "f82956caf0fa90aecf24d5ef9541f624".to_string(),
            body_aes_key: "97b1f52761ffc7f8".to_string(),
            header_aes_key: "f58c15f54e8f7826".to_string(),
        }
    }
}

impl DeviceConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            app_ver: 680,
            package_code: 9803,
            plat: 0,
            language: 2,
            sys_mac: uuid::Uuid::new_v4().to_string(),
            sys_model: "PC-TOOLS".to_string(),
        }
    }
}
