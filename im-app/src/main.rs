#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 桌面应用入口：初始化日志、账号基础设施与共享状态，注册 Tauri 命令并运行事件循环。

mod account;
mod commands;
mod message_content;
mod state;

use account::CredentialStore;
use state::AppState;
use std::io::Write;
use std::sync::Arc;
use tauri::Manager;

/// 从本机硬件信息生成确定性设备标识。
///
/// - macOS：`system_profiler SPHardwareDataType` 的 Hardware UUID
/// - Linux：`/sys/class/dmi/id/product_uuid`
/// - Windows：WMI `Win32_ComputerSystemProduct.UUID`
///
/// 任一方式失败时返回 `None`，由调用方回退到随机 UUID 并写入磁盘。
fn hardware_device_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("system_profiler")
            .arg("SPHardwareDataType")
            .output()
            .ok()?;
        String::from_utf8(output.stdout)
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.contains("Hardware UUID"))
                    .and_then(|l| l.split(':').nth(1).map(|v| v.trim().to_string()))
            })
            .filter(|v| !v.is_empty())
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/sys/class/dmi/id/product_uuid")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|v| !v.is_empty())
    }
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("wmic")
            .args(["computersystemproduct", "get", "uuid"])
            .output()
            .ok()?;
        String::from_utf8(output.stdout)
            .ok()
            .and_then(|s| {
                s.lines()
                    .skip(1)
                    .next()
                    .map(|v| v.trim().to_string())
            })
            .filter(|v| !v.is_empty())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = ();
        None
    }
}

#[tokio::main]
async fn main() {
    // 默认记录 im-app 的 info 级别日志；调试构建额外打开 HTTP 与聊天模块的 debug 日志。
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("im_app=info".parse().unwrap());
    #[cfg(debug_assertions)]
    let env_filter = env_filter
        .add_directive("im_http=debug".parse().unwrap())
        .add_directive("im_chat=debug".parse().unwrap());

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    // 凭据库必须在 Tauri setup 之前异步打开：setup 运行在 Tokio runtime 内，不得嵌套 block_on。
    let data_root = match account::AppPaths::default_data_root() {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(error = %error, "Failed to resolve monitor data root; application cannot store credentials");
            eprintln!("IM Monitor: failed to resolve data directory: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&data_root) {
        tracing::error!(path = %data_root.display(), error = %error, "Failed to create data directory");
        eprintln!("IM Monitor: failed to create data directory {}: {error}", data_root.display());
        return;
    }
    let paths = account::AppPaths::new(data_root);
    let credentials: Arc<dyn CredentialStore> = match account::SqliteCredentialStore::open(&paths).await {
        Ok(store) => Arc::new(store),
        Err(error) => {
            tracing::error!(error = %error, "Failed to open credential store");
            eprintln!("IM Monitor: failed to open credential store: {error}");
            return;
        }
    };

    // sys_mac 持久化到磁盘，保证同一设备重装后仍使用相同设备标识。
    // 优先级：IM_SYS_MAC 环境变量 > 磁盘文件 > 硬件标识 > 随机 fallback。
    let sys_mac_path = paths.credential_key_file().with_file_name("sys_mac");
    let persisted_mac =
        std::fs::read_to_string(&sys_mac_path).ok().filter(|s| !s.trim().is_empty());
    let sys_mac = if let Some(mac) = std::env::var("IM_SYS_MAC").ok().filter(|s| !s.is_empty()) {
        mac
    } else if let Some(mac) = persisted_mac {
        mac
    } else {
        // 用硬件信息生成确定性 ID，删除文件后重新生成仍与原来相同。
        hardware_device_id()
            .or_else(|| Some(uuid::Uuid::new_v4().to_string()))
            .inspect(|mac| {
                if let Ok(mut file) = std::fs::File::create(&sys_mac_path) {
                    let _ = write!(file, "{mac}");
                }
            })
            .unwrap_or_default()
    };

    tauri::Builder::default()
        .setup(move |app| {
            let app_handle = app.app_handle();
            // 真实服务参数来自构建时环境快照；缺项或格式错误时拒绝启动，避免安装包
            // 静默连接测试环境或使用历史密钥。
            let mut config = im_common::config::AppConfig::from_build_env()?;
            // 构建时未注入 IM_SYS_MAC 时，用运行时持久化值覆盖，保证设备标识稳定。
            if option_env!("IM_SYS_MAC").is_none() {
                config.device.sys_mac = sys_mac.clone();
            }
            let account_index = Arc::new(account::AccountIndexStore::new(paths.index_file()));
            let account_db = Arc::new(account::AccountDatabaseManager::new(paths.clone()));
            let legacy_migrator = Arc::new(account::LegacyDatabaseMigrator::new(paths.clone()));
            let pending_login = Arc::new(account::PendingLoginCache::default());
            let http = Arc::new(im_http::http_clients::AppHttpClients::new(&config)?);
            // AppState 按配置、账号服务、会话与连接协作组件的依赖顺序组装；
            // Arc 让 Tauri 命令及其后台任务共享同一份客户端、锁和协调器。
            let state = AppState {
                config: Arc::new(tokio::sync::RwLock::new(config)),
                paths,
                account_index,
                credentials,
                account_db,
                legacy_migrator,
                pending_login,
                chat_client: Arc::new(tokio::sync::Mutex::new(None)),
                auth_session: Arc::new(tokio::sync::RwLock::new(None)),
                monitoring_groups: Arc::new(tokio::sync::RwLock::new(
                    std::collections::HashSet::new(),
                )),
                group_ops: Arc::new(tokio::sync::Mutex::new(())),
                connection_coordinator: Arc::new(state::ConnectionCoordinator::new()),
                http,
                message_crypto: Arc::new(message_content::MessageCryptoState::default()),
                message_channel: Arc::new(tokio::sync::RwLock::new(None)),
                connected: Arc::new(tokio::sync::RwLock::new(false)),
                shutdown: tokio_util::sync::CancellationToken::new(),
                app_handle: Some(app_handle.clone()),
            };
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 认证与验证（7 项）。
            commands::auth::login,
            commands::auth::logout,
            commands::auth::send_sms_code,
            commands::auth::send_email_code,
            commands::auth::issue_validation_token,
            commands::auth::verify_validations,
            commands::auth::list_pending_validations,
            // 多账号恢复、列表、切换、暂停与移除（5 项）。
            commands::accounts::restore_session,
            commands::accounts::list_accounts,
            commands::accounts::switch_account,
            commands::accounts::pause_session,
            commands::accounts::remove_account,
            // 群组查询与监控配置（3 项）。
            commands::groups::fetch_group_list,
            commands::groups::refresh_group_list,
            commands::groups::toggle_monitor,
            // 聊天连接、状态与消息（6 项）。
            commands::chat::register_message_channel,
            commands::chat::connect_chat,
            commands::chat::disconnect_chat,
            commands::chat::get_connection_status,
            commands::chat::get_messages,
            commands::chat::download_message_attachment,
            // 开奖配置与历史。
            commands::lottery::get_lottery_config,
            commands::lottery::set_lottery_config,
            commands::lottery::fetch_lottery_history,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                // 退出前尝试清理过期消息，作为登录路径之外的兜底。
                let state = app_handle.state::<AppState>();
                let account_db = state.account_db.clone();
                let cleanup_fut = async move {
                    if let Ok(db) = account_db.active().await {
                        let cutoff = chrono::Utc::now()
                            .timestamp_millis()
                            .saturating_sub(im_store::message::MESSAGE_RETENTION_DAYS as i64 * 24 * 3600 * 1000);
                        let _ = db.messages.cleanup_old_messages(cutoff).await;
                    }
                };
                tokio::spawn(cleanup_fut);
                state.shutdown.cancel();
            }
        });
}
