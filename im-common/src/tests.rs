use super::aes::AesCipher;

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
