use im_common::config::AppConfig;
use im_http::{
    http_clients::AppHttpClients,
    openchat_user::{IssuedReq, ValidateScene, ValidateType},
};

#[tokio::test]
#[ignore = "calls the configured live OpenChat environment"]
async fn live_issued_endpoint_accepts_680_client() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("im_http=debug")
        .with_test_writer()
        .try_init();

    let config = AppConfig::default();
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
