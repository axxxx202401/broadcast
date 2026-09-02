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
