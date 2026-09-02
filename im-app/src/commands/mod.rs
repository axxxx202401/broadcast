pub mod auth;
pub mod chat;
pub mod groups;

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
        assert_eq!(
            parse_i64_id(&i64::MAX.to_string(), "group_id").unwrap(),
            i64::MAX
        );
    }

    #[test]
    fn rejects_non_decimal_or_out_of_range_identifier_strings() {
        for value in ["", " 7", "7.0", "90000000000000000000"] {
            assert!(parse_i64_id(value, "group_id")
                .unwrap_err()
                .contains("Invalid group_id"));
        }
    }
}
