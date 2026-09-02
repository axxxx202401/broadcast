/** Tauri 边界返回的群组数据；可能超出 JavaScript 安全整数范围的标识符均使用十进制字符串。 */
export interface GroupDto {
  /** 群组 ID 的十进制字符串表示。 */
  group_id: string
  /** 群组名称。 */
  name: string
  /** 群组头像地址。 */
  pic: string
  /** 群主用户 ID 的十进制字符串表示；服务端未提供时为 `null`。 */
  host_id: string | null
  /** 服务端群组快照中的成员数量。 */
  member_count: number
  /** 群组创建时间；当前远程刷新链路未保留该值时由后端填 `0`。 */
  created_at: number
  /** 本地监控开关的整数表示：`1` 为监控，`0` 为不监控。 */
  monitored: number
  /** 本地群组记录的更新时间；远程同步时为本次拉取完成后的毫秒时间戳。 */
  updated_at: number
}

/** 前端可见的群消息；正文保留原始字节，64 位标识符通过字符串跨越 IPC。 */
export interface MessageDto {
  /** 消息 ID 的十进制字符串表示。 */
  msg_id: string
  /** 群组 ID 的十进制字符串表示。 */
  group_id: string
  /** 发送者用户 ID 的十进制字符串表示。 */
  send_uid: string
  /** 协议定义的消息类型整数。 */
  msg_type: number
  /** 标准 Base64 编码的原始消息正文字节，不保证是 UTF-8 文本。 */
  content_b64: string
  /** 服务端记录的发送时间。 */
  send_time: number
  /** 消息正文的 MD5 摘要。 */
  content_md5: string
  /**
   * 数据库写入时间。2202 实时消息成功写入 SQLite 后才发送事件；事件没有回读或携带
   * INSERT 时生成的值，故为 `null`，不表示消息尚未落库。历史查询会返回已存的写入时间。
   */
  stored_at: number | null
}

/** 随验证码请求转发的 GT4 挑战结果；有效性由服务端判断。 */
export interface Gt4Fields {
  /** 标识本次 GT4 挑战批次的编号。 */
  lotNumber: string
  /** 完成挑战后产生的验证输出。 */
  captchaOutput: string
  /** GT4 返回的通过令牌。 */
  passToken: string
  /** GT4 返回的结果生成时间值；客户端不解析其格式。 */
  genTime: string
}

/** 需要本地账号字段校验的登录方式：手机验证码、邮件验证码、手机密码、邮件密码。 */
export type PrimaryLoginType = 1 | 2 | 3 | 4
/** 当前前端可提交的登录方式；`7`、`8`、`9` 分别对应人脸、交易密码和 Google 验证码。 */
export type LoginType = PrimaryLoginType | 7 | 8 | 9
/** 服务端校验类型整数 `16..26`；具体判定规则由服务端决定。 */
export type ValidateType = 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26

/** 请求向手机号发送认证验证码。 */
export interface SendSmsCodeRequest {
  /** 接收验证码的手机号码。 */
  phone: string
  /** 与手机号一起提交的国家或地区代码。 */
  countryCode: number
  /** 当前流程固定使用的服务端验证码用途分类值。 */
  codeType: 1
  /** JSON 字段名固定为 `gt4DTO` 的挑战结果。 */
  gt4DTO: Gt4Fields
}

/** 请求向邮箱发送认证验证码。 */
export interface SendEmailCodeRequest {
  /** 接收验证码的邮箱地址。 */
  email: string
  /** 当前流程固定使用的服务端验证码用途分类值。 */
  codeType: 1
  /** JSON 字段名固定为 `gt4DTO` 的挑战结果。 */
  gt4DTO: Gt4Fields
}

/** 为登录流程申请校验令牌及可用校验类型。 */
export interface IssuedRequest {
  /** 校验场景；`5` 与 Rust `ValidateScene::Login` 的 serde 整数编码一致。 */
  validateScene: 5
  /**
   * 调用方指定的校验类型集合；当前前端类型要求提供数组。
   * Rust 后端以 `Option` 接收，wire 字段省略时按 `None` 处理，因此协议层并非必填。
   */
  validateTypes: ValidateType[]
}

/** 服务端签发的校验上下文。 */
export interface IssuedResponse {
  /** 关联本轮校验流程、供后续查询和验证使用的令牌。 */
  validateToken: string
  /** 服务端为本轮流程返回的校验类型。 */
  validateTypes: ValidateType[]
}

/**
 * 服务端返回的一项待校验账号上下文；可选字段的组合由服务端流程决定。
 * Rust 的三个 `Option` 字段未跳过序列化，无值时 wire 上为 `null`；当前 TypeScript
 * 声明还允许省略这些字段，覆盖范围比后端实际输出更宽。
 */
export interface PendingValidation {
  /** 服务端随校验项返回的可选国家或地区代码。 */
  countryCode?: number | null
  /** 服务端随校验项返回的可选账号表示。 */
  account?: string | null
  /** 服务端用于区分账号类别的可选整数。 */
  accountType?: number | null
  /** 该项要求使用的校验类型。 */
  validateType: ValidateType
}

/** 提交给服务端的单项校验材料。 */
export interface PendingValidationDto extends PendingValidation {
  /** 验证码、密码摘要或其他材料；密码类值会由后端 IPC 命令按协议规则改写。 */
  validateValue: string
}

/** 向一轮校验流程提交一组校验材料。 */
export interface VerifyRequest {
  /** 标识本轮校验流程的令牌。 */
  validateToken: string
  /** JSON 字段名固定为 `pendingValidateDTOS` 的待验证材料。 */
  pendingValidateDTOS: PendingValidationDto[]
  /** 随整批材料提交的可选补充值；客户端不解释其内容。 */
  secondMac?: string
}

/** 服务端返回的一项业务处理结果；前端不自行解释业务码。 */
export interface BusinessProcessing {
  /** 服务端业务码。 */
  businessCode: number
  /**
   * 服务端随业务码返回的可选说明。Rust `Option<String>` 无值时会在 wire 上序列化为
   * `null`；当前 `businessMsg?: string` 未覆盖 `null`，这是待后续单独修复的既有类型差异。
   */
  businessMsg?: string
}

/** 完成验证后返回的后续校验上下文和业务处理结果。 */
export interface VerifyResponse {
  /** JSON 字段名固定为 `validateModelVOS` 的后续校验项。 */
  validateModelVOS: PendingValidation[]
  /** 服务端业务处理结果。 */
  businessProcessing: BusinessProcessing[]
}

/** 查询一轮校验流程当前待校验项的请求。 */
export interface ListPendingValidationsRequest {
  /** 通常来自签发响应或登录挑战的校验流程令牌。 */
  validateToken: string
}

/** 登录命令参数；除后端明确校验的组合外，其余可选字段由服务端解释。 */
export interface LoginRequest {
  /** 登录方式整数，决定后端执行的本地必填字段检查。 */
  loginType: LoginType
  /** 手机验证码和手机密码方式使用的号码。 */
  phone?: string
  /** 邮件验证码和邮件密码方式使用的地址。 */
  email?: string
  /** 手机方式要求的国家区号；当前邮件流程可能显式传 `0`。 */
  countryCode?: number
  /** 可随登录请求提交的校验流程令牌。 */
  validateToken?: string
  /** 可选补充值；客户端不解释其内容。 */
  secondMac?: string
  /** 人脸方式要求的认证材料。 */
  credentials?: string
}

/** 登录 IPC 的带判别字段结果；字段名与 Rust `LoginResultDto` 的 camelCase serde 输出一致。 */
export type LoginResult =
  | {
      /** 表示认证、群组同步及本地会话发布均已完成。 */
      status: 'success'
      /** 用户 ID 的十进制字符串表示。 */
      uid: string
      /** 本次远程快照同步后得到的本地群组列表。 */
      groups: GroupDto[]
    }
  | {
      /** 表示服务端要求继续校验，本地认证会话尚未发布。 */
      status: 'challenge'
      /** 原样保留的服务端业务码。 */
      code: number
      /** 后续校验请求使用的令牌。 */
      validateToken: string
      /** 服务端返回的提示消息。 */
      message: string
      /** 服务端可选的待完成校验项。 */
      pending?: PendingValidation[]
    }

/** 认证 IPC 的结构化错误；字段名与 Rust `AuthCommandError` 的 camelCase serde 输出一致。 */
export type AuthCommandError =
  | {
      /** 表示服务端返回了可保留字段的业务错误。 */
      kind: 'business'
      /** 服务端业务码，本层不扩写其含义。 */
      code: number
      /** 服务端业务错误消息。 */
      msg: string
      /** 可选的业务错误附加数据。 */
      data?: unknown
      /** 可选的服务端展示方式标记。 */
      display?: number
      /** 可选的服务端展示标题。 */
      title?: string
      /** 可选的消息模板参数。 */
      params?: string[]
    }
  | {
      /** 表示传输、解析、本地校验或状态切换等非业务错误。 */
      kind: 'other'
      /** 可供前端记录或展示的错误文本。 */
      message: string
    }

/** 聊天连接状态；未知后端事件值应由归一化层降级为 `disconnected`。 */
export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected'
