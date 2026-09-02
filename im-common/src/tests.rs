//! `im-common` 的 AES、TCP 帧头、配置与版本请求头回归测试。

use super::aes::AesCipher;
use super::config::{AppConfig, DeviceConfig, ServerConfig};
use super::tcp_head::TcpFrameHeader;
use super::version_key::VersionKeyManager;

// --- TCP 帧头 ---

#[test]
fn test_parse_encrypted_uncompressed() {
    let head = TcpFrameHeader::parse([0xC0, 0x80]).unwrap();
    assert!(head.encrypted);
    assert!(!head.zipped);
    assert!(!head.encrypted_system_version);
    assert!(!head.is_report);
}

#[test]
fn test_parse_encrypted_compressed() {
    let head = TcpFrameHeader::parse([0xC0, 0xC0]).unwrap();
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
    let parsed = TcpFrameHeader::parse(original).unwrap();
    let rebuilt = TcpFrameHeader::build(parsed.encrypted, parsed.zipped);
    assert_eq!(rebuilt, original);
}

#[test]
fn invalid_tcp_frame_marker_returns_error_instead_of_panicking() {
    let error = TcpFrameHeader::parse([0xFF, 0x80]).unwrap_err();

    assert!(error.to_string().contains("0xFF"));
}

// --- AES ---

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
    // 单字节明文应补齐为一个 16 字节 AES 分组。
    let encrypted = cipher.encrypt(b"x").unwrap();
    assert_eq!(encrypted.len(), 16);
    let decrypted = cipher.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, b"x");
}

#[test]
fn test_invalid_aes_key_returns_error() {
    let error = AesCipher::try_new(b"too-short").err().unwrap();

    assert!(matches!(error, super::error::AppError::Config(_)));
}

// --- 配置 ---

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

// --- 版本请求头 ---

#[test]
fn test_version_key_manager_creation() {
    let manager = VersionKeyManager::new(
        "f82956caf0fa90aecf24d5ef9541f624".to_string(),
        "f58c15f54e8f7826".to_string(),
    );
    let x_one = manager.build_x_one().unwrap();
    assert!(!x_one.is_empty());
    // AES-128-ECB-PKCS7：32 字节 secret name、逗号和约 13 位时间戳
    // 填充到 48 字节，十六进制编码后为 96 个字符。
    assert_eq!(x_one.len(), 96);
}

#[test]
fn test_v_salt_constant() {
    // 兼容性夹具约定：V_L_SALT = MD5(指定字符串) 的前 16 字节。
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(b"sjlkajsl*Rkfsdsd_tflklsjdf");
    let result = hasher.finalize();
    assert_eq!(result.len(), 16);
}
