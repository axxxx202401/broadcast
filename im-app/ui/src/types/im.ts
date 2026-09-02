export interface GroupDto {
  group_id: string
  name: string
  pic: string
  host_id: string | null
  member_count: number
  created_at: number
  monitored: number
  updated_at: number
}

export interface MessageDto {
  msg_id: string
  group_id: string
  send_uid: string
  msg_type: number
  content_b64: string
  send_time: number
  content_md5: string
  stored_at: number | null
}

export interface Gt4Fields {
  lotNumber: string
  captchaOutput: string
  passToken: string
  genTime: string
}

export type PrimaryLoginType = 1 | 2 | 3 | 4
export type LoginType = PrimaryLoginType | 7 | 8 | 9
export type ValidateType = 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26

export interface SendSmsCodeRequest {
  phone: string
  countryCode: number
  codeType: 1
  gt4DTO: Gt4Fields
}

export interface SendEmailCodeRequest {
  email: string
  codeType: 1
  gt4DTO: Gt4Fields
}

export interface IssuedRequest {
  validateScene: 5
  validateTypes: ValidateType[]
}

export interface IssuedResponse {
  validateToken: string
  validateTypes: ValidateType[]
}

export interface PendingValidation {
  countryCode?: number | null
  account?: string | null
  accountType?: number | null
  validateType: ValidateType
}

export interface PendingValidationDto extends PendingValidation {
  validateValue: string
}

export interface VerifyRequest {
  validateToken: string
  pendingValidateDTOS: PendingValidationDto[]
  secondMac?: string
}

export interface BusinessProcessing {
  businessCode: number
  businessMsg?: string
}

export interface VerifyResponse {
  validateModelVOS: PendingValidation[]
  businessProcessing: BusinessProcessing[]
}

export interface ListPendingValidationsRequest {
  validateToken: string
}

export interface LoginRequest {
  loginType: LoginType
  phone?: string
  email?: string
  countryCode?: number
  validateToken?: string
  secondMac?: string
  credentials?: string
}

export type LoginResult =
  | { status: 'success'; uid: string; groups: GroupDto[] }
  | {
      status: 'challenge'
      code: number
      validateToken: string
      message: string
      pending?: PendingValidation[]
    }

export type AuthCommandError =
  | {
      kind: 'business'
      code: number
      msg: string
      data?: unknown
      display?: number
      title?: string
      params?: string[]
    }
  | { kind: 'other'; message: string }

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected'
