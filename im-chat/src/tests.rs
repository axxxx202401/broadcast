use im_common::{config::AppConfig, aes::AesCipher};
use super::frame::{decode_frame, encode_frame};

#[test]
fn test_encode_and_decode_frame() {
    let content = b"test protobuf data";
    let framed = encode_frame(1000, content, false, false);

    // Frame should start with [0xC0, 0x00] (encrypted=false, zipped=false)
    assert_eq!(&framed[0..2], &[0xC0, 0x00]);

    let (msg_id, body) = decode_frame(&framed).unwrap();
    assert_eq!(msg_id, 1000);
    assert_eq!(body, content);
}

#[test]
fn test_encode_frame_big_endian() {
    let content = b"hello";
    let framed = encode_frame(0x0102, content, false, false);

    // messageId 0x0102 should be big-endian: [0x01, 0x02]
    assert_eq!(&framed[2..4], &[0x01, 0x02]);

    // contentLength 5 should be big-endian: [0x00, 0x00, 0x00, 0x05]
    assert_eq!(&framed[4..8], &[0x00, 0x00, 0x00, 0x05]);
}

#[test]
fn test_decode_invalid_frame() {
    let invalid = vec![0xFF, 0xFF, 0xFF];
    assert!(decode_frame(&invalid).is_err());
}

// --- Integration tests ---

#[test]
fn test_full_frame_workflow() {
    let config = AppConfig::default();
    let key = AesCipher::new(config.server.body_aes_key.as_bytes());

    // 1. 加密数据
    let plaintext = b"test protobuf content";
    let encrypted = key.encrypt(plaintext).unwrap();

    // 2. 编码帧
    let frame = encode_frame(2202, &encrypted, true, false);

    // 3. 解码帧
    let (msg_id, decrypted) = decode_frame(&frame).unwrap();
    assert_eq!(msg_id, 2202);

    // 4. 解密
    let result = key.decrypt(&decrypted).unwrap();
    assert_eq!(result, plaintext);
}

#[test]
fn test_version_key_generation() {
    use im_common::version_key::VersionKeyManager;
    let manager = VersionKeyManager::new(
        "f82956caf0fa90aecf24d5ef9541f624".to_string(),
        "f58c15f54e8f7826".to_string(),
    );
    let x_one = manager.build_x_one().unwrap();
    assert_eq!(x_one.len(), 96);
    // 验证 hex 解码成功
    hex::decode(&x_one).unwrap();
}
