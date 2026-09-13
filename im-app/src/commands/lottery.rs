//! 开奖配置与开奖历史命令。
//!
//! 提供获取/保存当前账号的开奖配置，以及从远端拉取开奖历史列表的能力。
//! 配置持久化在账号专属的 SQLite 表 `lottery_config` 中；历史 API 调用通过
//! [`im_http::lottery`] 完成。

use chrono::Utc;
use im_http::lottery;
use im_store::lottery_config::LotteryConfigRow;
use im_store::lottery_template::LotteryTemplateRow;
use tauri::State;

use crate::state::AppState;

/// 暴露给前端的开奖配置。
#[derive(serde::Serialize, Clone)]
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
    /// 开奖号码，保留 API 的逗号分隔原始值。
    pub pre_draw_code: String,
    /// 三个开奖号码的和值。
    pub sum_num: i64,
    /// 和大/小/中标识：1=大，-1=小，其他=中。
    pub sum_big_small: i64,
    /// 和单/双/中标识：1=单，-1=双，其他=中。
    pub sum_single_double: i64,
}

/// 完整保留开奖 API 字段，供界面显示与保存消息匹配期号。
fn draw_item_dto(item: lottery::DrawItem) -> DrawItemDto {
    DrawItemDto {
        pre_draw_issue: item.pre_draw_issue,
        pre_draw_time: item.pre_draw_time,
        pre_draw_code: item.pre_draw_code,
        sum_num: item.sum_num,
        sum_big_small: item.sum_big_small,
        sum_single_double: item.sum_single_double,
    }
}

/// 暴露给前端的广播模板。
#[derive(serde::Serialize, Clone)]
pub struct LotteryTemplateDto {
    /// 模板内容；含占位符如 `${preDrawIssue}`。
    pub template: String,
    /// 广播功能是否启用。
    pub enabled: bool,
}

/// 读取当前账号的开奖配置；数据库未配置时回退到构建期注入的默认 API URL。
///
/// 若配置表中无记录且未设置默认 URL，返回空 URL 与空期号列表。
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
    let default_api_url = state.config.read().await.lottery_default_api_url.clone();
    let api_url = if row.api_url.is_empty() {
        default_api_url.clone()
    } else {
        row.api_url.clone()
    };
    tracing::info!(
        uid = session.uid,
        db_api_url = ?row.api_url,
        default_api_url = ?default_api_url,
        resolved_api_url = ?api_url,
        issue_count = row.current_issues.len(),
        "Loaded lottery config"
    );
    Ok(LotteryConfigDto {
        api_url,
        current_issues: row.current_issues.clone(),
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
    let api_url = if config.api_url.is_empty() {
        state.config.read().await.lottery_default_api_url.clone()
    } else {
        config.api_url
    };
    tracing::debug!(uid = session.uid, api_url = ?api_url, "Fetching lottery history");
    if api_url.is_empty() {
        return Err("Lottery API URL not configured".to_string());
    }
    let items = lottery::fetch_draw_history(&api_url).await?;
    tracing::debug!(
        uid = session.uid,
        count = items.len(),
        "Fetched lottery history"
    );
    Ok(items.into_iter().map(draw_item_dto).collect())
}

/// 读取当前账号的广播模板。
#[tauri::command]
pub async fn get_lottery_template(
    state: State<'_, AppState>,
) -> Result<LotteryTemplateDto, String> {
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
        .lottery_template
        .get(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    tracing::debug!(
        uid = session.uid,
        template_len = row.template.len(),
        enabled = row.enabled,
        "Loaded lottery template"
    );
    Ok(LotteryTemplateDto {
        template: row.template,
        enabled: row.enabled,
    })
}

/// 保存当前账号的广播模板及启用状态。
#[tauri::command]
pub async fn set_lottery_template(
    state: State<'_, AppState>,
    template: String,
    enabled: bool,
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
        template_len = template.len(),
        enabled,
        "Saving lottery template"
    );
    // 模板编辑与广播游标彼此独立；保存正文或开关时不得清空已处理期号。
    let current = db
        .lottery_template
        .get(session.uid)
        .await
        .map_err(|e| e.to_string())?;
    db.lottery_template
        .upsert(
            &LotteryTemplateRow {
                template,
                enabled,
                last_broadcast_issue: current.last_broadcast_issue,
                updated_at,
            },
            session.uid,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 暴露编译期构建配置（消息入库 / 消息匹配开关）。
///
/// 这两个值来自 `IM_PERSIST_RECEIVED_MESSAGES` 与 `IM_MATCH_LOTTERY_MESSAGES`，
/// 在构建时固定，应用运行期不可修改。前端据此展示真实状态。
#[derive(serde::Serialize, Clone)]
pub struct AppRuntimeConfigDto {
    pub persist_received_messages: bool,
    pub match_lottery_messages: bool,
}

/// 读取编译期构建配置；供前端展示当前构建的持久化与匹配开关状态。
#[tauri::command]
pub async fn get_app_runtime_config(
    state: State<'_, AppState>,
) -> Result<AppRuntimeConfigDto, String> {
    let config = state.config.read().await.clone();
    Ok(AppRuntimeConfigDto {
        persist_received_messages: config.persist_received_messages,
        match_lottery_messages: config.match_lottery_messages,
    })
}

#[cfg(test)]
mod tests {
    use super::draw_item_dto;

    #[test]
    fn draw_item_dto_keeps_all_lottery_fields() {
        let dto = draw_item_dto(im_http::lottery::DrawItem {
            pre_draw_issue: 20260914001,
            pre_draw_time: "2026-09-14 04:00:00".to_string(),
            pre_draw_code: "8,8,2".to_string(),
            sum_num: 18,
            sum_big_small: 1,
            sum_single_double: -1,
        });

        let json = serde_json::to_value(dto).unwrap();
        assert_eq!(json["preDrawCode"], "8,8,2");
        assert_eq!(json["sumNum"], 18);
        assert_eq!(json["sumBigSmall"], 1);
        assert_eq!(json["sumSingleDouble"], -1);
    }
}
