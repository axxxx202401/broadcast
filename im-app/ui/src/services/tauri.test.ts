import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))

import { api } from './tauri'

const gt4DTO = {
  lotNumber: 'lot',
  captchaOutput: 'output',
  passToken: 'pass',
  genTime: 'time',
}

describe('认证 IPC 契约', () => {
  beforeEach(() => mocks.invoke.mockReset())

  it('将每个认证命令的复杂参数统一包装在 request 字段中', async () => {
    mocks.invoke.mockResolvedValue(undefined)
    const requests = [
      ['send_sms_code', api.sendSmsCode({
        phone: '13800138000',
        countryCode: 86,
        codeType: 1,
        gt4DTO,
      })],
      ['send_email_code', api.sendEmailCode({
        email: 'operator@example.com',
        codeType: 1,
        gt4DTO,
      })],
      ['issue_validation_token', api.issueValidationToken({
        validateScene: 5,
        validateTypes: [17],
      })],
      ['verify_validations', api.verifyValidations({
        validateToken: 'token',
        pendingValidateDTOS: [{
          countryCode: 86,
          account: '13800138000',
          validateType: 17,
          validateValue: '123456',
        }],
      })],
      ['list_pending_validations', api.listPendingValidations({
        validateToken: 'challenge',
      })],
      ['login', api.login({
        loginType: 1,
        phone: '13800138000',
        countryCode: 86,
        validateToken: 'token',
      })],
    ] as const
    await Promise.all(requests.map(([, request]) => request))

    for (const [index, [command]] of requests.entries()) {
      expect(mocks.invoke.mock.calls[index]?.[0]).toBe(command)
      expect(mocks.invoke.mock.calls[index]?.[1]).toHaveProperty('request')
    }
  })

  it('原样保留拒绝结果中的结构化业务错误及附加数据', async () => {
    const businessError = {
      kind: 'business',
      code: 3110002,
      msg: '验证码错误',
      title: '认证失败',
      params: ['2'],
      data: { remaining: 2 },
    }
    mocks.invoke.mockRejectedValueOnce(businessError)

    await expect(api.login({ loginType: 1 })).rejects.toEqual(businessError)
  })

  it('使用 onMessage 参数登记实时消息 Channel', async () => {
    mocks.invoke.mockResolvedValueOnce(undefined)
    const channel = {} as never

    await api.registerMessageChannel(channel)

    expect(mocks.invoke).toHaveBeenCalledWith('register_message_channel', {
      onMessage: channel,
    })
  })

  it('只发送已保存密码标志，不发送密码明文', async () => {
    mocks.invoke.mockResolvedValueOnce({
      validateModelVOS: [],
      businessProcessing: [],
    })

    await api.verifyValidations({
      validateToken: 'issued',
      pendingValidateDTOS: [{
        account: 'a@example.com',
        validateType: 21,
        savedPasswordUid: '42',
      }],
    })

    expect(mocks.invoke).toHaveBeenCalledWith('verify_validations', {
      request: expect.objectContaining({
        pendingValidateDTOS: [expect.not.objectContaining({ validateValue: expect.anything() })],
      }),
    })
    const payload = mocks.invoke.mock.calls[0]?.[1] as {
      request: { pendingValidateDTOS: Array<Record<string, unknown>> }
    }
    expect(payload.request.pendingValidateDTOS[0]).toEqual({
      account: 'a@example.com',
      validateType: 21,
      savedPasswordUid: '42',
    })
    expect(JSON.stringify(payload)).not.toContain('saved-secret')
  })
})

describe('消息分页 IPC 契约', () => {
  beforeEach(() => mocks.invoke.mockReset())

  it('默认请求最近 200 条且不发送游标字段', async () => {
    const page = { messages: [], nextCursor: null, hasMore: false }
    mocks.invoke.mockResolvedValueOnce(page)

    await expect(api.getMessages('7')).resolves.toEqual(page)
    expect(mocks.invoke).toHaveBeenCalledWith('get_messages', {
      groupId: '7',
      limit: 200,
      beforeSendTime: undefined,
      beforeMsgId: undefined,
      matchedOnly: false,
    })
  })

  it('把复合游标拆成 camelCase 参数并保留字符串消息 ID', async () => {
    mocks.invoke.mockResolvedValueOnce({ messages: [], nextCursor: null, hasMore: false })

    await api.getMessages(
      undefined,
      { sendTime: 100, msgId: '9007199254740993' },
      50,
    )

    expect(mocks.invoke).toHaveBeenCalledWith('get_messages', {
      groupId: undefined,
      limit: 50,
      beforeSendTime: 100,
      beforeMsgId: '9007199254740993',
      matchedOnly: false,
    })
  })
})

describe('账号 IPC 契约', () => {
  beforeEach(() => mocks.invoke.mockReset())

  it('账号命令使用正确命令名，且载荷从不包含 password 或 token', async () => {
    mocks.invoke.mockResolvedValue({ warnings: [] })

    await api.restoreSession()
    await api.listAccounts()
    await api.switchAccount('42')
    await api.removeAccount('42')
    await api.logout()

    expect(mocks.invoke.mock.calls[0]).toEqual(['restore_session'])
    expect(mocks.invoke.mock.calls[1]).toEqual(['list_accounts'])
    expect(mocks.invoke.mock.calls[2]).toEqual(['switch_account', { uid: '42' }])
    expect(mocks.invoke.mock.calls[3]).toEqual(['remove_account', { uid: '42' }])
    expect(mocks.invoke.mock.calls[4]).toEqual(['logout'])

    for (const [, payload] of mocks.invoke.mock.calls) {
      const serialized = JSON.stringify(payload ?? {})
      expect(serialized).not.toMatch(/password/i)
      expect(serialized).not.toMatch(/token/i)
    }
  })
})
