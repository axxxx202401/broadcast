//! 开奖配置与开奖历史命令。
//!
//! 提供获取/保存当前账号的开奖配置，以及从远端拉取开奖历史列表的能力。
//! 配置持久化在账号专属的 SQLite 表 `lottery_config` 中；历史 API 调用通过
//! [`im_http::lottery`] 完成。

use chrono::Utc;
use im_http::lottery;
use im_store::lottery_config::LotteryConfigRow;
use tauri::State;

use crate::state::AppState;

/// 暴露给前端的开奖配置。
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LotteryConfigDto {
    /// 用户填写的 API URL。
    pub api_url: String,
    /// 当前关注的期号列表（从 API 历史获取的所有期号）；空列表表示尚未设置。
    pub current_issues: Vec<i64>,
}

/// 暴露给前端的开奖历史条目。
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DrawItemDto {
    /// 期号。
    pub pre_draw_issue: i64,
    /// 开奖时间字符串。
    pub pre_draw_time: String,
}

/// 读取当前账号的开奖配置。
///
/// 若配置表中无记录，返回空 URL 与空期号列表的默认值。
#[tauri::command]
pub async fn get_lottery_config(state: State<'_, AppState>) -> Result<LotteryConfigDto, String> {
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    let row = db
        .lottery_config
        .get(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    tracing::debug!(
        uid = session.uid,
        api_url = ?row.api_url,
        issue_count = row.current_issues.len(),
        "Loaded lottery config"
    );
    Ok(LotteryConfigDto {
        api_url: row.api_url,
        current_issues: row.current_issues,
    })
}

/// 保存当前账号的开奖配置。
///
/// `current_issues` 为从 API 获取的期号列表（降序排列的最新若干条）。
/// 历史消息的 `matched` 标记在入库时由 `persist_monitored_batch` 确定，
/// 此处仅作持久化，不 recompute 历史消息。
#[tauri::command]
pub async fn set_lottery_config(
    state: State<'_, AppState>,
    api_url: String,
    current_issues: Vec<i64>,
) -> Result<(), String> {
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    let updated_at = Utc::now().timestamp_millis();
    tracing::info!(
        uid = session.uid,
        api_url,
        issue_count = current_issues.len(),
        "Saving lottery config"
    );
    db.lottery_config
        .upsert(&LotteryConfigRow {
            uid: session.uid,
            api_url,
            current_issues,
            updated_at,
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 从远端拉取开奖历史，返回按期号降序排列的最新若干条。
///
/// 使用当前账号配置的 API URL；URL 为空时返回错误。
#[tauri::command]
pub async fn fetch_lottery_history(state: State<'_, AppState>) -> Result<Vec<DrawItemDto>, String> {
    let session = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?;
    let db = state
        .account_db
        .require(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    let config = db
        .lottery_config
        .get(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    tracing::debug!(uid = session.uid, api_url = ?config.api_url, "Fetching lottery history");
    if config.api_url.is_empty() {
        return Err("Lottery API URL not configured".to_string());
    }
    let items = lottery::fetch_draw_history(&config.api_url).await?;
    tracing::debug!(
        uid = session.uid,
        count = items.len(),
        "Fetched lottery history"
    );
    Ok(items
        .into_iter()
        .map(|item| DrawItemDto {
            pre_draw_issue: item.pre_draw_issue,
            pre_draw_time: item.pre_draw_time,
        })
        .collect())
}
