use std::io::{Read, Write};

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use im_common::aes::AesCipher;
use im_common::error::{AppError, AppResult};
use im_common::tcp_head::TcpFrameHeader;
pub use im_common::{MAX_DECOMPRESSED_BODY_SIZE, MAX_FRAME_BODY_SIZE};

pub const PRE_SESSION_AES_KEY: &str = "1234560000000000";
const SERVER_ERROR_MESSAGE_ID: u16 = 9999;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub header: TcpFrameHeader,
    pub message_id: u16,
    pub content: Vec<u8>,
    pub wire_len: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameDecodeError {
    #[error("incomplete frame: need {needed} bytes, have {available}")]
    Incomplete { needed: usize, available: usize },
    #[error("invalid frame: {0}")]
    Invalid(#[from] AppError),
}

/// Encode a TCP frame.
/// Wire format: [head(2)][messageId(2,BE)][contentLength(4,BE)][content]
pub fn encode_frame(
    message_id: u16,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> AppResult<Vec<u8>> {
    encode_frame_with_header(
        message_id,
        content,
        TcpFrameHeader::build(encrypted, zipped),
    )
}

pub(crate) fn encode_frame_with_header(
    message_id: u16,
    content: &[u8],
    head: [u8; 2],
) -> AppResult<Vec<u8>> {
    if content.len() > MAX_FRAME_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "frame body length {} exceeds limit {}",
            content.len(),
            MAX_FRAME_BODY_SIZE
        )));
    }
    let content_len = u32::try_from(content.len())
        .map_err(|_| AppError::TcpFrame("frame body length exceeds u32".to_string()))?;

    let mut buf = Vec::with_capacity(2 + 2 + 4 + content.len());
    buf.extend_from_slice(&head);
    buf.extend_from_slice(&message_id.to_be_bytes());
    buf.extend_from_slice(&content_len.to_be_bytes());
    buf.extend_from_slice(content);
    Ok(buf)
}

/// Apply Java im-chat transport transforms: AES, gzip, then framing.
pub fn encode_transport_frame(
    body_aes_key: &str,
    message_id: u16,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> AppResult<Vec<u8>> {
    if content.len() > MAX_DECOMPRESSED_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "application body length {} exceeds limit {}",
            content.len(),
            MAX_DECOMPRESSED_BODY_SIZE
        )));
    }
    let mut body = content.to_vec();

    if encrypted {
        let cipher = AesCipher::try_new(body_aes_key.as_bytes())?;
        body = cipher.encrypt(&body)?;
    }

    if zipped {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&body)?;
        body = encoder.finish()?;
    }

    encode_frame(message_id, &body, encrypted, zipped)
}

/// Decode a TCP frame, returning (message_id, content_bytes).
pub fn decode_frame(data: &[u8]) -> AppResult<(u16, Vec<u8>)> {
    if data.len() < 8 {
        return Err(AppError::TcpFrame(
            "data too short for frame header".to_string(),
        ));
    }

    if data[0] != 0xC0 {
        return Err(AppError::TcpFrame(format!(
            "invalid frame marker: 0x{:02X}",
            data[0]
        )));
    }

    let message_id = u16::from_be_bytes([data[2], data[3]]);
    let content_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    if content_len > MAX_FRAME_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "declared frame body length {} exceeds limit {}",
            content_len, MAX_FRAME_BODY_SIZE
        )));
    }

    let wire_len = 8usize
        .checked_add(content_len)
        .ok_or_else(|| AppError::TcpFrame("frame length overflow".to_string()))?;
    if data.len() < wire_len {
        return Err(AppError::TcpFrame(format!(
            "truncated frame: need {} bytes, have {}",
            wire_len,
            data.len() - 8
        )));
    }

    let content = data[8..wire_len].to_vec();
    Ok((message_id, content))
}

/// Decode one frame and reverse transforms according to its header flags.
pub fn decode_transport_frame(
    body_aes_key: &str,
    data: &[u8],
) -> Result<DecodedFrame, FrameDecodeError> {
    if data.len() < 8 {
        return Err(FrameDecodeError::Incomplete {
            needed: 8,
            available: data.len(),
        });
    }

    if data[0] != 0xC0 {
        return Err(FrameDecodeError::Invalid(AppError::TcpFrame(format!(
            "invalid frame marker: 0x{:02X}",
            data[0]
        ))));
    }

    let header = TcpFrameHeader::parse([data[0], data[1]])?;
    let message_id = u16::from_be_bytes([data[2], data[3]]);
    let content_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    if content_len > MAX_FRAME_BODY_SIZE {
        return Err(FrameDecodeError::Invalid(AppError::TcpFrame(format!(
            "declared frame body length {} exceeds limit {}",
            content_len, MAX_FRAME_BODY_SIZE
        ))));
    }
    let wire_len = 8usize.checked_add(content_len).ok_or_else(|| {
        FrameDecodeError::Invalid(AppError::TcpFrame("frame length overflow".to_string()))
    })?;
    if data.len() < wire_len {
        return Err(FrameDecodeError::Incomplete {
            needed: wire_len,
            available: data.len(),
        });
    }
    let mut content = data[8..wire_len].to_vec();

    // Java MessageUtil only applies transport transforms when content.length > 1.
    // The TCP dispatcher sends an encrypted-flagged, empty message 200 as the
    // connection acknowledgement.
    if content.len() > 1 {
        if header.zipped {
            content = decompress_limited(&content)?;
        }

        if header.encrypted {
            let cipher = AesCipher::try_new(body_aes_key.as_bytes())?;
            content = cipher.decrypt(&content)?;
        }
    }

    Ok(DecodedFrame {
        header,
        message_id,
        content,
        wire_len,
    })
}

/// Decode a server frame, allowing Java's pre-session error response to use
/// the legacy key before `KEY_SECRET_KEY` has been installed on the session.
pub fn decode_server_frame(
    body_aes_key: &str,
    data: &[u8],
) -> Result<DecodedFrame, FrameDecodeError> {
    match decode_transport_frame(body_aes_key, data) {
        Err(FrameDecodeError::Invalid(AppError::AesDecrypt(_)))
            if data.len() >= 4
                && u16::from_be_bytes([data[2], data[3]]) == SERVER_ERROR_MESSAGE_ID =>
        {
            decode_transport_frame(PRE_SESSION_AES_KEY, data)
        }
        result => result,
    }
}

fn decompress_limited(content: &[u8]) -> AppResult<Vec<u8>> {
    let decoder = GzDecoder::new(content);
    let mut limited = decoder.take((MAX_DECOMPRESSED_BODY_SIZE as u64) + 1);
    let mut decompressed = Vec::new();
    limited
        .read_to_end(&mut decompressed)
        .map_err(|error| AppError::TcpFrame(format!("gzip decompress failed: {}", error)))?;
    if decompressed.len() > MAX_DECOMPRESSED_BODY_SIZE {
        return Err(AppError::TcpFrame(format!(
            "decompressed frame body exceeds limit {}",
            MAX_DECOMPRESSED_BODY_SIZE
        )));
    }
    Ok(decompressed)
}
