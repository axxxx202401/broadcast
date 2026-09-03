//! 桌面应用入口：初始化日志、账号基础设施与共享状态，注册 Tauri 命令并运行事件循环。

mod account;
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
            // 数据根目录固定为用户主目录下的 `.im-monitor`。启动时只创建该目录和账号
            // 基础设施，不打开业务 SQLite，也不从磁盘恢复监控集合。
            let data_root = app
                .path()
                .home_dir()
                .map_err(|error| {
                    std::io::Error::other(format!("failed to resolve user home directory: {error}"))
                })?
                .join(".im-monitor");
            std::fs::create_dir_all(&data_root).map_err(|error| {
                std::io::Error::other(format!(
                    "failed to create data directory {}: {error}",
                    data_root.display()
                ))
            })?;
            let paths = account::AppPaths::new(data_root);
            let account_index = Arc::new(account::AccountIndexStore::new(paths.index_file()));
            let credentials: Arc<dyn account::CredentialStore> =
                Arc::new(account::KeyringCredentialStore);
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
            // 多账号恢复、列表、切换与移除（4 项）。
            commands::accounts::restore_session,
            commands::accounts::list_accounts,
            commands::accounts::switch_account,
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
