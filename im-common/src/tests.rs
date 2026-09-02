use super::aes::AesCipher;
use super::config::{AppConfig, DeviceConfig, ServerConfig};
use super::tcp_head::TcpFrameHeader;
use super::version_key::VersionKeyManager;

#[test]
fn test_parse_encrypted_uncompressed() {
    let head = TcpFrameHeader::parse([0xC0, 0x80]);
    assert!(head.encrypted);
    assert!(!head.zipped);
    assert!(!head.encrypted_system_version);
    assert!(!head.is_report);
}

#[test]
fn test_parse_encrypted_compressed() {
    let head = TcpFrameHeader::parse([0xC0, 0xC0]);
    assert!(head.encrypted);
    assert!(head.zipped);
}

#[test]
fn test_build_encrypted_uncompressed() {
    let result = TcpFrameHeader::build(true, false);
    assert_eq!(result, [0xC0, 0x80]);
}

#[test]
fn test_build_encrypted_compressed() {
    let result = TcpFrameHeader::build(true, true);
    assert_eq!(result, [0xC0, 0xC0]);
}

#[test]
fn test_roundtrip() {
    let original = [0xC0, 0x80];
    let parsed = TcpFrameHeader::parse(original);
    let rebuilt = TcpFrameHeader::build(parsed.encrypted, parsed.zipped);
    assert_eq!(rebuilt, original);
}

#[test]
fn test_aes_encrypt_decrypt() {
    let key = b"97b1f52761ffc7f8";
    let cipher = AesCipher::new(key);
    let plaintext = b"hello world";
    let encrypted = cipher.encrypt(plaintext).unwrap();
    let decrypted = cipher.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, b"hello world");
}

#[test]
fn test_aes_pkcs7_padding() {
    let key = b"97b1f52761ffc7f8";
    let cipher = AesCipher::new(key);
    // 1 byte input -> should be padded to 16 bytes
    let encrypted = cipher.encrypt(b"x").unwrap();
    assert_eq!(encrypted.len(), 16);
    let decrypted = cipher.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, b"x");
}

// --- config tests ---

#[test]
fn test_default_server_config_values() {
    let s = ServerConfig::default();
    assert_eq!(s.openchat_user_url, "https://test-ochat-user1.68chat.co");
    assert_eq!(s.im_biz_url, "https://test-biz-b.68chat.co");
    assert_eq!(s.im_chat_host, "35.220.159.225");
    assert_eq!(s.im_chat_port, 9500);
    assert_eq!(s.version_secret_name, "f82956caf0fa90aecf24d5ef9541f624");
    assert_eq!(s.body_aes_key, "97b1f52761ffc7f8");
    assert_eq!(s.header_aes_key, "f58c15f54e8f7826");
}

#[test]
fn test_device_config_defaults() {
    let d = DeviceConfig::default();
    assert_eq!(d.app_ver, 680);
    assert_eq!(d.package_code, 9803);
    assert_eq!(d.plat, 0);
    assert_eq!(d.language, 2);
    assert_eq!(d.sys_model, "PC-TOOLS");
    assert!(!d.sys_mac.is_empty());
}

#[test]
fn test_device_config_new() {
    let d = DeviceConfig::new();
    assert_eq!(d.app_ver, 680);
    assert_eq!(d.sys_model, "PC-TOOLS");
}

#[test]
fn test_app_config_defaults() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.server.im_chat_port, 9500);
    assert_eq!(cfg.device.app_ver, 680);
    assert_eq!(cfg.device.package_code, 9803);
}

#[test]
fn test_app_config_clone_and_debug() {
    let cfg = AppConfig::default();
    let cloned = cfg.clone();
    assert_eq!(cloned.server.im_chat_host, cfg.server.im_chat_host);
    assert_eq!(cloned.device.app_ver, cfg.device.app_ver);
    let _ = format!("{:?}", cfg);
}

#[test]
fn test_server_config_clone_and_debug() {
    let s1 = ServerConfig::default();
    let s2 = s1.clone();
    assert_eq!(s1.openchat_user_url, s2.openchat_user_url);
    let _ = format!("{:?}", s1);
}

// --- version_key tests ---

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
fn test_v_salt_constant() {
    // V_L_SALT = md5("sjlkajsl*Rkfsdsd_tflklsjdf")[0..16]
    use md5::{Md5, Digest};
    let mut hasher = Md5::new();
    hasher.update(b"sjlkajsl*Rkfsdsd_tflklsjdf");
    let result = hasher.finalize();
    assert_eq!(result.len(), 16);
}

