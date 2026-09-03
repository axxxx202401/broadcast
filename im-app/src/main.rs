//! 桌面应用入口：初始化日志、持久化存储与共享状态，注册 Tauri 命令并运行事件循环。

mod commands;
mod message_content;
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
            // 真实服务参数来自构建时环境快照；缺项或格式错误时拒绝启动，避免安装包
            // 静默连接测试环境或使用历史密钥。
            let config = im_common::config::AppConfig::from_build_env()?;
            // 数据库默认位于 ~/.im-monitor/im_monitor.db；无法取得 HOME 时退回当前目录。
            // setup 先创建父目录，再按 SqliteStore 契约打开数据库：文件缺失时可创建，随后依次
            // 建表、迁移旧版 groups 表并建立索引。文件系统步骤和这些 SQL 未组成整体事务；
            // 初始化失败时，已创建的目录、数据库文件或部分 schema 可能保留。
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
