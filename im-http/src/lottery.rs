//! 第三方开奖历史 API 客户端。
//!
//! 调用方提供完整的 API URL；本模块负责发起请求并解析返回的 JSON 为 [`DrawItem`] 列表。

use serde::Deserialize;

/// 开奖历史列表中的一条记录。
#[derive(Debug, Clone, Deserialize)]
pub struct DrawItem {
    /// 期号，例如 `3477887`。
    #[serde(rename = "preDrawIssue")]
    pub pre_draw_issue: i64,
    /// 开奖时间，格式为 `"YYYY-MM-DD HH:MM:SS"`。
    #[serde(rename = "preDrawTime")]
    pub pre_draw_time: String,
}

/// 调用开奖历史 API 并返回按期号降序排列的最新若干条记录。
///
/// `url` 应为完整的 API 地址（如 `https://go124.com/api/hash/get28HistoryList/10091`）；
/// 请求失败或响应 JSON 结构不匹配时返回错误。
pub async fn fetch_draw_history(url: &str) -> Result<Vec<DrawItem>, String> {
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("failed to call lottery API: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "lottery API returned status {}",
            resp.status()
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse lottery response: {e}"))?;

    tracing::debug!(url, "lottery API response keys: {:?}", body.as_object().map(|o| o.keys().collect::<Vec<_>>()));

    // 响应结构通常为 {"result": {"list": [...]}} 或 {"data": [...]} 或直接为数组；兼容三种格式。
    if body.get("success").is_some_and(|v| v.as_bool() == Some(false)) {
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
    draws.sort_by(|a, b| b.pre_draw_issue.cmp(&a.pre_draw_issue));
    Ok(draws)
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
}
