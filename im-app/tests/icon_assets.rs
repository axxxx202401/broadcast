//! B1 雷达监控图标的静态资源契约测试。
//!
//! 这些测试直接读取编译期嵌入的 SVG 文本，避免 XML 解析器对属性顺序、空白或路径数据做归一化，
//! 从而同时约束设计语义和主资源与 Web 副本的字节级一致性。

const PRIMARY_ICON: &str = include_str!("../icons/icon.svg");
const WEB_ICON: &str = include_str!("../ui/public/icon.svg");
const PNG_32: &[u8] = include_bytes!("../icons/32x32.png");
const PNG_128: &[u8] = include_bytes!("../icons/128x128.png");
const PNG_256: &[u8] = include_bytes!("../icons/128x128@2x.png");
const ICNS: &[u8] = include_bytes!("../icons/icon.icns");
const ICO: &[u8] = include_bytes!("../icons/icon.ico");
const TAURI_CONFIG: &str = include_str!("../tauri.conf.json");

const RIGHT_FIVE_PATH: &str = r#"<path opacity="0.4" d="M30.25 37.2003H37.8218C37.882 38.4157 38.2135 39.3501 38.8163 40.0035C39.419 40.6568 40.2477 40.9834 41.3025 40.9834C42.7942 40.9834 43.962 40.4289 44.8058 39.3197C45.6497 38.1954 46.0716 36.6457 46.0716 34.6706C46.0716 33.288 45.7175 32.1865 45.0093 31.366C44.3011 30.5456 43.3593 30.1354 42.184 30.1354C41.4155 30.1354 40.7299 30.3025 40.1272 30.6367C39.5245 30.9558 39.0423 31.4116 38.6806 32.0041L31.7644 31.5028L35.4259 14H54.9995L53.7563 19.8343H40.195L38.9971 25.6685C39.9162 25.1975 40.7902 24.8557 41.6189 24.643C42.4477 24.4151 43.299 24.3011 44.173 24.3011C47.0661 24.3011 49.4468 25.2203 51.3153 27.0587C53.1988 28.8971 54.1406 31.2141 54.1406 34.0097C54.1406 37.9751 52.9803 41.1354 50.6598 43.4903C48.3393 45.8301 45.2202 47 41.3025 47C37.867 47 35.2074 46.1644 33.3239 44.4931C31.4404 42.8066 30.4157 40.3757 30.25 37.2003Z" fill="white"/>"#;
const LEFT_FIVE_PATH: &str = r#"<path d="M5 37.2003H12.5718C12.632 38.4157 12.9635 39.3501 13.5663 40.0035C14.169 40.6568 14.9977 40.9834 16.0525 40.9834C17.5442 40.9834 18.712 40.4289 19.5558 39.3197C20.3997 38.1954 20.8216 36.6457 20.8216 34.6706C20.8216 33.288 20.4675 32.1865 19.7593 31.366C19.0511 30.5456 18.1093 30.1354 16.934 30.1354C16.1655 30.1354 15.4799 30.3025 14.8772 30.6367C14.2745 30.9558 13.7923 31.4116 13.4306 32.0041L6.51435 31.5028L10.1759 14H29.7495L28.5063 19.8343H14.945L13.7471 25.6685C14.6662 25.1975 15.5402 24.8557 16.3689 24.643C17.1977 24.4151 18.049 24.3011 18.923 24.3011C21.8161 24.3011 24.1968 25.2203 26.0653 27.0587C27.9488 28.8971 28.8906 31.2141 28.8906 34.0097C28.8906 37.9751 27.7303 41.1354 25.4098 43.4903C23.0893 45.8301 19.9702 47 16.0525 47C12.617 47 9.95743 46.1644 8.07391 44.4931C6.19038 42.8066 5.16575 40.3757 5 37.2003Z" fill="white"/>"#;

/// 以完整元素片段校验关键图形，确保路径数据及相关视觉属性不会被单独改动。
fn assert_contains(svg: &str, expected: &str, element_name: &str) {
    assert!(
        svg.contains(expected),
        "B1 SVG 缺少或改动了设计契约元素：{element_name}"
    );
}

/// 从 PNG 文件头读取 IHDR 中声明的画布尺寸。
///
/// 调用方必须传入完整 PNG 资源；函数先校验最小头部长度与八字节 PNG 签名，
/// 再按 PNG 规范从固定偏移读取大端序宽高。该辅助函数仅用于静态资源契约测试，
/// 目的是在不引入图像解码器的情况下发现 Tauri 图标生成尺寸回归。
fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() >= 24, "PNG 资源至少需要包含 24 字节文件头");
    assert_eq!(
        &bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "资源必须包含标准 PNG 八字节签名"
    );

    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("宽度字段固定为四字节"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("高度字段固定为四字节"));
    (width, height)
}

/// 从指定偏移读取四字节大端无符号整数，并把字段截断统一转换为结构错误。
fn read_u32_be(bytes: &[u8], start: usize, error: &'static str) -> Result<u32, &'static str> {
    let end = start.checked_add(4).ok_or(error)?;
    let field = bytes.get(start..end).ok_or(error)?;
    Ok(u32::from_be_bytes([field[0], field[1], field[2], field[3]]))
}

/// 从指定偏移读取两字节小端无符号整数，用于解析 ICO 文件头。
fn read_u16_le(bytes: &[u8], start: usize, error: &'static str) -> Result<u16, &'static str> {
    let end = start.checked_add(2).ok_or(error)?;
    let field = bytes.get(start..end).ok_or(error)?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

/// 从指定偏移读取四字节小端无符号整数，用于解析 ICO 目录条目。
fn read_u32_le(bytes: &[u8], start: usize, error: &'static str) -> Result<u32, &'static str> {
    let end = start.checked_add(4).ok_or(error)?;
    let field = bytes.get(start..end).ok_or(error)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

/// 校验 PNG 的 chunk 边界、首块 IHDR 约束以及唯一文件结尾位置。
///
/// 每个 chunk 由四字节长度、四字节类型、变长数据和四字节 CRC 组成。这里不复算
/// CRC，但会把 CRC 纳入边界计算，避免数据被截断后仍仅凭签名和尺寸通过测试。
fn validate_png(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err("PNG 签名无效或文件头被截断");
    }

    let mut offset = 8_usize;
    let mut is_first_chunk = true;
    loop {
        let chunk_length = usize::try_from(read_u32_be(bytes, offset, "PNG chunk 长度字段被截断")?)
            .map_err(|_| "PNG chunk 长度无法转换为平台 usize")?;
        let type_start = offset.checked_add(4).ok_or("PNG chunk 类型偏移溢出")?;
        let type_end = type_start
            .checked_add(4)
            .ok_or("PNG chunk 类型末尾偏移溢出")?;
        let chunk_type = bytes
            .get(type_start..type_end)
            .ok_or("PNG chunk 类型字段被截断")?;

        if is_first_chunk && (chunk_type != b"IHDR" || chunk_length != 13) {
            return Err("PNG 首个 chunk 必须是长度为 13 的 IHDR");
        }

        // 声明长度仅覆盖数据；完整边界还必须容纳头部后的四字节 CRC。
        let data_start = type_end;
        let data_end = data_start
            .checked_add(chunk_length)
            .ok_or("PNG chunk 数据末尾偏移溢出")?;
        let chunk_end = data_end
            .checked_add(4)
            .ok_or("PNG chunk CRC 末尾偏移溢出")?;
        if chunk_end > bytes.len() {
            return Err("PNG chunk 数据或 CRC 超出文件末尾");
        }

        if chunk_type == b"IEND" {
            if chunk_length != 0 {
                return Err("PNG IEND chunk 的数据长度必须为零");
            }
            if chunk_end != bytes.len() {
                return Err("PNG IEND 后不允许存在额外字节");
            }
            return Ok(());
        }

        offset = chunk_end;
        is_first_chunk = false;
        if offset == bytes.len() {
            return Err("PNG 缺少 IEND chunk");
        }
    }
}

/// 校验 ICNS 总长度及每个条目的声明边界。
///
/// ICNS 文件头和条目头均为八字节；条目长度包含自身头部，因此小于八字节会导致
/// 解析无法前进。最终偏移必须精确等于文件总长，不能接受截断或尾随数据。
fn validate_icns(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.get(..4) != Some(b"icns") {
        return Err("ICNS magic 无效或文件头被截断");
    }

    let declared_length = usize::try_from(read_u32_be(bytes, 4, "ICNS 总长度字段被截断")?)
        .map_err(|_| "ICNS 总长度无法转换为平台 usize")?;
    if declared_length != bytes.len() {
        return Err("ICNS 声明总长度与实际文件长度不一致");
    }

    let mut offset = 8_usize;
    while offset < bytes.len() {
        let length_offset = offset.checked_add(4).ok_or("ICNS 条目长度偏移溢出")?;
        let entry_length = usize::try_from(read_u32_be(bytes, length_offset, "ICNS 条目头被截断")?)
            .map_err(|_| "ICNS 条目长度无法转换为平台 usize")?;
        if entry_length < 8 {
            return Err("ICNS 条目长度不能小于八字节头部");
        }

        let entry_end = offset
            .checked_add(entry_length)
            .ok_or("ICNS 条目末尾偏移溢出")?;
        if entry_end > bytes.len() {
            return Err("ICNS 条目声明范围超出文件末尾");
        }
        offset = entry_end;
    }

    if offset != bytes.len() {
        return Err("ICNS 条目未精确结束于文件末尾");
    }
    Ok(())
}

/// 校验 ICO 文件头、目录范围以及每个图像资源的偏移和长度。
///
/// ICO 目录从六字节文件头后开始，每项固定十六字节。资源偏移不得落入目录，
/// 且 `offset + size` 必须使用受检加法，防止恶意大值绕过文件末尾判断。
fn validate_ico(bytes: &[u8]) -> Result<(), &'static str> {
    if read_u16_le(bytes, 0, "ICO reserved 字段被截断")? != 0 {
        return Err("ICO reserved 必须为零");
    }
    if read_u16_le(bytes, 2, "ICO type 字段被截断")? != 1 {
        return Err("ICO type 必须为图标类型 1");
    }

    let count = usize::from(read_u16_le(bytes, 4, "ICO count 字段被截断")?);
    if count == 0 {
        return Err("ICO 至少需要一个目录条目");
    }
    let directory_size = count.checked_mul(16).ok_or("ICO 目录长度计算溢出")?;
    let directory_end = 6_usize
        .checked_add(directory_size)
        .ok_or("ICO 目录末尾偏移溢出")?;
    if directory_end > bytes.len() {
        return Err("ICO 目录超出文件末尾");
    }

    for index in 0..count {
        let entry_offset = 6_usize
            .checked_add(index.checked_mul(16).ok_or("ICO 条目偏移计算溢出")?)
            .ok_or("ICO 条目起始偏移溢出")?;
        let size = usize::try_from(read_u32_le(
            bytes,
            entry_offset
                .checked_add(8)
                .ok_or("ICO 资源长度字段偏移溢出")?,
            "ICO 资源长度字段被截断",
        )?)
        .map_err(|_| "ICO 资源长度无法转换为平台 usize")?;
        let image_offset = usize::try_from(read_u32_le(
            bytes,
            entry_offset
                .checked_add(12)
                .ok_or("ICO 资源偏移字段位置溢出")?,
            "ICO 资源偏移字段被截断",
        )?)
        .map_err(|_| "ICO 资源偏移无法转换为平台 usize")?;

        if size == 0 {
            return Err("ICO 资源长度必须大于零");
        }
        if image_offset < directory_end {
            return Err("ICO 资源偏移不得指向目录内部");
        }
        let image_end = image_offset
            .checked_add(size)
            .ok_or("ICO 资源末尾偏移溢出")?;
        if image_end > bytes.len() {
            return Err("ICO 资源声明范围超出文件末尾");
        }
    }
    Ok(())
}

#[test]
fn primary_icon_matches_b1_design_contract() {
    let required_elements = [
        (r#"viewBox="0 0 61 61""#, "61×61 viewBox"),
        (
            r##"<rect width="61" height="61" rx="15" fill="#178AFF"/>"##,
            "蓝色圆角背景",
        ),
        (
            r##"<path d="M30.5 30.5L55 13A30 30 0 0 1 58 39Z" fill="#71F0D0" opacity="0.22"/>"##,
            "半透明扫描扇面",
        ),
        (
            r##"<circle cx="30.5" cy="30.5" r="25" fill="none" stroke="#A7FFEA" stroke-width="1.3" opacity="0.55"/>"##,
            "外层雷达圆环",
        ),
        (
            r##"<circle cx="30.5" cy="30.5" r="17" fill="none" stroke="#A7FFEA" stroke-width="1.1" opacity="0.45"/>"##,
            "中层雷达圆环",
        ),
        (
            r##"<circle cx="30.5" cy="30.5" r="9" fill="none" stroke="#A7FFEA" stroke-width="1" opacity="0.4"/>"##,
            "内层雷达圆环",
        ),
        (
            r##"<path d="M30.5 30.5L53 14" stroke="#8CFFE2" stroke-width="1.5" stroke-linecap="round"/>"##,
            "雷达扫描线",
        ),
        (
            r##"<circle cx="47" cy="21" r="2" fill="#8CFFE2"/>"##,
            "雷达目标点",
        ),
        (RIGHT_FIVE_PATH, "右侧半透明数字 5"),
        (LEFT_FIVE_PATH, "左侧纯白数字 5"),
    ];

    // 逐项报告契约元素名称，使资源回归时能直接定位被误改的视觉组成。
    for (expected, element_name) in required_elements {
        assert_contains(PRIMARY_ICON, expected, element_name);
    }
}

#[test]
fn web_icon_is_byte_identical_to_primary_icon() {
    assert_eq!(
        WEB_ICON, PRIMARY_ICON,
        "Web SVG 必须与主 SVG 保持字节文本完全一致"
    );
}

/// 验证 Tauri CLI 生成的桌面图标具有打包工具要求的格式标识与像素尺寸。
#[test]
fn generated_desktop_icons_have_expected_formats_and_sizes() {
    validate_png(PNG_32).expect("32×32 PNG 容器结构必须有效");
    validate_png(PNG_128).expect("128×128 PNG 容器结构必须有效");
    validate_png(PNG_256).expect("256×256 PNG 容器结构必须有效");
    validate_icns(ICNS).expect("ICNS 容器结构必须有效");
    validate_ico(ICO).expect("ICO 容器结构必须有效");

    assert_eq!(png_dimensions(PNG_32), (32, 32));
    assert_eq!(png_dimensions(PNG_128), (128, 128));
    assert_eq!(png_dimensions(PNG_256), (256, 256));
    assert_eq!(&ICNS[..4], b"icns");
    assert_eq!(&ICO[..4], &[0, 0, 1, 0]);
}

/// 证明容器校验器会拒绝边界不完整或声明范围越过文件末尾的图标资源。
#[test]
fn malformed_icon_containers_are_rejected() {
    // 截到 IHDR 数据末尾，故意移除该块 CRC 以及后续所有块。
    let truncated_png = &PNG_32[..29];
    assert!(validate_png(truncated_png).is_err());

    let mut wrong_length_icns = ICNS.to_vec();
    let declared_length = u32::try_from(ICNS.len() + 1).expect("测试资源长度可由 u32 表示");
    wrong_length_icns[4..8].copy_from_slice(&declared_length.to_be_bytes());
    assert!(validate_icns(&wrong_length_icns).is_err());

    // 单条 ICO 目录占 22 字节；资源偏移 22 指向文件末尾，配合 size=1 必然越界。
    let mut external_entry_ico = vec![0_u8; 22];
    external_entry_ico[2..4].copy_from_slice(&1_u16.to_le_bytes());
    external_entry_ico[4..6].copy_from_slice(&1_u16.to_le_bytes());
    external_entry_ico[14..18].copy_from_slice(&1_u32.to_le_bytes());
    external_entry_ico[18..22].copy_from_slice(&22_u32.to_le_bytes());
    assert!(validate_ico(&external_entry_ico).is_err());
}

/// 验证打包配置完整且按稳定顺序引用全部桌面平台图标。
#[test]
fn tauri_bundle_references_all_desktop_icons() {
    let config: serde_json::Value =
        serde_json::from_str(TAURI_CONFIG).expect("tauri.conf.json 必须是有效 JSON");
    assert_eq!(
        config["bundle"]["icon"],
        serde_json::json!([
            "icons/32x32.png",
            "icons/128x128.png",
            "icons/128x128@2x.png",
            "icons/icon.icns",
            "icons/icon.ico"
        ])
    );
}
