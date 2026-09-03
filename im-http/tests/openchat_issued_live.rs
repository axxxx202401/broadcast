//! OpenChat `issued` 端点的人工联调测试。
//!
//! 测试默认 `ignored`，通过 [`AppConfig::from_build_env`] 使用构建时注入的环境并访问
//! 真实网络，仅用于显式联调，不作为常规验证的硬门槛。测试代码和配置不得加入真实
//! 账号、访问令牌、验证码或其他凭据。

use im_common::config::AppConfig;
use im_http::{
    http_clients::AppHttpClients,
    openchat_user::{IssuedReq, ValidateScene, ValidateType},
};

#[tokio::test]
#[ignore = "calls the configured live OpenChat environment"]
async fn live_issued_endpoint_accepts_680_client() {
    // 只确认当前默认客户端参数能被真实端点接受，不断言环境相关的校验策略。
    let _ = tracing_subscriber::fmt()
        .with_env_filter("im_http=debug")
        .with_test_writer()
        .try_init();

    let config =
        AppConfig::from_build_env().expect("live test requires complete build environment");
    assert_eq!(config.device.app_ver, 680);
    assert_eq!(config.device.package_code, 9803);

    let clients = AppHttpClients::new(&config).expect("live HTTP clients should initialize");
    let response = clients
        .openchat_user
        .issued(&IssuedReq {
            validate_scene: ValidateScene::Login,
            validate_types: Some(vec![ValidateType::PhoneCode]),
        })
        .await
        .expect("live /user/unauthorized/issued request should succeed");

    assert!(
        !response.validate_token.trim().is_empty(),
        "issued response must contain validateToken"
    );
    assert!(
        response.validate_types.is_empty()
            || response.validate_types.contains(&ValidateType::PhoneCode),
        "issued response returned unexpected validation types"
    );
}
