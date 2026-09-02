//! TCP 两字节帧头的解析与构建。
//!
//! 线上格式固定为：
//! - `byte[0]`：固定标志 `0xC0`。
//! - `byte[1]`：位 7 表示正文已加密（`0x80`），位 6 表示正文已压缩
//!   （`0x40`），位 5 表示系统版本已加密（`0x20`），位 4 表示上报帧
//!   （`0x10`），位 3..=0 表示协议版本（`0x0F`）。

/// TCP 帧头中由 `byte[1]` 编码的标志与协议版本。
///
/// 使用 [`Self::parse`] 从线上字节读取，或使用 [`Self::build`] 和
/// [`Self::build_with_metadata`] 构建线上字节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpFrameHeader {
    /// `byte[1]` 位 7；正文是否已加密。
    pub encrypted: bool,
    /// `byte[1]` 位 6；正文是否已压缩。
    pub zipped: bool,
    /// `byte[1]` 位 5；系统版本字段是否已加密。
    pub encrypted_system_version: bool,
    /// `byte[1]` 位 4；该帧是否为上报帧。
    pub is_report: bool,
    /// `byte[1]` 的低 4 位协议版本，范围为 `0..=15`。
    pub protocol_version: u8,
}

impl TcpFrameHeader {
    /// 解析线上两字节帧头。
    ///
    /// `head` 的 `byte[0]` 必须为固定标志 `0xC0`；`byte[1]` 按结构体各
    /// 字段说明拆分为标志位和低 4 位协议版本。
    ///
    /// # Errors
    ///
    /// 当 `byte[0]` 不是 `0xC0` 时返回
    /// [`crate::error::AppError::TcpFrame`]。
    pub fn parse(head: [u8; 2]) -> crate::error::AppResult<Self> {
        if head[0] != 0xC0 {
            return Err(crate::error::AppError::TcpFrame(format!(
                "invalid TCP head byte[0]: 0x{:02X}, expected 0xC0",
                head[0]
            )));
        }
        let b1 = head[1];
        Ok(Self {
            encrypted: (b1 & 0x80) != 0,
            zipped: (b1 & 0x40) != 0,
            encrypted_system_version: (b1 & 0x20) != 0,
            is_report: (b1 & 0x10) != 0,
            protocol_version: b1 & 0x0F,
        })
    }

    /// 使用正文加密、压缩标志构建基础线上帧头。
    ///
    /// `encrypted` 和 `zipped` 分别写入 `byte[1]` 的位 7 和位 6；其他
    /// 元数据位及协议版本均写为零，`byte[0]` 固定写为 `0xC0`。
    pub fn build(encrypted: bool, zipped: bool) -> [u8; 2] {
        Self::build_with_metadata(encrypted, zipped, false, false, 0)
    }

    /// 使用全部标志和协议版本构建线上两字节帧头。
    ///
    /// `encrypted`、`zipped`、`encrypted_system_version`、`is_report`
    /// 依次写入 `byte[1]` 的位 7、6、5、4；`protocol_version` 仅取低
    /// 4 位写入位 3..=0。返回值的 `byte[0]` 固定为 `0xC0`。
    pub fn build_with_metadata(
        encrypted: bool,
        zipped: bool,
        encrypted_system_version: bool,
        is_report: bool,
        protocol_version: u8,
    ) -> [u8; 2] {
        let mut b1 = 0x00u8;
        if encrypted {
            b1 |= 0x80;
        }
        if zipped {
            b1 |= 0x40;
        }
        if encrypted_system_version {
            b1 |= 0x20;
        }
        if is_report {
            b1 |= 0x10;
        }
        b1 |= protocol_version & 0x0F;
        [0xC0, b1]
    }
}
