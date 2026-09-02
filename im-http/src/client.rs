use im_common::aes::AesCipher;
use im_common::error::AppError;
use im_common::tcp_head::TcpFrameHeader;

/// Encrypt JSON bytes and wrap in gateway request body format:
/// [0xC0, 0x80] + AES-encrypted content (no length prefix for gateway requests)
pub fn build_gateway_request_body(cipher: &AesCipher, json_bytes: &[u8]) -> Vec<u8> {
    let encrypted = cipher.encrypt(json_bytes).unwrap();
    let mut body = TcpFrameHeader::build(true, false).to_vec();
    body.extend_from_slice(&encrypted);
    body
}

/// Decrypt and optionally decompress a gateway response.
/// Response format: [0xC0, head][4B length(big-endian)][AES-encrypted possibly-gzipped content]
pub fn parse_gateway_response(
    cipher: &AesCipher,
    data: &[u8],
) -> Result<Vec<u8>, AppError> {
    if data.len() < 6 {
        return Err(AppError::TcpFrame("response too short".to_string()));
    }

    let head = [data[0], data[1]];
    let frame = TcpFrameHeader::parse(head);
    let len = u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize;

    if data.len() < 6 + len {
        return Err(AppError::TcpFrame("response truncated".to_string()));
    }

    let encrypted = &data[6..6 + len];
    let mut decrypted = cipher.decrypt(encrypted)?;

    // If compressed, decompress with gzip
    if frame.zipped {
        let mut decoded = flate2::read::GzDecoder::new(&decrypted[..]);
        let mut decompressed = Vec::new();
        use std::io::Read;
        decoded.read_to_end(&mut decompressed).map_err(|e| {
            AppError::TcpFrame(format!("gzip decompress failed: {}", e))
        })?;
        decrypted = decompressed;
    }

    Ok(decrypted)
}

/// Build an im-biz request body: [2B head][4B length(big-endian)][AES-encrypted content]
pub fn build_im_biz_request_body(cipher: &AesCipher, content: &[u8]) -> Vec<u8> {
    let encrypted = cipher.encrypt(content).unwrap();
    let mut body = TcpFrameHeader::build(true, false).to_vec();
    // Write 4B big-endian length of encrypted content
    let len_bytes = (encrypted.len() as u32).to_be_bytes();
    body.extend_from_slice(&len_bytes);
    body.extend_from_slice(&encrypted);
    body
}

/// Decrypt and optionally decompress an im-biz response.
/// Response format: [2B head][4B length][AES-encrypted possibly-gzipped content]
pub fn parse_im_biz_response(cipher: &AesCipher, data: &[u8]) -> Result<Vec<u8>, AppError> {
    if data.len() < 6 {
        return Err(AppError::TcpFrame("response too short".to_string()));
    }

    let head = [data[0], data[1]];
    let frame = TcpFrameHeader::parse(head);
    let len = u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize;

    if data.len() < 6 + len {
        return Err(AppError::TcpFrame("response truncated".to_string()));
    }

    let encrypted = &data[6..6 + len];
    let mut decrypted = cipher.decrypt(encrypted)?;

    if frame.zipped {
        let mut decoded = flate2::read::GzDecoder::new(&decrypted[..]);
        let mut decompressed = Vec::new();
        use std::io::Read;
        decoded.read_to_end(&mut decompressed).map_err(|e| {
            AppError::TcpFrame(format!("gzip decompress failed: {}", e))
        })?;
        decrypted = decompressed;
    }

    Ok(decrypted)
}
