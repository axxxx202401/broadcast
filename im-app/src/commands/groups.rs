use std::future::Future;

use crate::state::AppState;
use tauri::State;

#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupDto {
    pub group_id: String,
    pub name: String,
    pub pic: String,
    pub host_id: Option<String>,
    pub member_count: i64,
    pub created_at: i64,
    pub monitored: i32,
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

#[tauri::command]
pub async fn fetch_group_list(state: State<'_, AppState>) -> Result<Vec<GroupDto>, String> {
    let groups = state
        .db
        .groups
        .list_all()
        .await
        .map_err(|e| e.to_string())?;
    Ok(group_dtos(groups))
}

#[tauri::command]
pub async fn refresh_group_list(state: State<'_, AppState>) -> Result<Vec<GroupDto>, String> {
    let token = state
        .auth_session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())?
        .token;
    let groups = fetch_and_apply_remote_groups(
        &state.group_ops,
        &state.db,
        &state.monitoring_groups,
        fetch_remote_groups(&state, &token),
    )
    .await?;
    Ok(group_dtos(groups))
}

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

#[tauri::command]
pub async fn toggle_monitor(
    state: State<'_, AppState>,
    group_id: String,
    monitored: bool,
) -> Result<(), String> {
    let group_id = super::parse_i64_id(&group_id, "group_id")?;
    toggle_monitor_serialized(
        &state.group_ops,
        &state.db,
        &state.monitoring_groups,
        group_id,
        monitored,
    )
    .await
}

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
