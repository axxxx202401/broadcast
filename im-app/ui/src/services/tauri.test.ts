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

describe('authentication IPC', () => {
  beforeEach(() => mocks.invoke.mockReset())

  it('wraps every backend auth command argument in request', async () => {
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

  it('preserves structured business error fields including data', async () => {
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
})
