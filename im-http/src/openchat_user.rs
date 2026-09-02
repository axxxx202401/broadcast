//! OpenChat 用户认证 HTTP 客户端、请求/响应 DTO 与错误模型。
//!
//! 本模块按 OpenChat 网关协议发送 JSON：请求正文封装为加密网关帧并以
//! `application/octet-stream` 发送，响应正文则依据帧标志选择是否解压和解密。

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

/// OpenChat 用户认证 API 客户端。
///
/// 客户端保存网关地址、HTTP 客户端、正文密码器、请求头生成器及设备参数；调用端点
/// 时会执行真实网络请求。匿名请求与认证请求都会生成 `X-One`、`X-Ten`，后者还会
/// 将访问令牌交给认证请求头生成逻辑。
pub struct OpenChatUserClient {
    base_url: String,
    http: reqwest::Client,
    body_cipher: AesCipher,
    header_manager: HeaderManager,
    device: DeviceConfig,
}

const USER_DETAIL_PATH: &str = "/user/user/userDetail";

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
/// OpenChat API 的通用响应 envelope。
///
/// 直接通过 serde 反序列化 `ApiResponse<T>` 时，serde 会立即按 `T` 解析 `data`，
/// [`ApiResponse::is_success`] 只是响应码判断方法，不会控制该过程。模块内部的
/// `parse_api_response` 会先以 `serde_json::Value` 接收 `data`，仅在 `code == 200`
/// 时再解析为端点的成功类型；非成功响应则转为 [`ApiBusinessError`]。
pub struct ApiResponse<T> {
    /// 服务端响应码；当前协议仅将 `200` 视为成功。
    pub code: i32,
    #[serde(default)]
    /// 服务端返回的消息，缺失时为空字符串。
    pub msg: String,
    #[serde(default)]
    /// 服务端返回的数据；其具体结构由端点和响应码决定。
    pub data: Option<T>,
    #[serde(default)]
    /// 服务端可选的展示控制值，不在客户端解释其业务含义。
    pub display: Option<i32>,
    #[serde(default)]
    /// 服务端可选标题。
    pub title: Option<String>,
    #[serde(default)]
    /// 服务端可选参数列表。
    pub params: Option<Vec<String>>,
}

impl<T> ApiResponse<T> {
    /// 判断响应码是否严格等于 `200`。
    ///
    /// 本方法不改变 `data`，也不参与 serde 的反序列化控制。
    pub fn is_success(&self) -> bool {
        self.code == 200
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("business error {code}: {msg}")]
/// OpenChat 返回非 `200` 响应码时形成的业务错误。
///
/// 该类型保留响应 envelope 中的响应码、消息、原始 `data` 及可选展示字段，便于上层
/// 在不丢失服务端上下文的情况下决定后续流程。
pub struct ApiBusinessError {
    /// 服务端业务响应码。
    pub code: i32,
    /// 服务端业务错误消息。
    pub msg: String,
    /// 未转换为成功 DTO 的原始业务数据。
    pub data: Option<serde_json::Value>,
    /// 服务端可选的展示控制值。
    pub display: Option<i32>,
    /// 服务端可选标题。
    pub title: Option<String>,
    /// 服务端可选参数列表。
    pub params: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
/// OpenChat 用户认证调用可能产生的错误。
pub enum OpenChatUserError {
    /// HTTP、网关帧、加解密或响应大小限制等传输层错误。
    #[error(transparent)]
    Transport(#[from] AppError),
    /// JSON 编码或解码失败。
    #[error("invalid API response: {0}")]
    Decode(#[from] serde_json::Error),
    /// 客户端在发起网络请求前发现请求参数不满足本地约束。
    #[error("invalid request: {0}")]
    Validation(String),
    /// 服务端返回了非 `200` 业务响应，且 envelope 字段保存在错误值中。
    #[error(transparent)]
    Business(#[from] ApiBusinessError),
}

/// 解析 OpenChat 通用响应 envelope。
///
/// `code != 200` 时不把 `data` 反序列化为成功类型，而是返回保留 envelope 字段的
/// [`ApiBusinessError`]；仅在 `code == 200` 时将 `data`（缺失时按 JSON `null`）解析
/// 为目标类型。
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

/// 定义使用 `i32` 作为 serde wire 表示的协议枚举。
///
/// 生成的反序列化实现只接受声明过的整数，未知值直接返回 serde 错误。
macro_rules! integer_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident = $value:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant = $value
            ),+
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
    /// `/sns/login/login` 请求选择的登录方式。
    ///
    /// JSON 中使用 `i32` 编码；未知整数会被拒绝。除 [`LoginReq::validate`] 明确列出的
    /// 本地必填规则外，各方式的认证细节由服务端决定。
    pub enum LoginType {
        /// 手机验证码登录；本地要求 `phone` 非空并提供 `country_code`。
        PhoneCode = 1,
        /// 邮件验证码登录；本地要求 `email` 非空。
        EmailCode = 2,
        /// 手机密码登录；本地要求 `phone` 非空并提供 `country_code`。
        PhonePassword = 3,
        /// 邮件密码登录；本地要求 `email` 非空。
        EmailPassword = 4,
        /// 通过登录端点提交注册流程请求；当前不增加本地必填检查。
        Registration = 5,
        /// 通过登录端点提交 PC 扫码登录请求；当前不增加本地必填检查。
        PcScan = 6,
        /// 人脸方式登录；本地要求 `credentials` 非空。
        Face = 7,
        /// 通过登录端点提交交易密码方式请求；当前不增加本地必填检查。
        TradePassword = 8,
        /// 通过登录端点提交 Google 验证码方式请求；当前不增加本地必填检查。
        GoogleCode = 9,
    }
}

integer_enum! {
    /// `issued`、待校验查询与 `verify` 流程使用的校验方式。
    ///
    /// JSON 中使用 `i32` 编码；未知整数会被拒绝。枚举确定 `validateValue` 所属的
    /// 校验类别，具体判定规则由服务端决定。
    pub enum ValidateType {
        /// 邮件验证码校验；验证码通过 `validateValue` 提交。
        EmailCode = 16,
        /// 手机验证码校验；验证码通过 `validateValue` 提交。
        PhoneCode = 17,
        /// 交易密码校验；应用认证命令在发送前对该值执行双 MD5。
        TradePassword = 18,
        /// Google 验证码校验。
        GoogleCode = 19,
        /// 登录密码校验；应用认证命令在发送前使用登录密码摘要算法处理该值。
        LoginPassword = 20,
        /// 邮件密码校验；应用认证命令在发送前使用登录密码摘要算法处理该值。
        EmailPassword = 21,
        /// 人脸校验。
        FaceVerify = 22,
        /// Messenger 验证码校验。
        MessengerCode = 23,
        /// 协助验证流程使用的校验类型。
        AssistVerify = 24,
        /// iToken 验证码校验。
        ITokenVerifyCode = 25,
        /// iToken 生物特征验证码校验。
        ITokenBiometricVerifyCode = 26,
    }
}

integer_enum! {
    /// `/user/unauthorized/issued` 申请校验令牌时声明的场景。
    ///
    /// JSON 中使用 `i32` 编码；未知整数会被拒绝。
    pub enum ValidateScene {
        /// 为注册流程申请校验信息。
        Register = 4,
        /// 为登录流程申请校验信息。
        Login = 5,
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 随短信或邮件验证码请求提交的 GT4 挑战结果。
///
/// 结构内字段使用 camelCase，外层属性固定为 `gt4DTO`。客户端只转发这些挑战结果，
/// 是否有效由服务端校验。
pub struct Gt4Dto {
    /// 标识本次 GT4 挑战批次的编号。
    pub lot_number: String,
    /// 完成挑战后产生的验证输出。
    pub captcha_output: String,
    /// GT4 返回的通过令牌。
    pub pass_token: String,
    /// GT4 返回的结果生成时间值；客户端不解析其格式。
    pub gen_time: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 请求向指定手机号发送验证码。
///
/// `code_type` 的具体业务分类由服务端决定；客户端将其与目标号码及 GT4 挑战结果
/// 一并提交。
pub struct SendSmsCodeReq {
    /// 接收验证码的手机号码。
    pub phone: String,
    /// 与手机号一起提交的国家或地区代码。
    pub country_code: i32,
    /// 服务端验证码用途分类值，JSON 名为 `codeType`。
    pub code_type: i32,
    #[serde(rename = "gt4DTO")]
    /// 本次发送请求的 GT4 挑战结果；JSON 名固定为 `gt4DTO`。
    pub gt4_dto: Gt4Dto,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 请求向指定邮箱发送验证码。
///
/// `code_type` 的具体业务分类由服务端决定；客户端将其与目标邮箱及 GT4 挑战结果
/// 一并提交。
pub struct SendEmailCodeReq {
    /// 接收验证码的邮箱地址。
    pub email: String,
    /// 服务端验证码用途分类值，JSON 名为 `codeType`。
    pub code_type: i32,
    #[serde(rename = "gt4DTO")]
    /// 本次发送请求的 GT4 挑战结果；JSON 名固定为 `gt4DTO`。
    pub gt4_dto: Gt4Dto,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 为注册或登录流程申请校验令牌及校验类型。
pub struct IssuedReq {
    /// 本次申请所属的注册或登录场景。
    pub validate_scene: ValidateScene,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 调用方指定的校验类型集合；为 `None` 时省略 `validateTypes`，由服务端决定响应。
    pub validate_types: Option<Vec<ValidateType>>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// `issued` 返回的校验上下文。
pub struct IssuedResp {
    /// 关联本轮校验流程的令牌，供待校验查询和 `verify` 请求使用。
    pub validate_token: String,
    #[serde(default)]
    /// 服务端为本轮流程返回的校验类型；缺失时解码为空列表。
    pub validate_types: Vec<ValidateType>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// `verify` 请求中的单项校验材料。
///
/// 账号相关字段并非所有校验类型都需要；具体组合由服务端流程决定。客户端只检查
/// `validate_value` 非空，上层认证命令会对三种密码校验值先做对应摘要。
pub struct PendingValidateDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 账号使用手机号时可随材料提交的国家或地区代码；为 `None` 时省略。
    pub country_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 与本项校验关联的可选账号；为 `None` 时省略。
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 服务端用于区分账号类别的可选整数；为 `None` 时省略 `accountType`。
    pub account_type: Option<i32>,
    /// 指明服务端应如何解释本项 `validate_value`。
    pub validate_type: ValidateType,
    /// 本项验证码、密码摘要或其他校验材料；`verify` 拒绝空白值。
    pub validate_value: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 向校验流程提交一组校验材料。
pub struct VerifyReq {
    /// 标识本轮校验流程的令牌；`verify` 在请求前要求其非空。
    pub validate_token: String,
    #[serde(rename = "pendingValidateDTOS")]
    /// 至少一项待验证材料；JSON 名固定为 `pendingValidateDTOS`。
    pub pending_validate_dtos: Vec<PendingValidateDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 随整批校验材料提交的可选补充值；客户端不解释内容，为 `None` 时省略
    /// `secondMac`。
    pub second_mac: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 服务端返回的一项待校验账号上下文。
///
/// 该结构出现在待校验查询、校验响应及登录挑战数据中。字段的具体业务解释由服务端
/// 决定，客户端将其呈现给上层以构造后续校验请求。
pub struct ValidateModelVo {
    /// 服务端随该待校验项返回的可选国家或地区代码。
    pub country_code: Option<i32>,
    /// 服务端随该待校验项返回的可选账号表示。
    pub account: Option<String>,
    /// 服务端随该待校验项返回的可选账号类别值。
    pub account_type: Option<i32>,
    /// 该待校验项要求的校验类型。
    pub validate_type: ValidateType,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// `verify` 响应中的一项服务端业务处理结果。
///
/// 客户端不解释业务码与消息的具体含义，仅按响应原样承载。
pub struct BusinessProcessingDto {
    /// 服务端为该处理结果返回的业务码。
    pub business_code: i32,
    #[serde(default)]
    /// 与业务码一起返回的可选说明；字段缺失时为 `None`。
    pub business_msg: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// `verify` 完成后返回的校验模型与业务处理结果。
pub struct VerifyResp {
    #[serde(default, rename = "validateModelVOS")]
    /// 服务端返回的后续校验上下文；JSON 名固定为 `validateModelVOS`，缺失时为空列表。
    pub validate_model_vos: Vec<ValidateModelVo>,
    #[serde(default)]
    /// 服务端业务处理结果；字段缺失时为空列表。
    pub business_processing: Vec<BusinessProcessingDto>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 查询一轮校验流程当前待校验项的请求。
pub struct ListPendingValidateReq {
    /// 标识待查询校验流程的令牌，通常来自 [`IssuedResp`] 或登录挑战。
    pub validate_token: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// `/sns/login/login` 的登录参数。
///
/// 可选字段为 `None` 时不写入 JSON。除 [`LoginReq::validate`] 明确检查的组合外，
/// 其余字段是否需要及如何解释由所选登录方式和服务端决定。
pub struct LoginReq {
    /// 选择登录方式，并决定本地必填字段检查。
    pub login_type: LoginType,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 手机验证码和手机密码登录使用的号码；这两种方式要求非空。
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 邮件验证码和邮件密码登录使用的地址；这两种方式要求非空。
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 手机登录随号码提交的国家或地区代码；手机方式要求存在该值。
    pub country_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 可随登录请求提交的校验流程令牌；客户端不要求所有登录方式都提供。
    pub validate_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 可随登录请求提交的补充值；客户端不解释内容或校验其存在性。
    pub second_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 人脸方式要求的认证材料；其他方式是否使用由服务端决定。
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
    /// 按当前客户端规则检查登录请求的必填字段。
    ///
    /// 手机验证码或手机密码登录要求非空 `phone` 和存在 `country_code`；邮件验证码或
    /// 邮件密码登录要求非空 `email`；人脸登录要求非空 `credentials`。注册、扫码、
    /// 交易密码和 Google 验证码登录目前不增加本地必填检查。
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

/// 生成供日志使用的脱敏 JSON 文本。
///
/// 该函数递归遍历对象和数组，并在忽略键名中的 `_`、`-` 及大小写后，对当前明确
/// 列出的令牌、账号、验证码等敏感键替换值。它不承诺覆盖未列入匹配表的其他敏感键；
/// 非 JSON 输入只记录字节数。
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
/// 登录响应中的授权信息。
pub struct Authorization {
    #[serde(
        default,
        rename = "access_token",
        alias = "accessToken",
        alias = "token"
    )]
    /// 访问令牌；反序列化兼容 `access_token`、`accessToken` 和 `token`，序列化使用
    /// `access_token`。
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 登录成功响应中的数据。
///
/// 字段通常按 camelCase 解码；直接令牌字段还兼容 `token`、`access_token` 和
/// `accessToken`。设备状态相关字段的具体业务解释由服务端决定，当前客户端仅保留。
pub struct LoginData {
    /// 服务端可能随登录结果返回的用户标识；缺失时上层可通过用户详情补取。
    pub uid: Option<i64>,
    /// 登录响应携带的可选设备状态标志；当前客户端不据此改变流程。
    pub is_not_last_device_mac: Option<bool>,
    /// 登录响应携带的可选登出状态值；当前客户端原样保留，不解释整数含义。
    pub is_login_out: Option<i32>,
    /// 登录响应携带的可选旧会话标识；当前客户端不使用该值。
    pub old_session_id: Option<String>,
    #[serde(default)]
    /// 服务端可能返回的嵌套授权信息，是 [`LoginData::access_token`] 的首选来源。
    pub authorization: Option<Authorization>,
    #[serde(default, alias = "access_token", alias = "accessToken")]
    /// 服务端直接放在登录数据中的访问令牌，作为嵌套授权信息缺失时的兼容来源；
    /// 反序列化兼容 `token`、`access_token` 和 `accessToken`。
    pub token: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 用户详情中的基础数据。
pub struct UserBase {
    /// 用户详情返回的用户标识；登录响应未给出 `uid` 时，上层用它完成身份初始化。
    pub uid: Option<i64>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// 用户详情端点返回的数据。
pub struct UserDetailResp {
    /// 包含用户标识的基础资料对象，对应响应中的 `userBase`。
    pub user_base: UserBase,
}

impl LoginData {
    /// 返回可用的访问令牌切片。
    ///
    /// 优先读取嵌套 `authorization` 中已通过 `access_token`、`accessToken` 或 `token`
    /// 映射得到的值；没有时再读取登录数据顶层的兼容令牌字段。
    pub fn access_token(&self) -> Option<&str> {
        self.authorization
            .as_ref()
            .and_then(|authorization| authorization.access_token.as_deref())
            .or(self.token.as_deref())
    }
}

impl OpenChatUserClient {
    /// 创建 OpenChat 用户认证客户端。
    ///
    /// `body_aes_key` 在构造时用于初始化网关正文密码器；密钥无效时返回
    /// [`AppError`]。本方法只创建客户端，不发起网络请求。
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

    /// 通过匿名 OpenChat 请求头发送 JSON 字节。
    ///
    /// 该层委托给令牌可选的传输实现，并生成匿名 `X-One`、`X-Ten`。
    async fn post_encrypted(&self, path: &str, json_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
        self.post_encrypted_with_token(path, json_bytes, None).await
    }

    /// 将 JSON 封装为 Gateway 加密帧并执行 POST。
    ///
    /// 请求使用 `application/octet-stream`；无令牌时生成匿名 `X-One`、`X-Ten`，有
    /// 令牌时使用认证请求头生成逻辑。响应先受大小限制，再依据帧中的压缩、加密标志
    /// 选择是否解压和解密，并非每个响应都必经两步。调试日志仅记录经
    /// [`sanitize_debug_json`] 处理的请求、原始响应和解码响应。网络、HTTP 状态、
    /// 帧解析及密码处理失败均返回 [`AppError`]。
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

    /// 序列化请求、经匿名加密传输发送，并解析通用响应 envelope。
    ///
    /// 序列化失败、传输失败、成功数据解码失败或服务端业务错误分别转换为
    /// [`OpenChatUserError`]。
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

    /// 序列化请求、经带令牌的认证加密传输发送，并解析通用响应 envelope。
    ///
    /// 认证传输仍生成 `X-One`、`X-Ten`，日志继续使用统一递归脱敏。
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

    /// 向 `/user/unauthorized/sendSmsCaptchaWithGt4` 发送短信验证码请求。
    ///
    /// 此匿名端点以带 `X-One`、`X-Ten` 的 `application/octet-stream` Gateway 加密帧
    /// 执行真实 POST；请求被服务端接受时会触发向目标手机号发送验证码。响应成功数据
    /// 按 `()` 解析；编码、网络、HTTP、帧或业务失败返回 [`OpenChatUserError`]。若
    /// 请求已到达服务端，即使客户端最终收到错误，也不能据此断定验证码未发送。
    pub async fn send_sms_code(&self, request: &SendSmsCodeReq) -> Result<(), OpenChatUserError> {
        self.post_api("/user/unauthorized/sendSmsCaptchaWithGt4", request)
            .await
    }

    /// 向 `/user/unauthorized/sendEmailCaptchaWithGt4` 发送邮件验证码请求。
    ///
    /// 此匿名端点以带 `X-One`、`X-Ten` 的 `application/octet-stream` Gateway 加密帧
    /// 执行真实 POST；请求被服务端接受时会触发向目标邮箱发送验证码。响应成功数据按
    /// `()` 解析；编码、网络、HTTP、帧或业务失败返回 [`OpenChatUserError`]。若请求
    /// 已到达服务端，即使客户端最终收到错误，也不能据此断定验证码未发送。
    pub async fn send_email_code(
        &self,
        request: &SendEmailCodeReq,
    ) -> Result<(), OpenChatUserError> {
        self.post_api("/user/unauthorized/sendEmailCaptchaWithGt4", request)
            .await
    }

    /// 向 `/user/unauthorized/issued` 申请校验信息。
    ///
    /// 此匿名端点以带 `X-One`、`X-Ten` 的 `application/octet-stream` Gateway 加密帧
    /// 执行真实 POST，请求服务端签发后续查询或验证所用的校验令牌，并将成功数据解析
    /// 为 [`IssuedResp`]。编码、网络、HTTP、帧、业务或响应解码失败返回
    /// [`OpenChatUserError`]；请求到达服务端后，错误响应不保证令牌未被签发。
    pub async fn issued(&self, request: &IssuedReq) -> Result<IssuedResp, OpenChatUserError> {
        self.post_api("/user/unauthorized/issued", request).await
    }

    /// 向 `/user/unauthorized/verify` 提交校验数据。
    ///
    /// 发起网络请求前要求非空 `validate_token`、至少一个待校验项目，且每个
    /// `validate_value` 非空。通过检查后，此匿名端点以带 `X-One`、`X-Ten` 的
    /// `application/octet-stream` Gateway 加密帧执行真实 POST。校验、编码、网络、
    /// HTTP、帧、业务或响应解码失败返回 [`OpenChatUserError`]。服务端接受材料后
    /// 可能推进该校验流程并返回业务处理结果；客户端报错不代表远程流程一定未变化。
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

    /// 向 `/user/unauthorized/listPedingValidate` 查询待校验项目。
    ///
    /// 路径中的 `Peding` 是服务端 API 的既有拼写，为端点兼容而保留，并非 JSON
    /// wire 字段兼容。此匿名端点以带 `X-One`、`X-Ten` 的
    /// `application/octet-stream` Gateway 加密帧执行真实 POST。编码、网络、HTTP、
    /// 帧、业务或响应解码失败返回 [`OpenChatUserError`]。该调用读取指定校验流程
    /// 当前返回的待校验模型，客户端自身不修改本地认证状态。
    pub async fn list_pending_validations(
        &self,
        request: &ListPendingValidateReq,
    ) -> Result<Vec<ValidateModelVo>, OpenChatUserError> {
        self.post_api("/user/unauthorized/listPedingValidate", request)
            .await
    }

    /// 向 `/sns/login/login` 发起登录。
    ///
    /// 先执行 [`LoginReq::validate`]，通过后以匿名 `X-One`、`X-Ten` 和
    /// `application/octet-stream` Gateway 加密帧执行真实 POST。校验、编码、网络、
    /// HTTP、帧、业务或响应解码失败返回 [`OpenChatUserError`]。服务端接受登录后
    /// 可能建立授权并返回访问令牌，或返回需要继续校验的业务数据；客户端报错不保证
    /// 远程登录处理未发生。
    pub async fn login(&self, request: &LoginReq) -> Result<LoginData, OpenChatUserError> {
        request.validate().map_err(OpenChatUserError::Validation)?;
        self.post_api("/sns/login/login", request).await
    }

    /// 使用访问令牌向 `/user/user/userDetail` 查询用户详情。
    ///
    /// 空白令牌会在网络请求前被拒绝；有效令牌交给认证请求头生成逻辑，生成
    /// `X-One`、`X-Ten` 后，以 `application/octet-stream` Gateway 加密帧执行真实
    /// POST，请求 JSON 正文固定为 `{}`。该调用读取用户基础资料，客户端自身不修改
    /// 本地认证状态；编码、网络、HTTP、帧、业务或响应解码失败返回
    /// [`OpenChatUserError`]。
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
        // 锁定整数 wire 表示，并确认未知值不会静默映射为已有枚举项。
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
        // 同时覆盖常规 camelCase 与服务端约定的 DTO 大写缩写字段名。
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
        // 服务端历史响应使用过三种令牌键名，三者应汇聚到同一读取入口。
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
        // 非 200 数据可能承载后续流程所需上下文，不能按成功 DTO 解码或丢弃。
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
        // 锁定 validateModelVOS 的特殊缩写大小写及嵌套业务处理对象映射。
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
        // 验证递归对象中的已知认证、账号和校验值不会进入调试文本。
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
        // 用本地监听器观察真实请求头，避免依赖外部服务或暴露实际凭据。
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
