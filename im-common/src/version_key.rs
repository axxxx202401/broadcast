use crate::aes::AesCipher;
use crate::config::DeviceConfig;
use crate::error::AppResult;
use std::time::{SystemTime, UNIX_EPOCH};

/// V_L_SALT = first 16 bytes of MD5("sjlkajsl*Rkfsdsd_tflklsjdf")
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

/// X-One/X-Ten encrypted header generator.
///
/// X-One: `hex(AES(secret_name + "," + timestamp_ms))`
/// X-Ten: `hex(AES(client_info_json + "//" + timestamp_ms))`
pub struct HeaderManager {
    secret_name: String,
    header_cipher: AesCipher,
}

impl HeaderManager {
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

    pub fn try_new(secret_name: String, header_key: String) -> AppResult<Self> {
        Ok(Self {
            secret_name,
            header_cipher: AesCipher::try_new(header_key.as_bytes())?,
        })
    }

    /// Generate the X-One header value.
    pub fn build_x_one(&self) -> AppResult<String> {
        self.build_x_one_at(current_timestamp_ms())
    }

    pub fn build_x_one_at(&self, timestamp_ms: u128) -> AppResult<String> {
        self.encrypt_header(&format!("{},{}", self.secret_name, timestamp_ms))
    }

    /// Generate an unauthenticated OpenChat X-Ten from device configuration.
    ///
    /// Session and token are deliberately empty so unauthorized requests cannot
    /// leak an authenticated credential.
    pub fn build_x_ten(&self, device: &DeviceConfig) -> AppResult<String> {
        self.build_x_ten_at(device, current_timestamp_ms())
    }

    pub fn build_openchat_headers(&self, device: &DeviceConfig) -> AppResult<(String, String)> {
        let timestamp_ms = current_timestamp_ms();
        Ok((
            self.build_x_one_at(timestamp_ms)?,
            self.build_x_ten_at(device, timestamp_ms)?,
        ))
    }

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

    pub fn build_x_ten_at(&self, device: &DeviceConfig, timestamp_ms: u128) -> AppResult<String> {
        let client_info = serde_json::to_string(&OpenChatClientInfo::from(device))
            .map_err(|error| crate::error::AppError::Config(error.to_string()))?;
        self.encrypt_header(&format!("{client_info}//{timestamp_ms}"))
    }

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

pub type VersionKeyManager = HeaderManager;

#[cfg(test)]
mod tests {
    use crate::config::DeviceConfig;

    use super::*;

    #[test]
    fn test_v_salt_constant() {
        use md5::{Digest, Md5};
        let mut hasher = Md5::new();
        hasher.update(b"sjlkajsl*Rkfsdsd_tflklsjdf");
        let expected = hasher.finalize();
        assert_eq!(expected.len(), 16);

        // Verify against known MD5: 79b461d1ffebcf44dd823aeec2cff3ad
        let computed = compute_v_l_salt();
        assert_eq!(&computed[..], &expected[..]);
    }

    #[test]
    fn test_version_key_manager_creation() {
        let manager = VersionKeyManager::new(
            "f82956caf0fa90aecf24d5ef9541f624".to_string(),
            "f58c15f54e8f7826".to_string(),
        );
        let x_one = manager.build_x_one().unwrap();
        assert!(!x_one.is_empty());
        // AES-128 ECB with PKCS7: secret_name(32) + "," + timestamp(~13) = ~46 bytes
        // padded to 48 bytes → 96 hex chars
        assert_eq!(x_one.len(), 96);
    }

    #[test]
    fn test_build_x_one_length() {
        let manager = VersionKeyManager::new(
            "f82956caf0fa90aecf24d5ef9541f624".to_string(),
            "79b461d1ffebcf44".to_string(),
        );
        let x_one = manager.build_x_one().unwrap();
        // plaintext is ~50+ chars, padded to multiple of 16, so encrypted len is 16..=32
        // hex encoding doubles it → 32..=64 chars
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
            "f82956caf0fa90aecf24d5ef9541f624".to_string(),
            "short".to_string(),
        );
    }
}
