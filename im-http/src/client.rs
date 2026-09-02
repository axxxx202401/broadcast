use std::io::{Read, Write};

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use im_common::aes::AesCipher;
use im_common::error::AppError;
use im_common::tcp_head::TcpFrameHeader;
pub use im_common::{MAX_DECOMPRESSED_BODY_SIZE, MAX_FRAME_BODY_SIZE};

const JAVA_GZIP_THRESHOLD: usize = 5 * 1024;
const GATEWAY_REQUEST_MARKER: u8 = 0xC0;
const IM_BIZ_REQUEST_MARKER: u8 = 0xC1;

/// Encrypt JSON bytes and wrap in gateway request body format:
/// [2B head][4B big-endian length][AES-encrypted, optionally gzipped content].
///
/// Java enables gzip only when the encrypted payload exceeds 5 KiB.
pub fn build_gateway_request_body(
    cipher: &AesCipher,
    json_bytes: &[u8],
) -> Result<Vec<u8>, AppError> {
    validate_plain_request_size(json_bytes)?;
    let encrypted = cipher.encrypt(json_bytes)?;
    let zipped = encrypted.len() > JAVA_GZIP_THRESHOLD;
    let content = if zipped { gzip(&encrypted)? } else { encrypted };
    build_length_framed_request(GATEWAY_REQUEST_MARKER, true, zipped, content)
}

pub fn build_gateway_request_body_with_options(
    cipher: &AesCipher,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Result<Vec<u8>, AppError> {
    let content = encode_request_content(cipher, content, encrypted, zipped)?;
    build_length_framed_request(GATEWAY_REQUEST_MARKER, encrypted, zipped, content)
}

/// Decrypt and optionally decompress a gateway response.
/// Response format: [0xC0, head][4B length(big-endian)][AES-encrypted possibly-gzipped content]
pub fn parse_gateway_response(cipher: &AesCipher, data: &[u8]) -> Result<Vec<u8>, AppError> {
    parse_length_framed_response(cipher, data)
}

/// Build an im-biz request body: [2B head][4B length(big-endian)][AES-encrypted content]
pub fn build_im_biz_request_body(cipher: &AesCipher, content: &[u8]) -> Result<Vec<u8>, AppError> {
    build_im_biz_request_body_with_options(cipher, content, true, false)
}

pub fn build_im_biz_request_body_with_options(
    cipher: &AesCipher,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Result<Vec<u8>, AppError> {
    let content = encode_request_content(cipher, content, encrypted, zipped)?;
    build_length_framed_request(IM_BIZ_REQUEST_MARKER, encrypted, zipped, content)
}

fn build_length_framed_request(
    marker: u8,
    encrypted: bool,
    zipped: bool,
    content: Vec<u8>,
) -> Result<Vec<u8>, AppError> {
    validate_encoded_request_size(&content)?;
    let mut head = TcpFrameHeader::build(encrypted, zipped);
    head[0] = marker;
    let mut body = head.to_vec();
    let content_len = u32::try_from(content.len())
        .map_err(|_| AppError::TcpFrame("request body length exceeds u32".to_string()))?;
    body.extend_from_slice(&content_len.to_be_bytes());
    body.extend_from_slice(&content);
    Ok(body)
}

fn encode_request_content(
    cipher: &AesCipher,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Result<Vec<u8>, AppError> {
    validate_plain_request_size(content)?;
    let mut content = content.to_vec();

    if encrypted {
        content = cipher.encrypt(&content)?;
    }

    if zipped {
        content = gzip(&content)?;
    }

    validate_encoded_request_size(&content)?;
    Ok(content)
}

fn validate_plain_request_size(content: &[u8]) -> Result<(), AppError> {
    if content.len() > MAX_DECOMPRESSED_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "request body length {} exceeds limit {}",
            content.len(),
            MAX_DECOMPRESSED_BODY_SIZE
        )));
    }
    Ok(())
}

fn validate_encoded_request_size(content: &[u8]) -> Result<(), AppError> {
    if content.len() > MAX_FRAME_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "encoded request body length {} exceeds limit {}",
            content.len(),
            MAX_FRAME_BODY_SIZE
        )));
    }
    Ok(())
}

fn gzip(content: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content)?;
    Ok(encoder.finish()?)
}

/// Decrypt and optionally decompress an im-biz response.
/// Response format: [2B head][4B length][AES-encrypted possibly-gzipped content]
pub fn parse_im_biz_response(cipher: &AesCipher, data: &[u8]) -> Result<Vec<u8>, AppError> {
    parse_length_framed_response(cipher, data)
}

fn parse_length_framed_response(cipher: &AesCipher, data: &[u8]) -> Result<Vec<u8>, AppError> {
    if data.len() < 6 {
        return Err(AppError::TcpFrame("response too short".to_string()));
    }

    if data[0] & 0xC0 != 0xC0 {
        return Err(AppError::TcpFrame(format!(
            "invalid response marker: 0x{:02X}",
            data[0]
        )));
    }

    let frame = TcpFrameHeader::parse([0xC0, data[1]])?;
    let len = u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize;
    if len > MAX_FRAME_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "declared response body length {} exceeds limit {}",
            len, MAX_FRAME_BODY_SIZE
        )));
    }

    let wire_len = 6usize
        .checked_add(len)
        .ok_or_else(|| AppError::TcpFrame("response length overflow".to_string()))?;
    if data.len() < wire_len {
        return Err(AppError::TcpFrame("response truncated".to_string()));
    }

    let mut content = data[6..wire_len].to_vec();

    if frame.zipped {
        content = decompress_limited(&content)?;
    }

    if frame.encrypted {
        content = cipher.decrypt(&content)?;
    }

    Ok(content)
}

fn decompress_limited(content: &[u8]) -> Result<Vec<u8>, AppError> {
    let decoder = GzDecoder::new(content);
    let mut limited = decoder.take((MAX_DECOMPRESSED_BODY_SIZE as u64) + 1);
    let mut decompressed = Vec::new();
    limited
        .read_to_end(&mut decompressed)
        .map_err(|error| AppError::TcpFrame(format!("gzip decompress failed: {}", error)))?;
    if decompressed.len() > MAX_DECOMPRESSED_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "decompressed response body exceeds limit {}",
            MAX_DECOMPRESSED_BODY_SIZE
        )));
    }
    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use flate2::{write::GzEncoder, Compression};
    use im_common::tcp_head::TcpFrameHeader;

    use super::*;

    const KEY: &[u8] = b"97b1f52761ffc7f8";

    fn length_framed_response(encrypted: bool, zipped: bool, content: &[u8]) -> Vec<u8> {
        let mut response = TcpFrameHeader::build(encrypted, zipped).to_vec();
        response.extend_from_slice(&(content.len() as u32).to_be_bytes());
        response.extend_from_slice(content);
        response
    }

    #[test]
    fn gateway_request_is_length_framed_and_encrypts_body() {
        let cipher = AesCipher::new(KEY);
        let plaintext = br#"{"phone":"123"}"#;

        let body = build_gateway_request_body(&cipher, plaintext).unwrap();

        assert_eq!(&body[..2], &[0xC0, 0x80]);
        let declared_len = u32::from_be_bytes(body[2..6].try_into().unwrap()) as usize;
        assert_eq!(declared_len, body.len() - 6);
        assert_ne!(&body[6..], plaintext);
        assert_eq!(cipher.decrypt(&body[6..]).unwrap(), plaintext);
    }

    #[test]
    fn gateway_request_encrypts_then_compresses() {
        let cipher = AesCipher::new(KEY);
        let plaintext = br#"{"payload":"repeat repeat repeat repeat"}"#;

        let body = build_gateway_request_body_with_options(&cipher, plaintext, true, true).unwrap();

        assert_eq!(&body[..2], &[0xC0, 0xC0]);
        let declared_len = u32::from_be_bytes(body[2..6].try_into().unwrap()) as usize;
        assert_eq!(declared_len, body.len() - 6);
        let mut decoder = flate2::read::GzDecoder::new(&body[6..]);
        let mut encrypted = Vec::new();
        decoder.read_to_end(&mut encrypted).unwrap();
        assert_eq!(cipher.decrypt(&encrypted).unwrap(), plaintext);
    }

    #[test]
    fn gateway_request_auto_compresses_only_above_java_encrypted_threshold() {
        let cipher = AesCipher::new(KEY);

        let below = build_gateway_request_body(&cipher, &vec![0u8; 5104]).unwrap();
        let above = build_gateway_request_body(&cipher, &vec![0u8; 5120]).unwrap();

        assert_eq!(&below[..2], &[0xC0, 0x80]);
        assert_eq!(&above[..2], &[0xC0, 0xC0]);
    }

    #[test]
    fn gateway_response_honors_unencrypted_header() {
        let cipher = AesCipher::new(KEY);
        let plaintext = br#"{"success":true}"#;
        let response = length_framed_response(false, false, plaintext);

        let parsed = parse_gateway_response(&cipher, &response).unwrap();

        assert_eq!(parsed, plaintext);
    }

    #[test]
    fn gateway_response_decompresses_then_decrypts() {
        let cipher = AesCipher::new(KEY);
        let plaintext = br#"{"payload":"compress me compress me compress me"}"#;
        let encrypted = cipher.encrypt(plaintext).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&encrypted).unwrap();
        let compressed = encoder.finish().unwrap();
        let response = length_framed_response(true, true, &compressed);

        let parsed = parse_gateway_response(&cipher, &response).unwrap();

        assert_eq!(parsed, plaintext);
    }

    #[test]
    fn im_biz_request_length_covers_encrypted_payload() {
        let cipher = AesCipher::new(KEY);
        let plaintext = b"protobuf request";

        let body = build_im_biz_request_body(&cipher, plaintext).unwrap();

        assert_eq!(&body[..2], &[0xC1, 0x80]);
        let declared_len = u32::from_be_bytes(body[2..6].try_into().unwrap()) as usize;
        assert_eq!(declared_len, body.len() - 6);
        assert_eq!(cipher.decrypt(&body[6..]).unwrap(), plaintext);
    }

    #[test]
    fn im_biz_request_can_compress_without_encrypting() {
        let cipher = AesCipher::new(KEY);
        let plaintext = b"protobuf request protobuf request";

        let body = build_im_biz_request_body_with_options(&cipher, plaintext, false, true).unwrap();

        assert_eq!(&body[..2], &[0xC1, 0x40]);
        let declared_len = u32::from_be_bytes(body[2..6].try_into().unwrap()) as usize;
        assert_eq!(declared_len, body.len() - 6);
        let mut decoder = flate2::read::GzDecoder::new(&body[6..]);
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, plaintext);
    }

    #[test]
    fn im_biz_response_decompresses_then_decrypts() {
        let cipher = AesCipher::new(KEY);
        let plaintext = b"protobuf response protobuf response";
        let encrypted = cipher.encrypt(plaintext).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&encrypted).unwrap();
        let compressed = encoder.finish().unwrap();
        let response = length_framed_response(true, true, &compressed);

        let parsed = parse_im_biz_response(&cipher, &response).unwrap();

        assert_eq!(parsed, plaintext);
    }

    #[test]
    fn response_accepts_protocol_version_in_first_head_byte() {
        let cipher = AesCipher::new(KEY);
        let plaintext = b"versioned response";
        let encrypted = cipher.encrypt(plaintext).unwrap();
        let mut response = length_framed_response(true, false, &encrypted);
        response[0] = 0xC1;

        let parsed = parse_im_biz_response(&cipher, &response).unwrap();

        assert_eq!(parsed, plaintext);
    }

    #[test]
    fn im_biz_response_honors_compressed_unencrypted_header() {
        let cipher = AesCipher::new(KEY);
        let plaintext = b"protobuf response protobuf response";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(plaintext).unwrap();
        let compressed = encoder.finish().unwrap();
        let response = length_framed_response(false, true, &compressed);

        let parsed = parse_im_biz_response(&cipher, &response).unwrap();

        assert_eq!(parsed, plaintext);
    }

    #[test]
    fn request_body_over_limit_is_rejected() {
        let cipher = AesCipher::new(KEY);
        let oversized = vec![0u8; MAX_FRAME_BODY_SIZE + 1];

        let error =
            build_gateway_request_body_with_options(&cipher, &oversized, false, false).unwrap_err();

        assert!(matches!(error, AppError::TcpFrame(_)));
    }

    #[test]
    fn oversized_declared_response_is_rejected_before_waiting_for_body() {
        let cipher = AesCipher::new(KEY);
        let mut response = TcpFrameHeader::build(false, false).to_vec();
        response.extend_from_slice(&((MAX_FRAME_BODY_SIZE + 1) as u32).to_be_bytes());

        let error = parse_gateway_response(&cipher, &response).unwrap_err();

        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn gzip_response_over_decompressed_limit_is_rejected() {
        let cipher = AesCipher::new(KEY);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        let chunk = [0u8; 8192];
        let mut remaining = MAX_DECOMPRESSED_BODY_SIZE + 1;
        while remaining > 0 {
            let count = remaining.min(chunk.len());
            encoder.write_all(&chunk[..count]).unwrap();
            remaining -= count;
        }
        let compressed = encoder.finish().unwrap();
        let response = length_framed_response(false, true, &compressed);

        let error = parse_gateway_response(&cipher, &response).unwrap_err();

        assert!(error.to_string().contains("decompressed"));
    }
}
