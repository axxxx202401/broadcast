use super::aes::AesCipher;
use super::tcp_head::TcpFrameHeader;

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
