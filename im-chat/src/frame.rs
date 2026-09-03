use std::io::{Read, Write};

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use im_common::aes::AesCipher;
use im_common::error::{AppError, AppResult};
use im_common::tcp_head::TcpFrameHeader;
/// 重新导出正文长度上限：线上帧正文最大 8 MiB，应用原文或解压后正文最大 32 MiB。
pub use im_common::{MAX_DECOMPRESSED_BODY_SIZE, MAX_FRAME_BODY_SIZE};

/// 完整解码并撤销传输变换后的单个 TCP 帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    /// 从线上两字节 `head` 解析出的标志位；各位含义见
    /// [`im_common::tcp_head::TcpFrameHeader`]。
    pub header: TcpFrameHeader,
    /// 线上帧中以大端序编码的消息 ID。
    pub message_id: u16,
    /// 已按 Java 兼容规则，根据帧头标志完成解压和解密的应用正文。
    pub content: Vec<u8>,
    /// 当前帧在线上占用的总字节数，供流式读取方从缓冲区移除已消费数据。
    pub wire_len: usize,
}

/// 流式帧解码失败。
///
/// [`FrameDecodeError::Incomplete`] 表示缓冲区只含半包，调用方应保留现有数据并
/// 等待后续字节；[`FrameDecodeError::Invalid`] 表示完整数据违反协议或传输变换
/// 失败，当前连接应终止处理。
#[derive(Debug, thiserror::Error)]
pub enum FrameDecodeError {
    /// 数据尚不足以组成声明长度的完整帧。
    #[error("incomplete frame: need {needed} bytes, have {available}")]
    Incomplete {
        /// 按当前帧头声明，完成该帧所需的总字节数。
        needed: usize,
        /// 调用解码器时缓冲区内实际可用的字节数。
        available: usize,
    },
    /// 帧头、长度或正文传输变换无效。
    #[error("invalid frame: {0}")]
    Invalid(#[from] AppError),
}

/// 将正文封装为 TCP 帧，不执行加密或压缩。
///
/// 线格式统一为
/// `[head(2)][message_id(2, BE)][content_length(4, BE)][content]`。
/// `head` 的位含义由 [`im_common::tcp_head::TcpFrameHeader`] 定义。线上正文不得
/// 超过 8 MiB。
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

/// 使用调用方提供的两字节帧头封装 TCP 帧。
///
/// 线格式为
/// `[head(2)][message_id(2, BE)][content_length(4, BE)][content]`；
/// 本函数不解析或改写 `head`，仅校验 8 MiB 线上正文上限。
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

/// 按 Java im-chat 的顺序执行发送侧传输变换并封帧。
///
/// 应用明文先在启用加密时经过 AES，再在启用压缩时经过 gzip，最后按
/// `[head(2)][message_id(2, BE)][content_length(4, BE)][content]` 封装。
/// 输入应用正文受 32 MiB 上限约束，变换后的线上正文受 8 MiB 上限约束。
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

/// 从完整 TCP 帧中读取消息 ID 和原始线上正文，不撤销加密或压缩。
///
/// 线格式为
/// `[head(2)][message_id(2, BE)][content_length(4, BE)][content]`；
/// 声明的线上正文长度不得超过 8 MiB。
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

/// 解码一个 TCP 帧，并按帧头标志撤销接收侧传输变换。
///
/// 接收顺序与发送相反：先在压缩标志存在时解 gzip，再在加密标志存在时执行
/// AES 解密。线上正文不得超过 8 MiB，解压后正文不得超过 32 MiB。
///
/// 为兼容 Java `MessageUtil`，当 `content.len() <= 1` 时即使帧头设置了加密或
/// 压缩标志也不执行对应变换。
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

    // Java MessageUtil 仅在 content.length > 1 时执行传输变换；因此空正文等
    // 短正文即使带有加密标志，也必须原样交给上层。
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

/// 在 32 MiB 输出上限内解压 gzip 正文。
///
/// 解压读取最多允许上限加一个字节，以便识别并拒绝可能持续膨胀的压缩数据，
/// 避免将超限输出完整载入内存。
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
