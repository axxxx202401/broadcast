import { invoke, type Channel } from '@tauri-apps/api/core'

import type {
  AttachmentDownloadDto,
  GroupDto,
  IssuedRequest,
  IssuedResponse,
  ListPendingValidationsRequest,
  LoginRequest,
  LoginResult,
  MessageCursor,
  MessageDto,
  MessagePage,
  PendingValidation,
  SendEmailCodeRequest,
  SendSmsCodeRequest,
  VerifyRequest,
  VerifyResponse,
} from '../types/im'

/** 前端使用的 Tauri IPC 服务集合；各方法保持后端命令名、参数包装和返回类型契约。 */
export const api = {
  /**
   * 登记持续接收已入库实时消息批次的 Tauri Channel。
   * 新调用会替换后端保存的旧页面接收端，适配 WebView 重载；失败表示实时推送未就绪，
   * 但不影响后端继续接收和持久化消息。
   */
  registerMessageChannel: (onMessage: Channel<MessageDto[]>) =>
    invoke<void>('register_message_channel', { onMessage }),
  /**
   * 调用 `send_sms_code`，以 `{ request }` 包装参数。
   * 成功无返回值，失败拒绝为结构化认证错误；会请求远程发送短信，但错误不证明短信未发送。
   */
  sendSmsCode: (request: SendSmsCodeRequest) =>
    invoke<void>('send_sms_code', { request }),
  /**
   * 调用 `send_email_code`，以 `{ request }` 包装参数。
   * 成功无返回值，失败拒绝为结构化认证错误；会请求远程发送邮件，但错误不证明邮件未发送。
   */
  sendEmailCode: (request: SendEmailCodeRequest) =>
    invoke<void>('send_email_code', { request }),
  /**
   * 调用 `issue_validation_token`，以 `{ request }` 包装参数并返回远程签发结果。
   * 失败拒绝为结构化认证错误；请求可能推进远程校验流程，错误不证明远程状态未改变。
   */
  issueValidationToken: (request: IssuedRequest) =>
    invoke<IssuedResponse>('issue_validation_token', { request }),
  /**
   * 调用 `verify_validations`，以 `{ request }` 包装参数并返回远程验证结果。
   * 后端会改写密码类材料后请求远程验证；失败为结构化认证错误，但不证明远程状态未改变。
   */
  verifyValidations: (request: VerifyRequest) =>
    invoke<VerifyResponse>('verify_validations', { request }),
  /**
   * 调用 `list_pending_validations`，以 `{ request }` 包装参数并返回远程待校验项。
   * 失败拒绝为结构化认证错误；该命令不修改本地认证会话或连接状态。
   */
  listPendingValidations: (request: ListPendingValidationsRequest) =>
    invoke<PendingValidation[]>('list_pending_validations', { request }),
  /**
   * 调用 `login`，以 `{ request }` 包装参数并返回成功或挑战结果。
   * 它会执行远程认证及群组读取，并切换本地会话、数据库群组和自动连接状态；失败可能来自任一阶段，
   * 且不能据此断定远程认证状态未创建。
   */
  login: (request: LoginRequest) =>
    invoke<LoginResult>('login', { request }),
  /**
   * 无参数调用 `logout`，成功无返回值，失败拒绝为字符串错误。
   * 仅在本地取消连接并清空认证、监控和连接状态，不调用远程登出接口。
   */
  logout: () => invoke<void>('logout'),
  /**
   * 无参数调用 `fetch_group_list`，从本地数据库返回群组列表。
   * 查询失败拒绝为字符串错误；不发起远程请求，也不修改数据库或监控集合。
   */
  fetchGroups: () => invoke<GroupDto[]>('fetch_group_list'),
  /**
   * 无参数调用 `refresh_group_list`，拉取远程群组并更新本地数据库及监控快照。
   * 失败拒绝为字符串错误；远程读取与本地写入不是原子事务，错误也不证明远程请求未完成。
   */
  refreshGroups: () => invoke<GroupDto[]>('refresh_group_list'),
  /**
   * 调用 `toggle_monitor`，直接包装为 `{ groupId, monitored }`，成功无返回值。
   * 仅切换本地数据库和内存监控集合；ID、数据库或群组不存在错误以字符串拒绝。
   */
  toggleMonitor: (groupId: string, monitored: boolean) =>
    invoke<void>('toggle_monitor', { groupId, monitored }),
  /**
   * 无参数调用 `connect_chat` 建立聊天 TCP 连接，并更新本地状态及发送连接状态事件。
   * 成功无返回值；未登录、超时、取消或协议错误等以字符串拒绝，失败清理受断开超时约束。
   */
  connectChat: () => invoke<void>('connect_chat'),
  /**
   * 无参数调用 `disconnect_chat`，取消当前连接代并发布断开状态。
   * 成功无返回值；协调或断开超时以字符串拒绝，超时不表示底层资源已完成优雅断开。
   */
  disconnectChat: () => invoke<void>('disconnect_chat'),
  /**
   * 无参数调用 `get_connection_status`，只读取后端协调器并返回三种连接状态之一。
   * 当前后端命令恒返回规范状态且无网络、数据库或事件副作用；本包装不捕获或转换
   * `invoke` 的 Tauri transport 失败。
   */
  getConnectionStatus: () => invoke<'connected' | 'connecting' | 'disconnected'>('get_connection_status'),
  /**
   * 调用 `get_messages`，把可选复合游标拆为 camelCase IPC 参数；`limit` 默认 `200`。
   * `groupId` 省略时读取全部受监控群组；查询后端会按需请求群密钥并解密正文，
   * 单条解密失败通过 DTO 返回，不修改连接或原始消息。
   */
  getMessages: (groupId?: string, cursor?: MessageCursor, limit = 200) =>
    invoke<MessagePage>('get_messages', {
      groupId,
      limit,
      beforeSendTime: cursor?.sendTime,
      beforeMsgId: cursor?.msgId,
    }),
  /**
   * 下载并解密指定消息的主附件或缩略图，返回可转换为 asset URL 的本地缓存路径。
   * 该调用可能发起远程 OSS 下载并写入媒体缓存，失败不会修改原消息。
   */
  downloadMessageAttachment: (msgId: string, thumbnail = false) =>
    invoke<AttachmentDownloadDto>('download_message_attachment', { msgId, thumbnail }),
}
