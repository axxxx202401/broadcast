//! 桌面应用入口：初始化日志、持久化存储与共享状态，注册 Tauri 命令并运行事件循环。

mod commands;
mod state;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;

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

    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.app_handle();
            let config = im_common::config::AppConfig::default();
            // 数据库默认位于 ~/.im-monitor/im_monitor.db；无法取得 HOME 时退回当前目录。
            let db_path = std::env::var("HOME")
                .map(|home| {
                    std::path::PathBuf::from(home)
                        .join(".im-monitor")
                        .join("im_monitor.db")
                })
                .unwrap_or_else(|_| std::path::PathBuf::from("im_monitor.db"));
            if let Some(parent) = db_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(|error| {
                    std::io::Error::other(format!(
                        "failed to create database directory {}: {error}",
                        parent.display()
                    ))
                })?;
            }
            let db_path_text = db_path.to_string_lossy();
            let db = futures::executor::block_on(async {
                im_store::SqliteStore::new(&db_path_text).await
            })
            .map_err(|error| {
                std::io::Error::other(format!(
                    "failed to initialize SQLite database {}: {error}",
                    db_path.display()
                ))
            })?;
            // 在构造 AppState 前恢复监控集合，避免命令看到尚未加载的初始快照。
            let monitoring_groups = futures::executor::block_on(state::load_monitoring_groups(&db))
                .map_err(|error| {
                    std::io::Error::other(format!(
                        "failed to restore monitored groups from {}: {error}",
                        db_path.display()
                    ))
                })?;
            let http = Arc::new(im_http::http_clients::AppHttpClients::new(&config)?);
            // AppState 按配置、数据库、会话与连接协作组件的依赖顺序组装；
            // Arc 让 Tauri 命令及其后台任务共享同一份客户端、锁和协调器。
            let state = AppState {
                config: Arc::new(tokio::sync::RwLock::new(config)),
                db: Arc::new(db),
                chat_client: Arc::new(tokio::sync::Mutex::new(None)),
                auth_session: Arc::new(tokio::sync::RwLock::new(None)),
                monitoring_groups: Arc::new(tokio::sync::RwLock::new(monitoring_groups)),
                group_ops: Arc::new(tokio::sync::Mutex::new(())),
                connection_coordinator: Arc::new(state::ConnectionCoordinator::new()),
                http,
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
            // 群组查询与监控配置（3 项）。
            commands::groups::fetch_group_list,
            commands::groups::refresh_group_list,
            commands::groups::toggle_monitor,
            // 聊天连接、状态与消息（4 项）。
            commands::chat::connect_chat,
            commands::chat::disconnect_chat,
            commands::chat::get_connection_status,
            commands::chat::get_messages,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                // 退出请求触发全局取消令牌，使连接及消息后台任务开始收尾。
                app_handle.state::<AppState>().shutdown.cancel();
            }
        });
}
