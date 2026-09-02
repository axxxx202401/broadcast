use super::{
    client::{build_gateway_request_body, parse_gateway_response},
    http_clients::{read_response_body_limited, MAX_HTTP_RESPONSE_SIZE},
};
use im_common::aes::AesCipher;
use im_common::config::DeviceConfig;
use im_common::error::AppError;
use im_common::version_key::HeaderManager;

#[cfg(debug_assertions)]
use std::time::Instant;

pub struct OpenChatUserClient {
    base_url: String,
    http: reqwest::Client,
    body_cipher: AesCipher,
    header_manager: HeaderManager,
    device: DeviceConfig,
}

const USER_DETAIL_PATH: &str = "/user/user/userDetail";

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Option<T>,
    #[serde(default)]
    pub display: Option<i32>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub params: Option<Vec<String>>,
}

impl<T> ApiResponse<T> {
    pub fn is_success(&self) -> bool {
        self.code == 200
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("business error {code}: {msg}")]
pub struct ApiBusinessError {
    pub code: i32,
    pub msg: String,
    pub data: Option<serde_json::Value>,
    pub display: Option<i32>,
    pub title: Option<String>,
    pub params: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenChatUserError {
    #[error(transparent)]
    Transport(#[from] AppError),
    #[error("invalid API response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid request: {0}")]
    Validation(String),
    #[error(transparent)]
    Business(#[from] ApiBusinessError),
}

fn parse_api_response<T>(bytes: &[u8]) -> Result<T, OpenChatUserError>
where
    T: serde::de::DeserializeOwned,
{
    let response: ApiResponse<serde_json::Value> = serde_json::from_slice(bytes)?;
    if !response.is_success() {
        return Err(ApiBusinessError {
            code: response.code,
            msg: response.msg,
            data: response.data,
            display: response.display,
            title: response.title,
            params: response.params,
        }
        .into());
    }

    serde_json::from_value(response.data.unwrap_or(serde_json::Value::Null))
        .map_err(OpenChatUserError::from)
}

macro_rules! integer_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident = $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant = $value),+
        }

        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_i32(*self as i32)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <i32 as serde::Deserialize>::deserialize(deserializer)?;
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(serde::de::Error::custom(format!(
                        "invalid {} value: {value}",
                        stringify!($name)
                    ))),
                }
            }
        }
    };
}

integer_enum! {
    pub enum LoginType {
        PhoneCode = 1,
        EmailCode = 2,
        PhonePassword = 3,
        EmailPassword = 4,
        Registration = 5,
        PcScan = 6,
        Face = 7,
        TradePassword = 8,
        GoogleCode = 9,
    }
}

integer_enum! {
    pub enum ValidateType {
        EmailCode = 16,
        PhoneCode = 17,
        TradePassword = 18,
        GoogleCode = 19,
        LoginPassword = 20,
        EmailPassword = 21,
        FaceVerify = 22,
        MessengerCode = 23,
        AssistVerify = 24,
        ITokenVerifyCode = 25,
        ITokenBiometricVerifyCode = 26,
    }
}

integer_enum! {
    pub enum ValidateScene {
        Register = 4,
        Login = 5,
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gt4Dto {
    pub lot_number: String,
    pub captcha_output: String,
    pub pass_token: String,
    pub gen_time: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendSmsCodeReq {
    pub phone: String,
    pub country_code: i32,
    pub code_type: i32,
    #[serde(rename = "gt4DTO")]
    pub gt4_dto: Gt4Dto,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendEmailCodeReq {
    pub email: String,
    pub code_type: i32,
    #[serde(rename = "gt4DTO")]
    pub gt4_dto: Gt4Dto,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedReq {
    pub validate_scene: ValidateScene,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_types: Option<Vec<ValidateType>>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedResp {
    pub validate_token: String,
    #[serde(default)]
    pub validate_types: Vec<ValidateType>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingValidateDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<i32>,
    pub validate_type: ValidateType,
    pub validate_value: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReq {
    pub validate_token: String,
    #[serde(rename = "pendingValidateDTOS")]
    pub pending_validate_dtos: Vec<PendingValidateDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second_mac: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateModelVo {
    pub country_code: Option<i32>,
    pub account: Option<String>,
    pub account_type: Option<i32>,
    pub validate_type: ValidateType,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BusinessProcessingDto {
    pub business_code: i32,
    #[serde(default)]
    pub business_msg: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResp {
    #[serde(default, rename = "validateModelVOS")]
    pub validate_model_vos: Vec<ValidateModelVo>,
    #[serde(default)]
    pub business_processing: Vec<BusinessProcessingDto>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPendingValidateReq {
    pub validate_token: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginReq {
    pub login_type: LoginType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

impl Default for LoginReq {
    fn default() -> Self {
        Self {
            login_type: LoginType::PhoneCode,
            phone: None,
            email: None,
            country_code: None,
            validate_token: None,
            second_mac: None,
            credentials: None,
        }
    }
}

impl LoginReq {
    pub fn validate(&self) -> Result<(), String> {
        match self.login_type {
            LoginType::PhoneCode | LoginType::PhonePassword => {
                require_non_empty(&self.phone, "phone")?;
                if self.country_code.is_none() {
                    return Err("countryCode is required for phone login".to_string());
                }
            }
            LoginType::EmailCode | LoginType::EmailPassword => {
                require_non_empty(&self.email, "email")?;
            }
            LoginType::Face => {
                require_non_empty(&self.credentials, "credentials")?;
            }
            LoginType::Registration
            | LoginType::PcScan
            | LoginType::TradePassword
            | LoginType::GoogleCode => {}
        }
        Ok(())
    }
}

fn require_non_empty(value: &Option<String>, field: &str) -> Result<(), String> {
    if value
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        Ok(())
    } else {
        Err(format!("{field} is required"))
    }
}

pub(crate) fn sanitize_debug_json(bytes: &[u8]) -> String {
    fn redact(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                for (key, value) in fields {
                    let normalized = key
                        .chars()
                        .filter(|character| !matches!(character, '_' | '-'))
                        .collect::<String>()
                        .to_ascii_lowercase();
                    if matches!(
                        normalized.as_str(),
                        "account"
                            | "accesstoken"
                            | "captchaoutput"
                            | "credentials"
                            | "email"
                            | "gentime"
                            | "lotnumber"
                            | "password"
                            | "passtoken"
                            | "phone"
                            | "refreshtoken"
                            | "sessionid"
                            | "sysmac"
                            | "token"
                            | "uid"
                            | "validatetoken"
                            | "validatevalue"
                    ) {
                        *value = serde_json::Value::String("<redacted>".to_string());
                    } else {
                        redact(value);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    redact(value);
                }
            }
            _ => {}
        }
    }

    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(mut value) => {
            redact(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| "<JSON serialization failed>".into())
        }
        Err(_) => format!("<non-JSON body: {} bytes>", bytes.len()),
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Authorization {
    #[serde(
        default,
        rename = "access_token",
        alias = "accessToken",
        alias = "token"
    )]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub uid: Option<i64>,
    pub is_not_last_device_mac: Option<bool>,
    pub is_login_out: Option<i32>,
    pub old_session_id: Option<String>,
    #[serde(default)]
    pub authorization: Option<Authorization>,
    #[serde(default, alias = "access_token", alias = "accessToken")]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserBase {
    pub uid: Option<i64>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDetailResp {
    pub user_base: UserBase,
}

impl LoginData {
    pub fn access_token(&self) -> Option<&str> {
        self.authorization
            .as_ref()
            .and_then(|authorization| authorization.access_token.as_deref())
            .or(self.token.as_deref())
    }
}

impl OpenChatUserClient {
    pub fn new(
        http: reqwest::Client,
        base_url: String,
        body_aes_key: String,
        header_manager: HeaderManager,
        device: DeviceConfig,
    ) -> Result<Self, AppError> {
        Ok(Self {
            base_url,
            http,
            body_cipher: AesCipher::try_new(body_aes_key.as_bytes())?,
            header_manager,
            device,
        })
    }

    async fn post_encrypted(&self, path: &str, json_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
        self.post_encrypted_with_token(path, json_bytes, None).await
    }

    async fn post_encrypted_with_token(
        &self,
        path: &str,
        json_bytes: &[u8],
        token: Option<&str>,
    ) -> Result<Vec<u8>, AppError> {
        #[cfg(debug_assertions)]
        let started_at = Instant::now();
        let body = build_gateway_request_body(&self.body_cipher, json_bytes)?;
        let (x_one, x_ten) = match token {
            Some(token) => {
                self.header_manager
                    .build_authenticated_openchat_headers(&self.device, token, "")
            }
            None => self.header_manager.build_openchat_headers(&self.device),
        }
        .map_err(|e| AppError::Http(e.to_string()))?;
        let url = format!("{}{}", self.base_url, path);

        #[cfg(debug_assertions)]
        tracing::debug!(
            method = "POST",
            %path,
            app_ver = self.device.app_ver,
            package_code = self.device.package_code,
            plat = self.device.plat,
            frame_byte_0 = format_args!("0x{:02X}", body[0]),
            frame_byte_1 = format_args!("0x{:02X}", body[1]),
            declared_body_len = u32::from_be_bytes(body[2..6].try_into().unwrap_or_default()),
            wire_len = body.len(),
            json_len = json_bytes.len(),
            x_one_len = x_one.len(),
            x_ten_len = x_ten.len(),
            request = %sanitize_debug_json(json_bytes),
            "OpenChat request"
        );

        let resp = self
            .http
            .post(url)
            .header("X-One", x_one)
            .header("X-Ten", x_ten)
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(|error| AppError::Http(format!("POST {path} request failed: {error}")))?;

        let status = resp.status();
        #[cfg(debug_assertions)]
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<missing>")
            .to_string();
        let data = read_response_body_limited(resp, MAX_HTTP_RESPONSE_SIZE).await?;

        #[cfg(debug_assertions)]
        tracing::debug!(
            method = "POST",
            %path,
            %status,
            %content_type,
            response_len = data.len(),
            elapsed_ms = started_at.elapsed().as_millis(),
            response = %sanitize_debug_json(&data),
            "OpenChat raw response"
        );

        if !status.is_success() {
            return Err(AppError::Http(format!(
                "POST {path} -> HTTP {}: {}",
                status,
                sanitize_debug_json(&data)
            )));
        }

        let decoded = parse_gateway_response(&self.body_cipher, &data).map_err(|error| {
            AppError::Http(format!("POST {path} response decode failed: {error}"))
        })?;

        #[cfg(debug_assertions)]
        tracing::debug!(
            method = "POST",
            %path,
            decoded_len = decoded.len(),
            response = %sanitize_debug_json(&decoded),
            "OpenChat decoded response"
        );

        Ok(decoded)
    }

    async fn post_api<Req, Resp>(
        &self,
        path: &str,
        request: &Req,
    ) -> Result<Resp, OpenChatUserError>
    where
        Req: serde::Serialize + ?Sized,
        Resp: serde::de::DeserializeOwned,
    {
        let json_bytes = serde_json::to_vec(request)?;
        let data = self.post_encrypted(path, &json_bytes).await?;
        parse_api_response(&data)
    }

    async fn post_authenticated_api<Req, Resp>(
        &self,
        path: &str,
        request: &Req,
        token: &str,
    ) -> Result<Resp, OpenChatUserError>
    where
        Req: serde::Serialize + ?Sized,
        Resp: serde::de::DeserializeOwned,
    {
        let json_bytes = serde_json::to_vec(request)?;
        let data = self
            .post_encrypted_with_token(path, &json_bytes, Some(token))
            .await?;
        parse_api_response(&data)
    }

    pub async fn send_sms_code(&self, request: &SendSmsCodeReq) -> Result<(), OpenChatUserError> {
        self.post_api("/user/unauthorized/sendSmsCaptchaWithGt4", request)
            .await
    }

    pub async fn send_email_code(
        &self,
        request: &SendEmailCodeReq,
    ) -> Result<(), OpenChatUserError> {
        self.post_api("/user/unauthorized/sendEmailCaptchaWithGt4", request)
            .await
    }

    pub async fn issued(&self, request: &IssuedReq) -> Result<IssuedResp, OpenChatUserError> {
        self.post_api("/user/unauthorized/issued", request).await
    }

    pub async fn verify(&self, request: &VerifyReq) -> Result<VerifyResp, OpenChatUserError> {
        if request.validate_token.trim().is_empty() {
            return Err(OpenChatUserError::Validation(
                "validateToken is required".to_string(),
            ));
        }
        if request.pending_validate_dtos.is_empty() {
            return Err(OpenChatUserError::Validation(
                "pendingValidateDTOS must not be empty".to_string(),
            ));
        }
        if request
            .pending_validate_dtos
            .iter()
            .any(|pending| pending.validate_value.trim().is_empty())
        {
            return Err(OpenChatUserError::Validation(
                "validateValue must not be empty".to_string(),
            ));
        }

        self.post_api("/user/unauthorized/verify", request).await
    }

    pub async fn list_pending_validations(
        &self,
        request: &ListPendingValidateReq,
    ) -> Result<Vec<ValidateModelVo>, OpenChatUserError> {
        self.post_api("/user/unauthorized/listPedingValidate", request)
            .await
    }

    pub async fn login(&self, request: &LoginReq) -> Result<LoginData, OpenChatUserError> {
        request.validate().map_err(OpenChatUserError::Validation)?;
        self.post_api("/sns/login/login", request).await
    }

    pub async fn user_detail(&self, token: &str) -> Result<UserDetailResp, OpenChatUserError> {
        if token.trim().is_empty() {
            return Err(OpenChatUserError::Validation(
                "access token is required".to_string(),
            ));
        }
        self.post_authenticated_api(USER_DETAIL_PATH, &serde_json::json!({}), token)
            .await
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use im_common::config::DeviceConfig;
    use im_common::version_key::HeaderManager;

    use super::{
        parse_api_response, sanitize_debug_json, ApiResponse, Authorization, Gt4Dto, IssuedReq,
        LoginData, LoginReq, LoginType, OpenChatUserClient, OpenChatUserError, PendingValidateDto,
        SendEmailCodeReq, SendSmsCodeReq, UserDetailResp, ValidateScene, ValidateType, VerifyReq,
        VerifyResp, USER_DETAIL_PATH,
    };

    #[test]
    fn response_success_matches_java_code_200_contract() {
        let success: ApiResponse<()> = serde_json::from_value(serde_json::json!({
            "code": 200,
            "msg": "成功",
            "data": null,
            "display": 0,
            "title": null,
            "params": ["x"]
        }))
        .unwrap();
        let failure: ApiResponse<()> = serde_json::from_value(serde_json::json!({
            "code": 3114179,
            "msg": "需要二次验证",
            "data": null
        }))
        .unwrap();

        assert!(success.is_success());
        assert!(!failure.is_success());
        assert_eq!(success.display, Some(0));
        assert_eq!(success.params, Some(vec!["x".to_string()]));
    }

    #[test]
    fn integer_enums_cover_java_contract_and_reject_unknown_values() {
        assert_eq!(serde_json::to_value(LoginType::GoogleCode).unwrap(), 9);
        assert_eq!(serde_json::to_value(ValidateType::EmailCode).unwrap(), 16);
        assert_eq!(
            serde_json::to_value(ValidateType::ITokenBiometricVerifyCode).unwrap(),
            26
        );
        assert_eq!(serde_json::to_value(ValidateScene::Register).unwrap(), 4);
        assert_eq!(
            serde_json::from_value::<ValidateScene>(serde_json::json!(5)).unwrap(),
            ValidateScene::Login
        );
        assert!(serde_json::from_value::<LoginType>(serde_json::json!(10)).is_err());
        assert!(serde_json::from_value::<ValidateType>(serde_json::json!(15)).is_err());
        assert!(serde_json::from_value::<ValidateScene>(serde_json::json!(3)).is_err());
    }

    #[test]
    fn request_dtos_serialize_documented_camel_case_wire_fields() {
        let gt4 = Gt4Dto {
            lot_number: "lot".to_string(),
            captcha_output: "output".to_string(),
            pass_token: "pass".to_string(),
            gen_time: "time".to_string(),
        };
        let sms = SendSmsCodeReq {
            phone: "13800138000".to_string(),
            country_code: 86,
            code_type: 1,
            gt4_dto: gt4.clone(),
        };
        let email = SendEmailCodeReq {
            email: "test@example.com".to_string(),
            code_type: 1,
            gt4_dto: gt4,
        };
        let issued = IssuedReq {
            validate_scene: ValidateScene::Login,
            validate_types: Some(vec![ValidateType::PhoneCode]),
        };
        let verify = VerifyReq {
            validate_token: "validation-id".to_string(),
            pending_validate_dtos: vec![PendingValidateDto {
                country_code: Some(86),
                account: Some("13800138000".to_string()),
                account_type: None,
                validate_type: ValidateType::PhoneCode,
                validate_value: "123456".to_string(),
            }],
            second_mac: Some("device-feature".to_string()),
        };

        assert_eq!(
            serde_json::to_value(sms).unwrap(),
            serde_json::json!({
                "phone": "13800138000",
                "countryCode": 86,
                "codeType": 1,
                "gt4DTO": {
                    "lotNumber": "lot",
                    "captchaOutput": "output",
                    "passToken": "pass",
                    "genTime": "time"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(email).unwrap()["gt4DTO"]["lotNumber"],
            "lot"
        );
        assert_eq!(
            serde_json::to_value(issued).unwrap(),
            serde_json::json!({
                "validateScene": 5,
                "validateTypes": [17]
            })
        );
        assert_eq!(
            serde_json::to_value(verify).unwrap(),
            serde_json::json!({
                "validateToken": "validation-id",
                "pendingValidateDTOS": [{
                    "countryCode": 86,
                    "account": "13800138000",
                    "validateType": 17,
                    "validateValue": "123456"
                }],
                "secondMac": "device-feature"
            })
        );
    }

    #[test]
    fn login_request_validates_account_requirements() {
        let phone = LoginReq {
            login_type: LoginType::PhonePassword,
            phone: Some("13800138000".to_string()),
            country_code: Some(86),
            ..Default::default()
        };
        let email = LoginReq {
            login_type: LoginType::EmailPassword,
            email: Some("test@example.com".to_string()),
            ..Default::default()
        };
        let face_without_credentials = LoginReq {
            login_type: LoginType::Face,
            phone: Some("13800138000".to_string()),
            country_code: Some(86),
            ..Default::default()
        };

        assert!(phone.validate().is_ok());
        assert!(email.validate().is_ok());
        assert!(LoginReq {
            country_code: None,
            ..phone
        }
        .validate()
        .unwrap_err()
        .contains("countryCode"));
        assert!(LoginReq {
            email: None,
            ..email
        }
        .validate()
        .unwrap_err()
        .contains("email"));
        assert!(face_without_credentials
            .validate()
            .unwrap_err()
            .contains("credentials"));
    }

    #[test]
    fn login_request_skips_absent_fields_and_never_contains_password() {
        let request = LoginReq {
            login_type: LoginType::PhonePassword,
            phone: Some("13800138000".to_string()),
            country_code: Some(86),
            validate_token: Some("validation-id".to_string()),
            ..Default::default()
        };
        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "phone": "13800138000",
                "countryCode": 86,
                "loginType": 3,
                "validateToken": "validation-id"
            })
        );
        assert!(value.get("password").is_none());
    }

    #[test]
    fn login_data_maps_java_fields_and_authorization_token_aliases() {
        for (field, token) in [
            ("access_token", "snake"),
            ("accessToken", "camel"),
            ("token", "short"),
        ] {
            let login: LoginData = serde_json::from_value(serde_json::json!({
                "uid": 7,
                "isNotLastDeviceMac": false,
                "isLoginOut": 0,
                "authorization": { field: token }
            }))
            .unwrap();

            assert_eq!(login.uid, Some(7));
            assert_eq!(login.old_session_id, None);
            assert_eq!(login.access_token(), Some(token));
        }

        let direct = LoginData {
            authorization: Some(Authorization {
                access_token: Some("direct".to_string()),
            }),
            ..Default::default()
        };
        assert_eq!(direct.access_token(), Some("direct"));
    }

    #[test]
    fn user_detail_maps_uid_from_user_base() {
        let detail: UserDetailResp = serde_json::from_value(serde_json::json!({
            "userBase": {
                "uid": 941546
            }
        }))
        .unwrap();

        assert_eq!(detail.user_base.uid, Some(941546));
    }

    #[test]
    fn user_detail_uses_gateway_prefixed_path() {
        assert_eq!(USER_DETAIL_PATH, "/user/user/userDetail");
    }

    #[test]
    fn business_error_preserves_code_message_and_challenge_data() {
        let error = parse_api_response::<LoginData>(
            br#"{
                "code": 3114179,
                "msg": "secondary validation required",
                "data": {
                    "validateToken": "challenge-token",
                    "validateModelVOS": [{
                        "countryCode": 86,
                        "account": "138****8000",
                        "accountType": 1,
                        "validateType": 17
                    }]
                },
                "display": 0,
                "title": null
            }"#,
        )
        .unwrap_err();

        let OpenChatUserError::Business(error) = error else {
            panic!("expected business error");
        };
        assert_eq!(error.code, 3114179);
        assert_eq!(error.msg, "secondary validation required");
        assert_eq!(
            error.data.as_ref().unwrap()["validateToken"],
            "challenge-token"
        );
    }

    #[test]
    fn verify_response_maps_business_processing_objects() {
        let response: VerifyResp = serde_json::from_value(serde_json::json!({
            "validateModelVOS": [],
            "businessProcessing": [{
                "businessCode": 3116029,
                "businessMsg": "device changed"
            }]
        }))
        .unwrap();

        assert_eq!(response.business_processing.len(), 1);
        assert_eq!(response.business_processing[0].business_code, 3116029);
        assert_eq!(
            response.business_processing[0].business_msg.as_deref(),
            Some("device changed")
        );
    }

    #[test]
    fn invalid_body_key_returns_constructor_error() {
        let header_manager =
            HeaderManager::new("secret".to_string(), "1234567890abcdef".to_string());

        let error = OpenChatUserClient::new(
            reqwest::Client::new(),
            "https://example.invalid".to_string(),
            "short".to_string(),
            header_manager,
            DeviceConfig::default(),
        )
        .err()
        .unwrap();

        assert!(error.to_string().contains("16 bytes"));
    }

    #[test]
    fn debug_json_redacts_authentication_and_account_values() {
        let sanitized = sanitize_debug_json(
            br#"{
                "phone":"13800138000",
                "email":"user@example.com",
                "access_token":"access-secret",
                "refresh_token":"refresh-secret",
                "validateToken":"validation-id",
                "pendingValidateDTOS":[{
                    "validateType":17,
                    "validateValue":"123456"
                }],
                "gt4DTO":{
                    "lotNumber":"lot",
                    "captchaOutput":"output",
                    "passToken":"pass",
                    "genTime":"time"
                },
                "loginType":1,
                "countryCode":86
            }"#,
        );

        assert!(!sanitized.contains("13800138000"));
        assert!(!sanitized.contains("user@example.com"));
        assert!(!sanitized.contains("access-secret"));
        assert!(!sanitized.contains("refresh-secret"));
        assert!(!sanitized.contains("validation-id"));
        assert!(!sanitized.contains("123456"));
        assert!(!sanitized.contains("\"lot\""));
        assert!(sanitized.contains("\"loginType\":1"));
        assert!(sanitized.contains("\"countryCode\":86"));
        assert!(sanitized.contains("<redacted>"));
    }

    #[tokio::test]
    async fn encrypted_openchat_request_includes_x_ten() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let count = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&request[..count]).into_owned()
        });
        let header_manager =
            HeaderManager::new("secret".to_string(), "1234567890abcdef".to_string());
        let client = OpenChatUserClient::new(
            reqwest::Client::new(),
            format!("http://{address}"),
            "97b1f52761ffc7f8".to_string(),
            header_manager,
            DeviceConfig::default(),
        )
        .unwrap();

        let error = client
            .post_encrypted("/test", br#"{"test":true}"#)
            .await
            .unwrap_err();
        let request = server.await.unwrap().to_ascii_lowercase();

        assert!(request.contains("\r\nx-ten: "));
        assert!(error.to_string().contains("POST /test"));
        assert!(error.to_string().contains("500"));
    }
}
