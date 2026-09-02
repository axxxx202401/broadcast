use im_common::{config::AppConfig, version_key::VersionKeyManager};
use super::openchat_user::OpenChatUserClient;
use super::im_biz::ImBizClient;

pub struct AppHttpClients {
    pub openchat_user: OpenChatUserClient,
    pub im_biz: ImBizClient,
}

impl AppHttpClients {
    pub fn new(config: &AppConfig) -> Self {
        let version_mgr_u = VersionKeyManager::new(
            config.server.version_secret_name.clone(),
            config.server.header_aes_key.clone(),
        );
        let version_mgr_b = VersionKeyManager::new(
            config.server.version_secret_name.clone(),
            config.server.header_aes_key.clone(),
        );
        Self {
            openchat_user: OpenChatUserClient::new(
                config.server.openchat_user_url.clone(),
                config.server.body_aes_key.clone(),
                version_mgr_u,
            ),
            im_biz: ImBizClient::new(
                config.server.im_biz_url.clone(),
                config.server.body_aes_key.clone(),
                version_mgr_b,
            ),
        }
    }
}
