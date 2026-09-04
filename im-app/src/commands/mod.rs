//! 暴露给前端的 Tauri 命令集合，并提供跨命令复用的参数解析辅助函数。

/// 账号列表、启动恢复、切换、暂停会话和移除。
pub mod accounts;
/// 登录、登出以及短信、邮箱等认证验证流程。
pub mod auth;
/// 聊天连接生命周期、连接状态查询与消息读取流程。
pub mod chat;
/// 群组同步、列表查询与监控开关流程。
pub mod groups;

/// 将前端传入的十进制字符串解析为 Rust `i64` 标识符。
///
/// JavaScript 的 `Number` 无法精确表示全部 64 位整数，因此群组、用户等标识符以十进制
/// 字符串跨越 IPC 边界。空白、非整数格式以及超出 `i64` 范围的值都会返回包含字段名的错误。
pub(crate) fn parse_i64_id(value: &str, field: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| format!("Invalid {field}: expected a decimal i64 string"))
}

#[cfg(test)]
mod tests {
    use super::parse_i64_id;

    #[test]
    fn parses_full_range_decimal_identifier_strings() {
        // 覆盖 JavaScript 安全整数范围之外的 i64 上界，确认字符串 IPC 契约不损失精度。
        assert_eq!(
            parse_i64_id(&i64::MAX.to_string(), "group_id").unwrap(),
            i64::MAX
        );
    }

    #[test]
    fn rejects_non_decimal_or_out_of_range_identifier_strings() {
        // 同时覆盖空值、前导空白、小数和溢出，确保调用方得到带字段上下文的统一错误。
        for value in ["", " 7", "7.0", "90000000000000000000"] {
            assert!(parse_i64_id(value, "group_id")
                .unwrap_err()
                .contains("Invalid group_id"));
        }
    }
}
