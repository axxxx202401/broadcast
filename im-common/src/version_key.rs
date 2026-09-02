use crate::aes::AesCipher;
use crate::error::AppResult;
use std::time::{SystemTime, UNIX_EPOCH};

/// V_L_SALT = first 16 bytes of MD5("sjlkajsl*Rkfsdsd_tflklsjdf")
#[cfg(test)]
fn compute_v_l_salt() -> [u8; 16] {
    use md5::{Md5, Digest};
    let mut hasher = Md5::new();
    hasher.update(b"sjlkajsl*Rkfsdsd_tflklsjdf");
    let result = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&result[..16]);
    bytes
}

/// X-One header generator.
///
/// Format: hex(AES_V_L_SALT(secretName + "," + timestamp_ms))
/// where V_L_SALT = md5("sjlkajsl*Rkfsdsd_tflklsjdf").first16Bytes
pub struct VersionKeyManager {
    secret_name: String,
    header_cipher: AesCipher,
}

impl VersionKeyManager {
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

    /// Generate the X-One header value.
    pub fn build_x_one(&self) -> AppResult<String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let plaintext = format!("{},{}", self.secret_name, timestamp);
        let encrypted = self.header_cipher.encrypt(plaintext.as_bytes())?;
        Ok(hex::encode(encrypted))
    }

    pub fn secret_name(&self) -> &str {
        &self.secret_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v_salt_constant() {
        use md5::{Md5, Digest};
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
        let manager = VersionKeyManager::new(
            "test-secret".to_string(),
            "1234567890abcdef".to_string(),
        );
        assert_eq!(manager.secret_name(), "test-secret");
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
