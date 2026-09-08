//! JSON 日志脱敏工具。
//!
//! 本模块提供 [`sanitize_debug_json`]，用于在 debug 日志中隐藏敏感字段。
//! 采用黑名单策略：任何字段名归一化后包含以下关键字的字段，其值会被替换为
//! `"<redacted>"`：`token`、`password`、`secret`、`key`、`mac`、`phone`、
//! `email`、`uid`、`session`。
//!
//! 非 JSON 输入原样返回字节数信息。递归处理嵌套对象和数组。

use serde_json::Value;

/// 将字节切片解析为 JSON 并对敏感字段脱敏，返回可供日志使用的字符串。
///
/// 字段名归一化方式：去除 `_` 和 `-`，转小写，再与已知敏感关键字列表匹配。
/// 该函数不承诺覆盖所有可能的敏感字段；若新增敏感字段出现在 API 响应中，
/// 需要在此处的关键字列表中补充对应条目。
pub fn sanitize_debug_json(bytes: &[u8]) -> String {
    fn redact(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    let normalized = key
                        .chars()
                        .filter(|c| !matches!(c, '_' | '-'))
                        .collect::<String>()
                        .to_ascii_lowercase();
                    if is_sensitive_field(&normalized) {
                        *value = Value::String("<redacted>".to_string());
                    } else {
                        redact(value);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    redact(value);
                }
            }
            _ => {}
        }
    }

    match serde_json::from_slice::<Value>(bytes) {
        Ok(mut value) => {
            redact(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| "<JSON serialization failed>".into())
        }
        Err(_) => format!("<non-JSON body: {} bytes>", bytes.len()),
    }
}

/// 判断归一化后的字段名是否属于敏感关键字。
fn is_sensitive_field(normalized: &str) -> bool {
    [
        "token", "password", "secret", "key", "mac", "phone", "email", "uid", "session", "value",
    ]
    .iter()
    .any(|&kw| normalized.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_sensitive_fields() {
        let json = br#"{"access_token":"abc123","user_id":1,"name":"test"}"#;
        let result = sanitize_debug_json(json);
        assert!(result.contains("\"access_token\":\"<redacted>\""));
        assert!(result.contains("\"user_id\":1"));
        assert!(result.contains("\"name\":\"test\""));
    }

    #[test]
    fn redacts_nested_sensitive_fields() {
        let json = br#"{"data":{"password":"secret","email":"a@b.com"},"count":1}"#;
        let result = sanitize_debug_json(json);
        assert!(result.contains("\"password\":\"<redacted>\""));
        assert!(result.contains("\"email\":\"<redacted>\""));
        assert!(result.contains("\"count\":1"));
    }

    #[test]
    fn redacts_key_variant_with_hyphen() {
        let json = br#"{"api-key":"sk-123","public_key":"pk-456"}"#;
        let result = sanitize_debug_json(json);
        assert!(result.contains("\"api-key\":\"<redacted>\""));
        assert!(result.contains("\"public_key\":\"<redacted>\""));
    }

    #[test]
    fn non_json_returns_byte_count() {
        let result = sanitize_debug_json(b"not json at all");
        assert!(result.contains("15 bytes"));
    }

    #[test]
    fn does_not_redact_non_sensitive_fields() {
        let json = br#"{"group_name":"test_group","send_time":1234,"content":"hello"}"#;
        let result = sanitize_debug_json(json);
        assert!(result.contains("\"group_name\":\"test_group\""));
        assert!(result.contains("\"send_time\":1234"));
        assert!(result.contains("\"content\":\"hello\""));
    }
}
