use std::io::{Read, Write};

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use im_common::aes::AesCipher;
use im_common::error::AppError;
use im_common::tcp_head::TcpFrameHeader;
/// 重新导出协议大小限制：[`MAX_FRAME_BODY_SIZE`] 限制帧中编码后的内容，
/// [`MAX_DECOMPRESSED_BODY_SIZE`] 限制请求明文和响应解压结果。
pub use im_common::{MAX_DECOMPRESSED_BODY_SIZE, MAX_FRAME_BODY_SIZE};

const JAVA_GZIP_THRESHOLD: usize = 5 * 1024;
const GATEWAY_REQUEST_MARKER: u8 = 0xC0;
const IM_BIZ_REQUEST_MARKER: u8 = 0xC1;

/// 将 JSON 明文编码为 Gateway 请求体。
///
/// 帧格式为 `[2B head][4B BE length][content]`，首字节 marker 固定为 `0xC0`。
/// 本函数先对明文做 AES 加密，再在加密后 payload **严格大于** 5 KiB 时 gzip 压缩，
/// 以兼容 Java 客户端的阈值与处理顺序。AES 仅用于协议兼容，不应据此推断该接口具备
/// 端到端安全性。
///
/// 明文超过 [`MAX_DECOMPRESSED_BODY_SIZE`]，或最终编码内容超过
/// [`MAX_FRAME_BODY_SIZE`] 时返回 [`AppError::TcpFrame`]；加密、压缩失败也会返回对应错误。
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

/// 按指定选项将内容编码为 Gateway 请求体。
///
/// 帧格式为 `[2B head][4B BE length][content]`，marker 为 `0xC0`。
/// `encrypted` 控制是否 AES 加密，`zipped` 控制是否 gzip；两者同时启用时固定为
/// “先加密、后压缩”，也允许只启用其中之一或均不启用。即使关闭加密或压缩，仍会执行
/// 明文与编码后大小检查。AES 的用途是兼容服务端协议，并不额外承诺传输安全性。
///
/// 大小超限、长度无法表示为 `u32`，或所选编码步骤失败时返回错误。
pub fn build_gateway_request_body_with_options(
    cipher: &AesCipher,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Result<Vec<u8>, AppError> {
    let content = encode_request_content(cipher, content, encrypted, zipped)?;
    build_length_framed_request(GATEWAY_REQUEST_MARKER, encrypted, zipped, content)
}

/// 解析 Gateway 的长度分帧响应。
///
/// 响应格式为 `[2B head][4B BE length][content]`。首个 head 字节仅要求高两位为
/// `0b11`，低六位由当前解析器为兼容性直接忽略，不解释其业务含义。协议版本位于
/// 第二个 head 字节的低四位；同一字节的位 7、6、5、4 依次为 `encrypted`、
/// `zipped`、`encrypted_system_version`（协议字段 `encryptedSystemVersion`）和
/// `is_report`（协议字段 `isReport`）。解析响应正文时当前只根据 `encrypted` 与
/// `zipped` 决定变换，其他元数据虽会解析，但不参与正文处理。两个标志同时存在时
/// 按“先 gzip 解压、后 AES 解密”的响应顺序处理。
///
/// 帧过短、marker 非法、声明长度超限或数据截断，以及解压、解密失败时返回错误。
pub fn parse_gateway_response(cipher: &AesCipher, data: &[u8]) -> Result<Vec<u8>, AppError> {
    parse_length_framed_response(cipher, data)
}

/// 将内容编码为 im-biz 请求体。
///
/// 帧格式为 `[2B head][4B BE length][content]`，首字节 marker 固定为 `0xC1`；
/// 默认对内容做 AES 加密但不压缩。AES 用于协议兼容，不代表额外的端到端安全保证。
/// 明文或编码后内容超限，以及加密失败时返回错误。
pub fn build_im_biz_request_body(cipher: &AesCipher, content: &[u8]) -> Result<Vec<u8>, AppError> {
    build_im_biz_request_body_with_options(cipher, content, true, false)
}

/// 按指定选项将内容编码为 im-biz 请求体。
///
/// 帧格式为 `[2B head][4B BE length][content]`，marker 为 `0xC1`。
/// `encrypted` 与 `zipped` 分别控制 AES 加密和 gzip 压缩；两者同时启用时先加密、
/// 后压缩，也支持仅加密、仅压缩或二者均关闭。所有组合都受明文和编码后大小限制。
///
/// 大小超限、长度无法表示为 `u32`，或所选编码步骤失败时返回错误。
pub fn build_im_biz_request_body_with_options(
    cipher: &AesCipher,
    content: &[u8],
    encrypted: bool,
    zipped: bool,
) -> Result<Vec<u8>, AppError> {
    let content = encode_request_content(cipher, content, encrypted, zipped)?;
    build_length_framed_request(IM_BIZ_REQUEST_MARKER, encrypted, zipped, content)
}

/// 写入两字节 head、四字节大端内容长度以及编码后的内容。
///
/// `marker` 区分请求通道，第二个 head 字节由加密和压缩标志生成；写入前再次检查
/// 编码后内容上限，避免内部调用绕过限制。
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

/// 根据选项编码请求内容，并在编码前后分别执行大小限制。
///
/// 同时启用两个选项时顺序固定为明文 → AES → gzip；关闭加密或压缩时跳过对应步骤，
/// 不改变剩余步骤的次序。
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

/// 限制编码前的请求明文，避免后续复制和编码处理无界增长。
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

/// 限制将写入帧的编码后内容，确保其不超过协议允许的帧体上限。
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

/// 使用默认压缩级别生成 gzip 数据，并将 I/O 错误转换为应用错误。
fn gzip(content: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content)?;
    Ok(encoder.finish()?)
}

/// 解析 im-biz 的长度分帧响应。
///
/// 响应格式为 `[2B head][4B BE length][content]`，首个 head 字节按高两位识别兼容
/// marker，低六位由当前解析器为兼容性忽略，不解释其业务含义；协议版本位于第二个
/// head 字节的低四位。第二字节的位 7、6、5、4 依次表示 `encrypted`、`zipped`、
/// `encrypted_system_version`（`encryptedSystemVersion`）与 `is_report`（`isReport`）。
/// 当前响应正文处理只使用前两个标志；其余元数据会被解析但不影响正文变换。若同时
/// 声明压缩和加密，则先 gzip 解压、后 AES 解密。帧校验、大小检查、解压或解密失败
/// 时返回错误。
pub fn parse_im_biz_response(cipher: &AesCipher, data: &[u8]) -> Result<Vec<u8>, AppError> {
    parse_length_framed_response(cipher, data)
}

/// 校验并解析通用的 `[2B head][4B BE length][content]` 响应帧。
///
/// marker 仅检查首字节高两位为 `0b11`；首字节低六位为兼容性直接忽略，代码不解释
/// 其业务含义。第二字节的位 7、6、5、4 依次解析为 `encrypted`、`zipped`、
/// `encrypted_system_version`、`is_report`，低四位解析为 `protocol_version`。当前
/// 只有 `encrypted` 与 `zipped` 决定响应正文的解压、解密，其余元数据不参与正文
/// 处理。函数先拒绝超过 [`MAX_FRAME_BODY_SIZE`] 的声明长度，再切取完整内容；若声明
/// 了压缩和加密，则按服务端响应顺序先解压、后解密。帧尾多余字节不属于当前帧，
/// 不参与解析。
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

/// 在解压过程中最多读取“解压上限 + 1”字节。
///
/// 额外的一字节用于可靠识别超限结果；一旦超过 [`MAX_DECOMPRESSED_BODY_SIZE`] 即报错，
/// 避免高压缩比数据（zip bomb）令解压后的内存无界增长。
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

    const KEY: &[u8] = b"0123456789abcdef";

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
        // 验证双开选项的线序是 AES 后 gzip，而非对明文先压缩。
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
        // 阈值针对带 AES 填充后的 payload，且必须严格大于 5 KiB 才自动压缩。
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
        // 响应线序与请求逆向对应：先还原 gzip，再解密其中的密文。
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
        // im-biz 与 Gateway 共用响应解析约定，双标志下同样先解压后解密。
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
        // 此处改变的是首字节被兼容性忽略的低六位；协议版本实际位于第二字节低四位。
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
        // 即使不加密也不压缩，超限明文仍必须在构帧前被拒绝。
        let cipher = AesCipher::new(KEY);
        let oversized = vec![0u8; MAX_FRAME_BODY_SIZE + 1];

        let error =
            build_gateway_request_body_with_options(&cipher, &oversized, false, false).unwrap_err();

        assert!(matches!(error, AppError::TcpFrame(_)));
    }

    #[test]
    fn oversized_declared_response_is_rejected_before_waiting_for_body() {
        // 仅凭帧头即可拒绝超限声明，无需等待攻击者发送完整大响应体。
        let cipher = AesCipher::new(KEY);
        let mut response = TcpFrameHeader::build(false, false).to_vec();
        response.extend_from_slice(&((MAX_FRAME_BODY_SIZE + 1) as u32).to_be_bytes());

        let error = parse_gateway_response(&cipher, &response).unwrap_err();

        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn gzip_response_over_decompressed_limit_is_rejected() {
        // 高压缩比响应解压超过上限时终止，覆盖 zip bomb 的内存防护。
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
