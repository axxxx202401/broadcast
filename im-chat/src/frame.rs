use im_common::error::{AppError, AppResult};
use im_common::tcp_head::TcpFrameHeader;

/// Encode a TCP frame.
/// Wire format: [head(2)][messageId(2,BE)][contentLength(4,BE)][content]
pub fn encode_frame(
    message_id: u16,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Vec<u8> {
    let head = TcpFrameHeader::build(encrypted, zipped);
    let content_len = content.len() as u32;

    let mut buf = Vec::with_capacity(2 + 2 + 4 + content.len());
    buf.extend_from_slice(&head);
    buf.extend_from_slice(&message_id.to_be_bytes());
    buf.extend_from_slice(&content_len.to_be_bytes());
    buf.extend_from_slice(content);
    buf
}

/// Decode a TCP frame, returning (message_id, content_bytes).
pub fn decode_frame(data: &[u8]) -> AppResult<(u16, Vec<u8>)> {
    if data.len() < 8 {
        return Err(AppError::TcpFrame(
            "data too short for frame header".to_string(),
        ));
    }

    let head = [data[0], data[1]];
    let _header = TcpFrameHeader::parse(head);
    let message_id = u16::from_be_bytes([data[2], data[3]]);
    let content_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;

    if data.len() < 8 + content_len {
        return Err(AppError::TcpFrame(format!(
            "truncated frame: need {} bytes, have {}",
            8 + content_len,
            data.len() - 8
        )));
    }

    let content = data[8..8 + content_len].to_vec();
    Ok((message_id, content))
}
