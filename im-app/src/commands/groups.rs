use std::future::Future;

use crate::state::AppState;
use tauri::State;

/// Tauri 边界使用的群组数据。
///
/// 可能超出 JavaScript 安全整数范围的标识符均转换为十进制字符串。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupDto {
    /// 十进制字符串形式的群组 ID。
    pub group_id: String,
    /// 群组名称。
    pub name: String,
    /// 群组头像地址。
    pub pic: String,
    /// 可选的群主用户 ID，以十进制字符串表示。
    pub host_id: Option<String>,
    /// 服务端快照中的成员数量。
    pub member_count: i64,
    /// 群组创建时间；远程刷新路径因中间 HTTP 映射未保留协议的 `create_time` 而填 `0`。
    pub created_at: i64,
    /// 本地监控开关，`1` 表示监控，`0` 表示不监控。
    pub monitored: i32,
    /// 本地群组记录的更新时间；远程同步时统一取本次拉取完成后的当前毫秒时间戳。
    pub updated_at: i64,
}

impl From<im_store::group::GroupRow> for GroupDto {
    fn from(row: im_store::group::GroupRow) -> Self {
        Self {
            group_id: row.group_id.to_string(),
            name: row.name,
            pic: row.pic,
            host_id: row.host_id.map(|id| id.to_string()),
            member_count: row.member_count,
            created_at: row.created_at,
            monitored: row.monitored,
            updated_at: row.updated_at,
        }
    }
}

fn group_dtos(groups: Vec<im_store::group::GroupRow>) -> Vec<GroupDto> {
    groups.into_iter().map(GroupDto::from).collect()
}

/// 从本地数据库读取全部群组。
///
/// 该命令不发起远程请求，也不修改数据库或内存监控集合；成功返回按 Tauri 边界格式转换
/// 的群组列表。未登录、活动库与会话 UID 不一致或数据库查询错误转换为字符串返回。
#[tauri::command]
pub async fn fetch_group_list(state: State<'_, AppState>) -> Result<Vec<GroupDto>, String> {
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
        .map_err(|error| error.to_string())?;
    let groups = db.groups.list_all().await.map_err(|e| e.to_string())?;
    Ok(group_dtos(groups))
}

/// 从服务端刷新群组快照，并更新本地数据库及内存监控集合。
///
/// 命令仅在请求前从当前会话复制 token，再用它拉取远程群组；网络响应返回后不会复核
/// uid 或 generation。远程请求期间不持有 `group_ops`；拉取成功后才取得该锁，串行
/// 执行数据库快照同步、数据库回读及内存监控快照替换。该锁只协调本地群组更新，不提供
/// 认证代际隔离，因此并发登出或换号时，旧会话请求的迟到响应仍可能覆盖新会话的数据库
/// 群组和 `monitoring_groups`。任一步失败都返回字符串错误；这里也不承诺网络请求与本地
/// 写入构成原子事务。
#[tauri::command]
pub async fn refresh_group_list(state: State<'_, AppState>) -> Result<Vec<GroupDto>, String> {
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
        .map_err(|error| error.to_string())?;
    let groups = fetch_and_apply_remote_groups(
        &state.group_ops,
        &db,
        &state.monitoring_groups,
        fetch_remote_groups(&state, &session.token),
    )
    .await?;
    Ok(group_dtos(groups))
}

/// 使用 token 拉取远程群组，并转换为待同步的本地行。
///
/// 请求使用当前设备配置构造客户端信息。底层协议 `GroupBase` 含 `create_time`，但
/// `im-http` 的 `GroupInfo` 中间映射未保留该字段，因此本层收到的数据无法恢复创建时间，
/// 只能令 `created_at` 使用 `0` 占位。`updated_at` 取远程请求成功后生成的当前 UTC
/// 毫秒时间戳，同批记录使用同一值；`monitored` 先置 `0`，数据库同步层负责保留可用的
/// 本地选择。
///
/// 此函数自身不获取 `group_ops`，也不写数据库或内存监控集合；它只使用调用方传入的
/// token，不复核该 token 所属会话的 uid 或 generation。
pub(crate) async fn fetch_remote_groups(
    state: &AppState,
    token: &str,
) -> Result<Vec<im_store::group::GroupRow>, String> {
    let device = state.config.read().await.device.clone();
    let client_info = im_proto::ClientInfo {
        session_id: String::new(),
        app_ver: device.app_ver,
        package_code: device.package_code,
        plat: im_proto::Platform::Android as i32,
        language: device.language,
        sys_mac: device.sys_mac,
        sys_model: device.sys_model,
        token: token.to_string(),
        version: format!("{}-{}", device.app_ver, device.package_code),
    };
    let remote_groups = state
        .http
        .im_biz
        .fetch_group_list(&client_info)
        .await
        .map_err(|e| e.to_string())?;
    let updated_at = chrono::Utc::now().timestamp_millis();

    Ok(remote_groups
        .into_iter()
        .map(|group| im_store::group::GroupRow {
            group_id: group.group_id,
            name: group.name,
            pic: group.pic,
            host_id: group.host_id,
            member_count: group.member_count,
            created_at: 0,
            monitored: 0,
            updated_at,
        })
        .collect())
}

/// 把远程群组快照写入数据库，并从数据库重建可用群组和监控集合。
///
/// 调用方负责需要的串行化。数据库同步完成后依次查询全部可用群组和受监控群组；错误
/// 直接返回，不更新调用方持有的内存快照。此函数不检查认证会话或 generation。
pub(crate) async fn apply_remote_groups(
    db: &im_store::SqliteStore,
    groups: &[im_store::group::GroupRow],
) -> Result<
    (
        Vec<im_store::group::GroupRow>,
        std::collections::HashSet<i64>,
    ),
    String,
> {
    db.groups
        .sync_remote_groups(groups)
        .await
        .map_err(|error| error.to_string())?;
    let available_groups = db
        .groups
        .list_all()
        .await
        .map_err(|error| error.to_string())?;
    let monitored = db
        .groups
        .list_monitored()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|group| group.group_id)
        .collect();
    Ok((available_groups, monitored))
}

/// 在群组操作锁内同步远程快照，并刷新内存监控集合。
///
/// 数据库同步与回读完成后才替换内存集合，因此这些本地更新按 `group_ops` 串行；失败时
/// 不执行内存替换。该函数接收已拉取的数据，不覆盖远程请求阶段，也不校验数据所属的
/// uid 或 generation；锁本身不阻止旧认证请求覆盖较新的群组状态。
pub(crate) async fn sync_remote_groups_and_refresh_monitoring(
    group_ops: &tokio::sync::Mutex<()>,
    db: &im_store::SqliteStore,
    monitoring_groups: &tokio::sync::RwLock<std::collections::HashSet<i64>>,
    remote_groups: &[im_store::group::GroupRow],
) -> Result<Vec<im_store::group::GroupRow>, String> {
    let _operation = group_ops.lock().await;
    let (groups, monitored) = apply_remote_groups(db, remote_groups).await?;
    *monitoring_groups.write().await = monitored;
    Ok(groups)
}

/// 先在锁外等待远程群组结果，再串行应用数据库和内存更新。
///
/// 该组合流程不携带认证身份或 generation，远程结果返回后会直接进入本地同步。
async fn fetch_and_apply_remote_groups<F>(
    group_ops: &tokio::sync::Mutex<()>,
    db: &im_store::SqliteStore,
    monitoring_groups: &tokio::sync::RwLock<std::collections::HashSet<i64>>,
    fetch: F,
) -> Result<Vec<im_store::group::GroupRow>, String>
where
    F: Future<Output = Result<Vec<im_store::group::GroupRow>, String>>,
{
    let remote_groups = fetch.await?;
    sync_remote_groups_and_refresh_monitoring(group_ops, db, monitoring_groups, &remote_groups)
        .await
}

/// 切换指定群组的本地监控状态。
///
/// `group_id` 必须是可解析为 i64 的十进制字符串，`monitored` 指定目标状态。该命令只
/// 修改本地数据库和内存监控集合，不调用远程接口。数据库错误、ID 无效或群组不存在时
/// 返回字符串错误；数据库更新失败或未找到群组时不会修改内存集合。
#[tauri::command]
pub async fn toggle_monitor(
    state: State<'_, AppState>,
    group_id: String,
    monitored: bool,
) -> Result<(), String> {
    let group_id = super::parse_i64_id(&group_id, "group_id")?;
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
        .map_err(|error| error.to_string())?;
    toggle_monitor_serialized(
        &state.group_ops,
        &db,
        &state.monitoring_groups,
        group_id,
        monitored,
    )
    .await
}

/// 在群组操作锁内依次更新数据库与内存监控集合。
///
/// 只有数据库确认目标群组存在且更新成功后才修改内存；锁保证它与刷新、重登录恢复等
/// 群组操作按取得锁的顺序串行执行。
pub(crate) async fn toggle_monitor_serialized(
    group_ops: &tokio::sync::Mutex<()>,
    db: &im_store::SqliteStore,
    monitoring_groups: &tokio::sync::RwLock<std::collections::HashSet<i64>>,
    group_id: i64,
    monitored: bool,
) -> Result<(), String> {
    let _operation = group_ops.lock().await;
    let found = db
        .groups
        .toggle_monitored(group_id, monitored)
        .await
        .map_err(|error| error.to_string())?;
    if !found {
        return Err(format!("Group {group_id} not found"));
    }

    let mut monitoring = monitoring_groups.write().await;
    if monitored {
        monitoring.insert(group_id);
    } else {
        monitoring.remove(&group_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use im_store::group::GroupRow;

    use super::{
        fetch_and_apply_remote_groups, sync_remote_groups_and_refresh_monitoring,
        toggle_monitor_serialized, GroupDto,
    };

    #[test]
    fn group_dto_serializes_identifier_fields_as_decimal_strings() {
        let dto = GroupDto::from(GroupRow {
            group_id: i64::MAX,
            name: "Precision".to_string(),
            pic: String::new(),
            host_id: Some(i64::MAX - 1),
            member_count: 3,
            created_at: 4,
            monitored: 1,
            updated_at: 5,
        });
        let json = serde_json::to_value(dto).unwrap();

        assert_eq!(json["group_id"], i64::MAX.to_string());
        assert_eq!(json["host_id"], (i64::MAX - 1).to_string());
    }

    /// 阻塞模拟的远程拉取，验证等待网络结果期间不会占用群组操作锁。
    #[tokio::test]
    async fn remote_fetch_does_not_hold_group_operation_lock() {
        let store = Arc::new(im_store::SqliteStore::new(":memory:").await.unwrap());
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let task_store = store.clone();
        let task_ops = group_ops.clone();
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let task_monitoring = monitoring_groups.clone();
        let sync = tokio::spawn(async move {
            fetch_and_apply_remote_groups(&task_ops, &task_store, &task_monitoring, async move {
                release_rx.await.unwrap();
                Ok(Vec::new())
            })
            .await
        });

        tokio::task::yield_now().await;
        assert!(group_ops.try_lock().is_ok());

        release_tx.send(()).unwrap();
        sync.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn toggle_rejects_missing_group_without_changing_memory() {
        let store = im_store::SqliteStore::new(":memory:").await.unwrap();
        let group_ops = tokio::sync::Mutex::new(());
        let monitoring_groups = tokio::sync::RwLock::new(HashSet::new());

        let error = toggle_monitor_serialized(&group_ops, &store, &monitoring_groups, 999, true)
            .await
            .unwrap_err();

        assert!(error.contains("Group 999 not found"));
        assert!(monitoring_groups.read().await.is_empty());
    }

    /// 连续应用远程快照，验证离开快照的群组暂不可用，重新出现后恢复其持久化监控选择。
    #[tokio::test]
    async fn snapshot_refresh_rebuilds_available_monitored_groups_and_restores_reappearing_group() {
        let store = im_store::SqliteStore::new(":memory:").await.unwrap();
        let group_ops = tokio::sync::Mutex::new(());
        let monitoring_groups = tokio::sync::RwLock::new([7].into_iter().collect());
        let group = |group_id| GroupRow {
            group_id,
            name: format!("Group {group_id}"),
            pic: String::new(),
            host_id: None,
            member_count: 0,
            created_at: 0,
            monitored: 1,
            updated_at: 1,
        };
        store
            .groups
            .sync_remote_groups(&[group(7), group(8)])
            .await
            .unwrap();

        sync_remote_groups_and_refresh_monitoring(
            &group_ops,
            &store,
            &monitoring_groups,
            &[group(8)],
        )
        .await
        .unwrap();
        assert_eq!(*monitoring_groups.read().await, [8].into_iter().collect());

        sync_remote_groups_and_refresh_monitoring(
            &group_ops,
            &store,
            &monitoring_groups,
            &[group(7), group(8)],
        )
        .await
        .unwrap();
        assert_eq!(
            *monitoring_groups.read().await,
            [7, 8].into_iter().collect()
        );
    }

    /// 控制两个并发切换的入锁顺序，验证数据库与内存最终反映后到达的关闭操作。
    #[tokio::test]
    async fn concurrent_toggles_apply_in_mutex_arrival_order() {
        let store = Arc::new(im_store::SqliteStore::new(":memory:").await.unwrap());
        store
            .groups
            .insert_or_update(&GroupRow {
                group_id: 7,
                name: "Group".to_string(),
                pic: String::new(),
                host_id: None,
                member_count: 0,
                created_at: 0,
                monitored: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
        let group_ops = Arc::new(tokio::sync::Mutex::new(()));
        let monitoring_groups = Arc::new(tokio::sync::RwLock::new(HashSet::new()));
        let gate = group_ops.lock().await;

        let first = {
            let store = store.clone();
            let group_ops = group_ops.clone();
            let monitoring_groups = monitoring_groups.clone();
            tokio::spawn(async move {
                toggle_monitor_serialized(&group_ops, &store, &monitoring_groups, 7, true).await
            })
        };
        tokio::task::yield_now().await;
        let second = {
            let store = store.clone();
            let group_ops = group_ops.clone();
            let monitoring_groups = monitoring_groups.clone();
            tokio::spawn(async move {
                toggle_monitor_serialized(&group_ops, &store, &monitoring_groups, 7, false).await
            })
        };
        tokio::task::yield_now().await;
        drop(gate);

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(store.groups.list_all().await.unwrap()[0].monitored, 0);
        assert!(monitoring_groups.read().await.is_empty());
    }
}
