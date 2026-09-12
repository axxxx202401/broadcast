//! 开奖配置与开奖历史命令。
//!
//! 提供获取/保存当前账号的开奖配置，以及从远端拉取开奖历史列表的能力。
//! 配置持久化在账号专属的 SQLite 表 `lottery_config` 中；历史 API 调用通过
//! [`im_http::lottery`] 完成。

use chrono::Utc;
use im_http::lottery;
use im_store::lottery_config::LotteryConfigRow;
use im_store::lottery_template::LotteryTemplateRow;
use sqlx::Row;
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
}

/// 暴露给前端的广播模板。
#[derive(serde::Serialize, Clone)]
pub struct LotteryTemplateDto {
    /// 模板内容；含占位符如 `${preDrawIssue}`。
    pub template: String,
    /// 广播功能是否启用。
    pub enabled: bool,
}

/// 暴露给前端的广播发送状态记录。
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSendLogDto {
    /// 消息主键。
    pub msg_id: String,
    /// 消息所属群组 ID。
    pub group_id: String,
    /// 期号；暂不填充，由前端从 content_text 中提取。
    pub issue: i64,
    /// 广播发送状态：0=待发送，1=发送成功，2=发送失败。
    pub broadcast_status: i32,
    /// 消息发送时间（Unix ms）。
    pub send_time: i64,
    /// 解密后的明文文本。
    pub content_text: String,
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
    Ok(items
        .into_iter()
        .map(|item| DrawItemDto {
            pre_draw_issue: item.pre_draw_issue,
            pre_draw_time: item.pre_draw_time,
        })
        .collect())
}

/// 读取当前账号的广播模板。
#[tauri::command]
pub async fn get_lottery_template(state: State<'_, AppState>) -> Result<LotteryTemplateDto, String> {
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
    db.lottery_template
        .upsert(
            &LotteryTemplateRow {
                template,
                enabled,
                updated_at,
            },
            session.uid,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 查询当前账号的广播发送日志（broadcast_status != 0 且 matched = 1 的消息）。
#[tauri::command]
pub async fn get_broadcast_send_log(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<BroadcastSendLogDto>, String> {
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
    let limit = limit.unwrap_or(50) as usize;
    let rows = sqlx::query(
        r#"SELECT m.msg_id, m.group_id, m.send_time, m.content_text, m.broadcast_status
           FROM messages m
           WHERE m.broadcast_status != 0 AND m.matched = 1
           ORDER BY m.send_time DESC, m.msg_id DESC
           LIMIT ?"#,
    )
    .bind(limit as i64)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| e.to_string())?;

    tracing::debug!(
        uid = session.uid,
        count = rows.len(),
        limit,
        "Fetched broadcast send logs"
    );

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let msg_id: i64 = row.get("msg_id");
        result.push(BroadcastSendLogDto {
            msg_id: msg_id.to_string(),
            group_id: row.get::<i64, _>("group_id").to_string(),
            issue: 0, // 暂不填充，由前端从 content_text 中提取
            broadcast_status: row.get("broadcast_status"),
            send_time: row.get("send_time"),
            content_text: row.get("content_text"),
        });
    }
    Ok(result)
}
