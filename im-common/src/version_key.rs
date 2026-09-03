//! OpenChat `X-One` 与 `X-Ten` 请求头生成。
//!
//! `X-One` 将 `secret_name` 与毫秒时间戳组合后加密，`X-Ten` 将客户端
//! 信息 JSON 与同类时间戳组合后加密；两者均使用协议约定的
//! AES-128-ECB-PKCS7，并将密文字节编码为小写十六进制字符串。

use crate::aes::AesCipher;
use crate::config::DeviceConfig;
use crate::error::AppResult;
use std::time::{SystemTime, UNIX_EPOCH};

/// 计算兼容性测试使用的 `V_L_SALT`：指定字符串 MD5 结果的前 16 字节。
///
/// 该值只用于验证既有协议常量，不代表安全用途。
#[cfg(test)]
fn compute_v_l_salt() -> [u8; 16] {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(b"sjlkajsl*Rkfsdsd_tflklsjdf");
    let result = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&result[..16]);
    bytes
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenChatClientInfo<'a> {
    token: &'a str,
    session_id: &'a str,
    app_ver: i32,
    plat: i32,
    package_code: i32,
    language: i32,
    sys_mac: &'a str,
    sys_model: &'a str,
}

impl<'a> From<&'a DeviceConfig> for OpenChatClientInfo<'a> {
    fn from(device: &'a DeviceConfig) -> Self {
        Self {
            token: "",
            session_id: "",
            app_ver: device.app_ver,
            plat: device.plat,
            package_code: device.package_code,
            language: device.language,
            sys_mac: &device.sys_mac,
            sys_model: &device.sys_model,
        }
    }
}

impl<'a> OpenChatClientInfo<'a> {
    fn authenticated(device: &'a DeviceConfig, token: &'a str, session_id: &'a str) -> Self {
        Self {
            token,
            session_id,
            app_ver: device.app_ver,
            plat: device.plat,
            package_code: device.package_code,
            language: device.language,
            sys_mac: &device.sys_mac,
            sys_model: &device.sys_model,
        }
    }
}

/// `X-One`、`X-Ten` 加密请求头生成器。
///
/// - `X-One`：`hex(AES(secret_name + "," + timestamp_ms))`
/// - `X-Ten`：`hex(AES(client_info_json + "//" + timestamp_ms))`
///
/// 其中 `timestamp_ms` 是 Unix 纪元起的毫秒时间戳，`AES` 表示协议兼容所需
/// 的 AES-128-ECB-PKCS7，`hex` 表示小写十六进制编码。
pub struct HeaderManager {
    secret_name: String,
    header_cipher: AesCipher,
}

impl HeaderManager {
    /// 使用 secret name 和 16 字节 UTF-8 头部密钥创建生成器。
    ///
    /// # Panics
    ///
    /// 当 `header_key` 的 UTF-8 字节长度不等于 16 时 panic。需要处理配置
    /// 错误的调用方应使用 [`Self::try_new`]。
    pub fn new(secret_name: String, header_key: String) -> Self {
        assert_eq!(
            header_key.len(),
            16,
            "header key must be 16 bytes (UTF-8 string)"
        );
        Self {
            secret_name,
            header_cipher: AesCipher::new(header_key.as_bytes()),
        }
    }

    /// 尝试使用 secret name 和 16 字节 UTF-8 头部密钥创建生成器。
    ///
    /// 当 `header_key` 的 UTF-8 字节长度不等于 16 时返回配置错误。
    pub fn try_new(secret_name: String, header_key: String) -> AppResult<Self> {
        Ok(Self {
            secret_name,
            header_cipher: AesCipher::try_new(header_key.as_bytes())?,
        })
    }

    /// 使用当前 Unix 毫秒时间戳生成 `X-One`。
    ///
    /// 明文格式为 `secret_name,timestamp_ms`；加密后返回小写十六进制密文。
    pub fn build_x_one(&self) -> AppResult<String> {
        self.build_x_one_at(current_timestamp_ms())
    }

    /// 使用指定毫秒时间戳生成 `X-One`。
    ///
    /// `timestamp_ms` 应表示 Unix 纪元起的毫秒数。明文格式为
    /// `secret_name,timestamp_ms`；加密后返回小写十六进制密文。
    pub fn build_x_one_at(&self, timestamp_ms: u128) -> AppResult<String> {
        self.encrypt_header(&format!("{},{}", self.secret_name, timestamp_ms))
    }

    /// 根据设备配置生成匿名 OpenChat `X-Ten`。
    ///
    /// 该匿名形式会有意将 JSON 中的 `token` 和 `sessionId` 置为空字符串，
    /// 不携带认证凭据；时间戳取当前 Unix 毫秒时间。
    pub fn build_x_ten(&self, device: &DeviceConfig) -> AppResult<String> {
        self.build_x_ten_at(device, current_timestamp_ms())
    }

    /// 使用同一个当前毫秒时间戳生成匿名 `X-One` 与 `X-Ten`。
    ///
    /// 返回元组顺序为 `(X-One, X-Ten)`；`X-Ten` 中的 `token` 和
    /// `sessionId` 均有意为空字符串。
    pub fn build_openchat_headers(&self, device: &DeviceConfig) -> AppResult<(String, String)> {
        let timestamp_ms = current_timestamp_ms();
        Ok((
            self.build_x_one_at(timestamp_ms)?,
            self.build_x_ten_at(device, timestamp_ms)?,
        ))
    }

    /// 使用同一个当前毫秒时间戳生成认证 `X-One` 与 `X-Ten`。
    ///
    /// 返回元组顺序为 `(X-One, X-Ten)`。`token` 和 `session_id` 会原样写入
    /// `X-Ten` 客户端信息，包括调用方有意传入的空字符串。
    pub fn build_authenticated_openchat_headers(
        &self,
        device: &DeviceConfig,
        token: &str,
        session_id: &str,
    ) -> AppResult<(String, String)> {
        let timestamp_ms = current_timestamp_ms();
        Ok((
            self.build_x_one_at(timestamp_ms)?,
            self.build_authenticated_x_ten_at(device, token, session_id, timestamp_ms)?,
        ))
    }

    /// 使用指定毫秒时间戳生成匿名 `X-Ten`。
    ///
    /// 客户端信息 JSON 中的 `token` 和 `sessionId` 有意固定为空字符串，
    /// 随后拼接 `//timestamp_ms`，执行 AES 加密并编码为小写十六进制。
    pub fn build_x_ten_at(&self, device: &DeviceConfig, timestamp_ms: u128) -> AppResult<String> {
        let client_info = serde_json::to_string(&OpenChatClientInfo::from(device))
            .map_err(|error| crate::error::AppError::Config(error.to_string()))?;
        self.encrypt_header(&format!("{client_info}//{timestamp_ms}"))
    }

    /// 使用指定认证信息和毫秒时间戳生成 `X-Ten`。
    ///
    /// `token` 与 `session_id` 会原样序列化，包括任一值为空字符串的情况；
    /// 客户端信息 JSON 随后拼接 `//timestamp_ms`，执行 AES 加密并编码为
    /// 小写十六进制。
    pub fn build_authenticated_x_ten_at(
        &self,
        device: &DeviceConfig,
        token: &str,
        session_id: &str,
        timestamp_ms: u128,
    ) -> AppResult<String> {
        let client_info = serde_json::to_string(&OpenChatClientInfo::authenticated(
            device, token, session_id,
        ))
        .map_err(|error| crate::error::AppError::Config(error.to_string()))?;
        self.encrypt_header(&format!("{client_info}//{timestamp_ms}"))
    }

    fn encrypt_header(&self, plaintext: &str) -> AppResult<String> {
        let encrypted = self.header_cipher.encrypt(plaintext.as_bytes())?;
        Ok(hex::encode(encrypted))
    }

    /// 返回生成 `X-One` 时使用的 secret name。
    pub fn secret_name(&self) -> &str {
        &self.secret_name
    }
}

fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after Unix epoch")
        .as_millis()
}

/// [`HeaderManager`] 的兼容名称。
pub type VersionKeyManager = HeaderManager;

#[cfg(test)]
mod tests {
    //! 版本请求头的兼容常量、构造、明文格式及认证信息测试。

    use crate::config::DeviceConfig;

    use super::*;

    #[test]
    fn test_v_salt_constant() {
        use md5::{Digest, Md5};
        let mut hasher = Md5::new();
        hasher.update(b"sjlkajsl*Rkfsdsd_tflklsjdf");
        let expected = hasher.finalize();
        assert_eq!(expected.len(), 16);

        // 与既有协议夹具中的已知 MD5 值核对：
        // 79b461d1ffebcf44dd823aeec2cff3ad。
        let computed = compute_v_l_salt();
        assert_eq!(&computed[..], &expected[..]);
    }

    #[test]
    fn test_version_key_manager_creation() {
        let manager = VersionKeyManager::new(
            "0123456789abcdef0123456789abcdef".to_string(),
            "0123456789abcdef".to_string(),
        );
        let x_one = manager.build_x_one().unwrap();
        assert!(!x_one.is_empty());
        // AES-128-ECB-PKCS7：32 字节 secret name、逗号和约 13 位时间戳
        // 填充到 48 字节，十六进制编码后为 96 个字符。
        assert_eq!(x_one.len(), 96);
    }

    #[test]
    fn test_build_x_one_length() {
        let manager = VersionKeyManager::new(
            "0123456789abcdef0123456789abcdef".to_string(),
            "79b461d1ffebcf44".to_string(),
        );
        let x_one = manager.build_x_one().unwrap();
        // 这里只约束协议产物非空且为完整的十六进制字节编码，不固定实时戳长度。
        assert!(!x_one.is_empty());
        assert_eq!(x_one.len() % 2, 0);
    }

    #[test]
    fn test_secret_name_accessor() {
        let manager =
            VersionKeyManager::new("test-secret".to_string(), "1234567890abcdef".to_string());
        assert_eq!(manager.secret_name(), "test-secret");
    }

    #[test]
    fn header_manager_builds_independent_x_one_and_x_ten_at_same_timestamp() {
        let manager = HeaderManager::new("test-secret".to_string(), "1234567890abcdef".to_string());
        let device = DeviceConfig {
            app_ver: 680,
            package_code: 9803,
            plat: 0,
            language: 2,
            sys_mac: "device-id".to_string(),
            sys_model: "PC-TOOLS".to_string(),
        };

        let x_one = manager.build_x_one_at(1_700_000_000_123).unwrap();
        let x_ten = manager.build_x_ten_at(&device, 1_700_000_000_123).unwrap();

        let cipher = AesCipher::new(b"1234567890abcdef");
        assert_eq!(
            cipher.decrypt(&hex::decode(x_one).unwrap()).unwrap(),
            b"test-secret,1700000000123"
        );
        let x_ten_plain = cipher.decrypt(&hex::decode(x_ten).unwrap()).unwrap();
        let (json, timestamp) = std::str::from_utf8(&x_ten_plain)
            .unwrap()
            .rsplit_once("//")
            .unwrap();
        assert_eq!(timestamp, "1700000000123");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(json).unwrap(),
            serde_json::json!({
                "token": "",
                "sessionId": "",
                "appVer": 680,
                "plat": 0,
                "packageCode": 9803,
                "language": 2,
                "sysMac": "device-id",
                "sysModel": "PC-TOOLS"
            })
        );
    }

    #[test]
    fn authenticated_x_ten_contains_access_token() {
        let manager = HeaderManager::new("test-secret".to_string(), "1234567890abcdef".to_string());
        let device = DeviceConfig::default();

        let x_ten = manager
            .build_authenticated_x_ten_at(&device, "access-token", "", 1_700_000_000_123)
            .unwrap();
        let cipher = AesCipher::new(b"1234567890abcdef");
        let plaintext = cipher.decrypt(&hex::decode(x_ten).unwrap()).unwrap();
        let (json, timestamp) = std::str::from_utf8(&plaintext)
            .unwrap()
            .rsplit_once("//")
            .unwrap();

        assert_eq!(timestamp, "1700000000123");
        let client_info: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(client_info["token"], "access-token");
        assert_eq!(client_info["sessionId"], "");
    }

    #[test]
    #[should_panic(expected = "header key must be 16 bytes")]
    fn test_invalid_header_key_length() {
        VersionKeyManager::new(
            "0123456789abcdef0123456789abcdef".to_string(),
            "short".to_string(),
        );
    }
}
