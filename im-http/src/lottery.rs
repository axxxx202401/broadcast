//! 第三方开奖历史 API 客户端。
//!
//! 调用方提供完整的 API URL；本模块负责发起请求并解析返回的 JSON 为 [`DrawItem`] 列表。
//! 使用模块级静态 `reqwest::Client` 复用连接池，并以异步互斥锁串行化后台轮询和手动刷新，
//! 避免同一进程对第三方接口发起重叠请求。

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static LOTTERY_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);
/// 进程内所有开奖请求共用串行锁，避免后台轮询与用户手动刷新重叠访问第三方 API。
static LOTTERY_FETCH_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// 开奖历史列表中的一条记录。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DrawItem {
    /// 期号，例如 `3477887`。
    #[serde(rename = "preDrawIssue")]
    pub pre_draw_issue: i64,
    /// 开奖时间，格式为 `"YYYY-MM-DD HH:MM:SS"`。
    #[serde(rename = "preDrawTime")]
    pub pre_draw_time: String,
    /// 开奖号码，逗号分隔，例如 `"8,8,2"`。
    #[serde(rename = "preDrawCode", default)]
    pub pre_draw_code: String,
    /// 和值（三位号码之和）。
    #[serde(rename = "sumNum", default)]
    pub sum_num: i64,
    /// 和大/小标识：1=大，-1=小，其他=中。
    #[serde(rename = "sumBigSmall", default)]
    pub sum_big_small: i64,
    /// 和单/双标识：1=单，-1=双，其他=中。
    #[serde(rename = "sumSingleDouble", default)]
    pub sum_single_double: i64,
}

/// 调用开奖历史 API 并返回按期号降序排列的最新若干条记录。
///
/// `url` 应为完整的 API 地址（如 `https://go124.com/api/hash/get28HistoryList/10091`）；
/// 请求失败或响应 JSON 结构不匹配时返回错误。
pub async fn fetch_draw_history(url: &str) -> Result<Vec<DrawItem>, String> {
    let _fetch_guard = LOTTERY_FETCH_LOCK.lock().await;
    let resp = LOTTERY_CLIENT
        .get(url)
        .send()
        .await
        .map_err(|e| format!("failed to call lottery API: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("lottery API returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse lottery response: {e}"))?;

    tracing::debug!(
        url,
        "lottery API response keys: {:?}",
        body.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );

    // 响应结构通常为 {"result": {"list": [...]}} 或 {"data": [...]} 或直接为数组；兼容三种格式。
    if body
        .get("success")
        .is_some_and(|v| v.as_bool() == Some(false))
    {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(format!("API 错误：{}", msg));
    }

    let items = if let Some(list) = body
        .get("result")
        .and_then(|v| v.get("list"))
        .and_then(|v| v.as_array())
    {
        tracing::debug!("matched result.list, length: {}", list.len());
        list
    } else if let Some(list) = body.get("data").and_then(|v| v.as_array()) {
        tracing::debug!("matched data, length: {}", list.len());
        list
    } else if let Some(list) = body.as_array() {
        tracing::debug!("matched direct array, length: {}", list.len());
        list
    } else {
        return Err("lottery API response has unexpected structure".to_string());
    };

    let mut draws: Vec<DrawItem> = items
        .iter()
        .filter_map(|item| DrawItem::deserialize(item).ok())
        .collect();

    // 按期号降序，最新的在前。
    draws.sort_by_key(|a| std::cmp::Reverse(a.pre_draw_issue));
    Ok(draws)
}

/// 将 API 返回的逗号分隔号码字符串转换为 "＋" 连接的补零格式。
/// 例如 `"8,8,2"` → `"08+08+02"`。
pub fn pre_draw_code_to_display(code: &str) -> String {
    code.split(',')
        .map(|n| format!("{:02}", n.trim().parse::<u32>().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join("+")
}

/// 将和值格式化为至少两位；0 至 9 补前导零，双位及以上保持原值。
pub fn lottery_sum_to_display(sum: i64) -> String {
    if (0..=9).contains(&sum) {
        format!("{sum:02}")
    } else {
        sum.to_string()
    }
}

/// 大/小/中枚举转换：1→"大"，-1→"小"，其他→"中"。
pub fn big_small_to_str(v: i64) -> &'static str {
    match v {
        1 => "大",
        -1 => "小",
        _ => "中",
    }
}

/// 单/双/中枚举转换：1→"单"，-1→"双"，其他→"中"。
pub fn single_double_to_str(v: i64) -> &'static str {
    match v {
        1 => "单",
        -1 => "双",
        _ => "中",
    }
}

/// 根据三位开奖号码判断组合特征：
/// - 三数相同 → "豹子"
/// - 三数连续且非 8,9,0/9,0,1 → "顺子"
/// - 恰好两数相同 → "对子"
/// - 以上均不满足 → ""
pub fn compute_pattern_desc(codes: &[i32]) -> String {
    if codes.len() != 3 {
        return String::new();
    }
    let mut sorted = codes.to_vec();
    sorted.sort_unstable();
    // 豹子：三数相同
    if sorted[0] == sorted[1] && sorted[1] == sorted[2] {
        return " 豹子".to_string();
    }
    // 顺子：排序后相邻差值均为1，排除 8,9,0 和 9,0,1
    if sorted[1] - sorted[0] == 1 && sorted[2] - sorted[1] == 1 {
        // 8,9,0 排序后为 [0,8,9]，差值为 8,1，不满足全1条件，已排除
        // 9,0,1 排序后为 [0,1,9]，差值为 1,8，不满足全1条件，已排除
        // 只有连续三数才满足此条件
        return " 顺子".to_string();
    }
    // 对子：恰好两数相同
    if sorted[0] == sorted[1] || sorted[1] == sorted[2] {
        return " 对子".to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_draw_history_parses_real_api() {
        let items = fetch_draw_history("https://go124.com/api/hash/get28HistoryList/10091")
            .await
            .expect("should fetch successfully");
        assert!(!items.is_empty(), "should return at least one item");
        // 按期号降序，第一条应该是最大的。
        if items.len() > 1 {
            assert!(items[0].pre_draw_issue > items[1].pre_draw_issue);
        }
    }

    #[test]
    fn pre_draw_code_to_display_formats_correctly() {
        assert_eq!(pre_draw_code_to_display("8,8,2"), "08+08+02");
        assert_eq!(pre_draw_code_to_display("7,2,6"), "07+02+06");
        assert_eq!(pre_draw_code_to_display("10,5,3"), "10+05+03");
    }

    #[test]
    fn lottery_sum_display_pads_single_digit_values() {
        assert_eq!(lottery_sum_to_display(0), "00");
        assert_eq!(lottery_sum_to_display(5), "05");
        assert_eq!(lottery_sum_to_display(9), "09");
        assert_eq!(lottery_sum_to_display(18), "18");
    }

    #[test]
    fn draw_item_serializes_for_frontend_event_with_camel_case_fields() {
        let draw = DrawItem {
            pre_draw_issue: 101,
            pre_draw_time: "2026-09-14 13:00:00".to_string(),
            pre_draw_code: "1,3,5".to_string(),
            sum_num: 9,
            sum_big_small: -1,
            sum_single_double: 1,
        };

        let json = serde_json::to_value(draw).unwrap();
        assert_eq!(json["preDrawIssue"], 101);
        assert_eq!(json["preDrawCode"], "1,3,5");
        assert_eq!(json["sumNum"], 9);
    }

    #[test]
    fn big_small_to_str_returns_correct_chinese() {
        assert_eq!(big_small_to_str(1), "大");
        assert_eq!(big_small_to_str(-1), "小");
        assert_eq!(big_small_to_str(0), "中");
    }

    #[test]
    fn single_double_to_str_returns_correct_chinese() {
        assert_eq!(single_double_to_str(1), "单");
        assert_eq!(single_double_to_str(-1), "双");
        assert_eq!(single_double_to_str(0), "中");
    }

    #[test]
    fn compute_pattern_desc_identifies_triple() {
        assert_eq!(compute_pattern_desc(&[8, 8, 8]), "豹子");
    }

    #[test]
    fn compute_pattern_desc_identifies_pair() {
        assert_eq!(compute_pattern_desc(&[8, 8, 2]), "对子");
        assert_eq!(compute_pattern_desc(&[2, 8, 8]), "对子");
    }

    #[test]
    fn compute_pattern_desc_identifies_shunzi() {
        assert_eq!(compute_pattern_desc(&[3, 4, 5]), "顺子");
        assert_eq!(compute_pattern_desc(&[5, 3, 4]), "顺子");
        assert_eq!(compute_pattern_desc(&[1, 2, 3]), "顺子");
        assert_eq!(compute_pattern_desc(&[6, 7, 8]), "顺子");
        assert_eq!(compute_pattern_desc(&[7, 8, 9]), "顺子");
    }

    #[test]
    fn compute_pattern_desc_excludes_special_sequences() {
        // 8,9,0 排序后 [0,8,9]，差值 8,1 → 不满足全1 → 无描述
        assert_eq!(compute_pattern_desc(&[8, 9, 0]), "");
        // 9,0,1 排序后 [0,1,9]，差值 1,8 → 不满足全1 → 无描述
        assert_eq!(compute_pattern_desc(&[9, 0, 1]), "");
    }

    #[test]
    fn compute_pattern_desc_no_match_for_random() {
        assert_eq!(compute_pattern_desc(&[1, 5, 9]), "");
        assert_eq!(compute_pattern_desc(&[2, 5, 8]), "");
    }
}
