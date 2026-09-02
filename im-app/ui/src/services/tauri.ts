import { invoke } from '@tauri-apps/api/core'

import type {
  GroupDto,
  IssuedRequest,
  IssuedResponse,
  ListPendingValidationsRequest,
  LoginRequest,
  LoginResult,
  MessageDto,
  PendingValidation,
  SendEmailCodeRequest,
  SendSmsCodeRequest,
  VerifyRequest,
  VerifyResponse,
} from '../types/im'

export const api = {
  sendSmsCode: (request: SendSmsCodeRequest) =>
    invoke<void>('send_sms_code', { request }),
  sendEmailCode: (request: SendEmailCodeRequest) =>
    invoke<void>('send_email_code', { request }),
  issueValidationToken: (request: IssuedRequest) =>
    invoke<IssuedResponse>('issue_validation_token', { request }),
  verifyValidations: (request: VerifyRequest) =>
    invoke<VerifyResponse>('verify_validations', { request }),
  listPendingValidations: (request: ListPendingValidationsRequest) =>
    invoke<PendingValidation[]>('list_pending_validations', { request }),
  login: (request: LoginRequest) =>
    invoke<LoginResult>('login', { request }),
  logout: () => invoke<void>('logout'),
  fetchGroups: () => invoke<GroupDto[]>('fetch_group_list'),
  refreshGroups: () => invoke<GroupDto[]>('refresh_group_list'),
  toggleMonitor: (groupId: string, monitored: boolean) =>
    invoke<void>('toggle_monitor', { groupId, monitored }),
  connectChat: () => invoke<void>('connect_chat'),
  disconnectChat: () => invoke<void>('disconnect_chat'),
  getConnectionStatus: () => invoke<'connected' | 'connecting' | 'disconnected'>('get_connection_status'),
  getMessages: (groupId: string, limit = 200, offset = 0) =>
    invoke<MessageDto[]>('get_messages', { groupId, limit, offset }),
}
