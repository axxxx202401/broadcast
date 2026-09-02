/// TCP 帧头字节解析。
///
/// byte[0] = 0xC0 固定标志位
/// byte[1] bit7 = encrypted  (0x80), bit6 = zipped  (0x40),
///           bit5 = encryptedSystemVersion (0x20), bit4 = isReport (0x10),
///           bits 3-0 = protocolVersion (0x0F)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpFrameHeader {
    pub encrypted: bool,
    pub zipped: bool,
    pub encrypted_system_version: bool,
    pub is_report: bool,
    pub protocol_version: u8,
}

impl TcpFrameHeader {
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

    pub fn build(encrypted: bool, zipped: bool) -> [u8; 2] {
        Self::build_with_metadata(encrypted, zipped, false, false, 0)
    }

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
