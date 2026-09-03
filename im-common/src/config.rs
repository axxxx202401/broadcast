//! 应用运行配置。
//!
//! 本模块将服务端连接参数与客户端设备信息分组，并支持通过 `serde` 序列化和
//! 反序列化。桌面程序通过编译期环境变量加载真实配置；内置 [`Default`] 仅为测试提供
//! 不访问远端服务的占位值。

use std::str::FromStr;

use crate::error::{AppError, AppResult};
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
    /// 组合离线测试服务端配置与新设备配置。
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            device: DeviceConfig::new(),
        }
    }
}

impl AppConfig {
    /// 从编译当前二进制时注入的环境变量构造应用配置。
    ///
    /// 所有变量均为必填项。该方法只在错误中记录变量名和约束，不回显配置值，避免密钥
    /// 进入启动日志。环境变量由构建脚本提供；运行已经生成的安装包时再设置变量不会改变
    /// 配置。
    pub fn from_build_env() -> AppResult<Self> {
        Self::from_values(&[
            ("IM_OPENCHAT_USER_URL", option_env!("IM_OPENCHAT_USER_URL")),
            ("IM_BIZ_URL", option_env!("IM_BIZ_URL")),
            ("IM_CHAT_HOST", option_env!("IM_CHAT_HOST")),
            ("IM_CHAT_PORT", option_env!("IM_CHAT_PORT")),
            (
                "IM_VERSION_SECRET_NAME",
                option_env!("IM_VERSION_SECRET_NAME"),
            ),
            ("IM_BODY_AES_KEY", option_env!("IM_BODY_AES_KEY")),
            ("IM_HEADER_AES_KEY", option_env!("IM_HEADER_AES_KEY")),
            ("IM_APP_VER", option_env!("IM_APP_VER")),
            ("IM_PACKAGE_CODE", option_env!("IM_PACKAGE_CODE")),
            ("IM_PLAT", option_env!("IM_PLAT")),
            ("IM_LANGUAGE", option_env!("IM_LANGUAGE")),
            ("IM_SYS_MODEL", option_env!("IM_SYS_MODEL")),
        ])
    }

    /// 从给定键值对构造配置，供不读取真实构建环境的单元测试覆盖校验分支。
    #[cfg(test)]
    pub(crate) fn from_pairs(values: &[(&str, &str)]) -> AppResult<Self> {
        let values = values
            .iter()
            .map(|(name, value)| (*name, Some(*value)))
            .collect::<Vec<_>>();
        Self::from_values(&values)
    }

    /// 解析并校验构建变量快照。
    fn from_values(values: &[(&str, Option<&str>)]) -> AppResult<Self> {
        let openchat_user_url = required(values, "IM_OPENCHAT_USER_URL")?;
        validate_http_url("IM_OPENCHAT_USER_URL", openchat_user_url)?;
        let im_biz_url = required(values, "IM_BIZ_URL")?;
        validate_http_url("IM_BIZ_URL", im_biz_url)?;

        let im_chat_host = required(values, "IM_CHAT_HOST")?;
        validate_chat_host("IM_CHAT_HOST", im_chat_host)?;

        let body_aes_key = required(values, "IM_BODY_AES_KEY")?;
        validate_aes_key("IM_BODY_AES_KEY", body_aes_key)?;
        let header_aes_key = required(values, "IM_HEADER_AES_KEY")?;
        validate_aes_key("IM_HEADER_AES_KEY", header_aes_key)?;
        let im_chat_port = parse_number(values, "IM_CHAT_PORT")?;
        if im_chat_port == 0 {
            return Err(invalid("IM_CHAT_PORT", "must be between 1 and 65535"));
        }

        Ok(Self {
            server: ServerConfig {
                openchat_user_url: openchat_user_url.to_string(),
                im_biz_url: im_biz_url.to_string(),
                im_chat_host: im_chat_host.to_string(),
                im_chat_port,
                version_secret_name: required_text(values, "IM_VERSION_SECRET_NAME")?.to_string(),
                body_aes_key: body_aes_key.to_string(),
                header_aes_key: header_aes_key.to_string(),
            },
            device: DeviceConfig {
                app_ver: parse_number(values, "IM_APP_VER")?,
                package_code: parse_number(values, "IM_PACKAGE_CODE")?,
                plat: parse_number(values, "IM_PLAT")?,
                language: parse_number(values, "IM_LANGUAGE")?,
                sys_mac: uuid::Uuid::new_v4().to_string(),
                sys_model: required_text(values, "IM_SYS_MODEL")?.to_string(),
            },
        })
    }
}

impl Default for ServerConfig {
    /// 返回不会访问真实远端服务的测试占位配置。
    fn default() -> Self {
        Self {
            openchat_user_url: "http://127.0.0.1".to_string(),
            im_biz_url: "http://127.0.0.1".to_string(),
            im_chat_host: "127.0.0.1".to_string(),
            im_chat_port: 1,
            version_secret_name: "test-version-secret".to_string(),
            body_aes_key: "0000000000000000".to_string(),
            header_aes_key: "0000000000000000".to_string(),
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

/// 读取必填变量，同时拒绝空值；错误信息只包含变量名。
fn required<'a>(values: &'a [(&str, Option<&'a str>)], name: &str) -> AppResult<&'a str> {
    values
        .iter()
        .find_map(|(candidate, value)| (*candidate == name).then_some(*value).flatten())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(name, "is required"))
}

/// 读取不允许首尾空白的文本变量。
fn required_text<'a>(values: &'a [(&str, Option<&'a str>)], name: &str) -> AppResult<&'a str> {
    let value = required(values, name)?;
    if value.trim().is_empty() || value.trim() != value {
        return Err(invalid(
            name,
            "must be non-empty and have no surrounding whitespace",
        ));
    }
    Ok(value)
}

/// 解析协议整数；具体目标类型由配置字段决定。
fn parse_number<T>(values: &[(&str, Option<&str>)], name: &str) -> AppResult<T>
where
    T: FromStr,
{
    required(values, name)?
        .parse()
        .map_err(|_| invalid(name, "must be a valid integer"))
}

/// 校验 HTTP(S) 基础 URL，不允许认证信息、查询参数、片段或首尾空白。
fn validate_http_url(name: &str, value: &str) -> AppResult<()> {
    let parsed = url::Url::parse(value);
    let valid = parsed.as_ref().is_ok_and(|url| {
        value.trim() == value
            && matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });
    if !valid {
        return Err(invalid(name, "must be an absolute HTTP(S) base URL"));
    }
    Ok(())
}

/// 校验 TCP 主机字段，允许域名、IPv4 与方括号 IPv6，但拒绝协议、端口和路径。
fn validate_chat_host(name: &str, value: &str) -> AppResult<()> {
    let parsed = url::Url::parse(&format!("tcp://{value}"));
    let valid = parsed.as_ref().is_ok_and(|url| {
        value.trim() == value
            && url.host_str().is_some()
            && url.port().is_none()
            && url.path().is_empty()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    });
    if !valid {
        return Err(invalid(
            name,
            "must be a host without scheme, port, or path",
        ));
    }
    Ok(())
}

/// 当前协议固定使用 AES-128，因此构建密钥必须恰好为 16 字节。
fn validate_aes_key(name: &str, value: &str) -> AppResult<()> {
    if value.len() != 16 {
        return Err(invalid(name, "must be exactly 16 bytes for AES-128"));
    }
    Ok(())
}

/// 创建不包含配置值的统一错误。
fn invalid(name: &str, constraint: &str) -> AppError {
    AppError::Config(format!("{name} {constraint}"))
}
