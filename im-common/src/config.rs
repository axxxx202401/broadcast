//! 应用运行配置。
//!
//! 本模块将服务端连接参数与客户端设备信息分组，并支持通过 `serde` 序列化和
//! 反序列化。内置 [`Default`] 值是当前开发、测试环境使用的配置，不代表生产部署约定。

use serde::{Deserialize, Serialize};

/// 应用顶层配置，由服务端连接配置和设备配置组成。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// HTTP、聊天长连接及加密相关服务端参数。
    pub server: ServerConfig,
    /// 请求和长连接登录上报的客户端设备参数。
    pub device: DeviceConfig,
}

/// 服务地址、端口及传输加密参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// OpenChat 用户服务的基础 URL。
    pub openchat_user_url: String,
    /// IM 业务服务的基础 URL。
    pub im_biz_url: String,
    /// IM 聊天长连接主机。
    pub im_chat_host: String,
    /// IM 聊天长连接端口。
    pub im_chat_port: u16,
    /// 生成版本请求头时使用的 secret name。
    pub version_secret_name: String,
    /// 加解密消息正文时使用的 AES 密钥。
    pub body_aes_key: String,
    /// 加密版本请求头时使用的 AES 密钥。
    pub header_aes_key: String,
}

/// 客户端版本、平台和设备标识配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// 应用版本号。
    pub app_ver: i32,
    /// 安装包版本码。
    pub package_code: i32,
    /// 平台编码；数值语义由调用协议约定。
    pub plat: i32,
    /// 语言编码；数值语义由调用协议约定。
    pub language: i32,
    /// 随请求和长连接登录上报的设备标识。
    pub sys_mac: String,
    /// 随请求和长连接登录上报的设备型号。
    pub sys_model: String,
}

impl Default for AppConfig {
    /// 组合当前开发、测试环境的默认服务端配置与新设备配置。
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            device: DeviceConfig::new(),
        }
    }
}

impl Default for ServerConfig {
    /// 返回当前开发、测试环境使用的服务地址和密钥参数。
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
    /// 创建一份默认设备配置。
    ///
    /// 该方法委托给 [`Default`]；每次调用都会生成新的 `sys_mac`。
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DeviceConfig {
    /// 返回当前开发、测试环境的版本和设备默认值。
    ///
    /// `sys_mac` 使用随机 UUID，因此不同构造结果拥有独立的设备标识。
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
