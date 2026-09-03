// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Gt4Fields, LoginResult, PendingValidation, PrimaryLoginType } from '../types/im'
import { useAuth } from './useAuth'

const gt4Fields: Gt4Fields = {
  lotNumber: 'lot',
  captchaOutput: 'output',
  passToken: 'pass',
  genTime: 'time',
}

/** 挂载带可控 IPC 与 GT4 依赖的认证组合式函数。 */
function setupAuth() {
  const backend = {
    sendSmsCode: vi.fn().mockResolvedValue(undefined),
    sendEmailCode: vi.fn().mockResolvedValue(undefined),
    issueValidationToken: vi.fn().mockResolvedValue({
      validateToken: 'issued-token',
      validateTypes: [],
    }),
    verifyValidations: vi.fn().mockImplementation(async (request: {
      pendingValidateDTOS: Array<{ reuseLoginPassword?: boolean }>
    }) => {
      // 默认把登录密码自动复用视为已消费，迫使界面展示密码输入；成功复用由个别用例覆盖。
      if (request.pendingValidateDTOS.some((item) => item.reuseLoginPassword)) {
        throw { kind: 'other', message: '登录密码已被使用，禁止重复消费' }
      }
      return { validateModelVOS: [], businessProcessing: [] }
    }),
    listPendingValidations: vi.fn().mockResolvedValue([]),
    login: vi.fn().mockResolvedValue({
      status: 'success',
      uid: '42',
      groups: [],
    } satisfies LoginResult),
  }
  let gt4Success: ((snapshot: string, fields: Gt4Fields) => void | Promise<void>) | undefined
  const gt4Loading = ref(false)
  const gt4Ready = ref(true)
  const gt4 = {
    loading: gt4Loading,
    ready: gt4Ready,
    error: ref(''),
    initialize: vi.fn(async () => {
      gt4Ready.value = true
      return true
    }),
    show: vi.fn((snapshot, success) => {
      gt4Success = success
      return true
    }),
    reset: vi.fn(),
    destroy: vi.fn(() => {
      gt4Ready.value = false
    }),
  }
  const onLogin = vi.fn()
  let auth!: ReturnType<typeof useAuth>
  const wrapper = mount(defineComponent({
    setup() {
      auth = useAuth(onLogin, { api: backend, gt4 })
      return () => h('div')
    },
  }))
  return {
    auth,
    backend,
    gt4,
    onLogin,
    wrapper,
    succeedGt4: () => gt4Success?.(gt4.show.mock.calls.at(-1)?.[0], gt4Fields),
  }
}

describe('useAuth', () => {
  beforeEach(() => vi.clearAllMocks())
  afterEach(() => {
    vi.useRealTimers()
  })

  it('仅验证码模式展示 GT4，并对冻结账号只发送一次验证码', async () => {
    const { auth, backend, gt4, succeedGt4, wrapper } = setupAuth()
    auth.loginMethod.value = 1
    auth.account.value = '13800138000'

    auth.sendCode()
    auth.account.value = '13900000000'
    await succeedGt4()
    await succeedGt4()

    expect(gt4.show).toHaveBeenCalledWith('13800138000', expect.any(Function))
    expect(backend.sendSmsCode).toHaveBeenCalledTimes(1)
    expect(backend.sendSmsCode).toHaveBeenCalledWith({
      phone: '13800138000',
      countryCode: 86,
      codeType: 1,
      gt4DTO: gt4Fields,
    })
    expect(gt4.destroy).toHaveBeenCalledTimes(1)

    auth.loginMethod.value = 3
    auth.sendCode()
    expect(gt4.show).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('上次成功销毁 GT4 后，再次发送验证码会重新初始化', async () => {
    const { auth, gt4, succeedGt4, wrapper } = setupAuth()
    auth.loginMethod.value = 1
    auth.account.value = '13800138000'
    auth.sendCode()
    await succeedGt4()

    await auth.sendCode()

    expect(gt4.initialize).toHaveBeenCalledTimes(1)
    expect(gt4.show).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })

  it.each([
    [1, 17, 'phone', '13800138000'],
    [2, 16, 'email', 'operator@example.com'],
    [3, 20, 'phone', '13800138000'],
    [4, 21, 'email', 'operator@example.com'],
  ] as const)(
    '登录方式 %s 按顺序执行 issued、verify 与 login',
    async (method, validateType, accountField, account) => {
      const { auth, backend, onLogin, wrapper } = setupAuth()
      auth.loginMethod.value = method as PrimaryLoginType
      auth.account.value = account
      auth.validateValue.value = method <= 2 ? '123456' : 'server-encrypted-value'

      await auth.submitLogin()

      expect(backend.issueValidationToken).toHaveBeenCalledWith({
        validateScene: 5,
        validateTypes: [validateType],
      })
      expect(backend.verifyValidations).toHaveBeenCalledWith({
        validateToken: 'issued-token',
        pendingValidateDTOS: [{
          countryCode: accountField === 'phone' ? 86 : 0,
          account,
          validateType,
          validateValue: auth.validateValue.value,
        }],
      })
      expect(backend.login).toHaveBeenCalledWith({
        loginType: method,
        [accountField]: account,
        countryCode: accountField === 'phone' ? 86 : 0,
        validateToken: 'issued-token',
      })
      expect(onLogin).toHaveBeenCalledWith({
        account: {
          uid: '42',
          displayAccount: '',
          loginType: method,
          hasSavedPassword: false,
          isCurrent: true,
        },
        groups: [],
        warnings: [],
      })
      wrapper.unmount()
    },
  )

  it('补查待验证项、验证选中项，并按映射后的登录方式重试', async () => {
    const { auth, backend, onLogin, wrapper } = setupAuth()
    const pending: PendingValidation[] = [{
      countryCode: 86,
      account: '138****8000',
      accountType: 1,
      validateType: 18,
    }]
    backend.login
      .mockResolvedValueOnce({
        status: 'challenge',
        code: 3114179,
        validateToken: 'challenge-token',
        message: '需要二次验证',
        pending: [],
      })
      .mockResolvedValueOnce({ status: 'success', uid: '42', groups: [] })
    backend.listPendingValidations.mockResolvedValueOnce(pending)
    auth.loginMethod.value = 1
    auth.account.value = '13800138000'
    auth.validateValue.value = '123456'

    await auth.submitLogin()

    expect(backend.listPendingValidations).toHaveBeenCalledWith({
      validateToken: 'challenge-token',
    })
    expect(auth.challengePending.value).toEqual(pending)
    auth.challengeValue.value = 'trade-password-value'
    await auth.submitChallenge()

    expect(backend.verifyValidations).toHaveBeenLastCalledWith({
      validateToken: 'challenge-token',
      pendingValidateDTOS: [{
        countryCode: 86,
        account: '138****8000',
        accountType: 1,
        validateType: 18,
        validateValue: 'trade-password-value',
      }],
    })
    expect(backend.login).toHaveBeenLastCalledWith({
      loginType: 8,
      phone: '13800138000',
      countryCode: 86,
      validateToken: 'challenge-token',
    })
    expect(onLogin).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('邮箱验证码 challenge 通过 GT4 发送验证码', async () => {
    const { auth, backend, gt4, succeedGt4, wrapper } = setupAuth()
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱验证码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()

    await auth.sendChallengeCode()
    auth.account.value = 'changed@example.com'
    await succeedGt4()

    expect(gt4.initialize).toHaveBeenCalledTimes(1)
    expect(gt4.show).toHaveBeenLastCalledWith('operator@example.com', expect.any(Function))
    expect(backend.sendEmailCode).toHaveBeenCalledWith({
      email: 'operator@example.com',
      codeType: 1,
      gt4DTO: gt4Fields,
    })
    expect(auth.notice.value).toBe('二次验证验证码已发送')
    auth.challengeValue.value = '654321'
    await auth.submitChallenge()
    expect(backend.verifyValidations).toHaveBeenLastCalledWith({
      validateToken: 'challenge-token',
      pendingValidateDTOS: [{
        account: 'op***@example.com',
        accountType: 7,
        countryCode: 0,
        validateType: 16,
        validateValue: '654321',
      }],
    })
    expect(backend.login).toHaveBeenLastCalledWith({
      loginType: 2,
      email: 'operator@example.com',
      countryCode: 0,
      validateToken: 'challenge-token',
    })
    wrapper.unmount()
  })

  it('手机验证码 challenge 使用原手机号和国家区号', async () => {
    const { auth, backend, succeedGt4, wrapper } = setupAuth()
    const pending: PendingValidation = {
      countryCode: 86,
      account: '138****8000',
      accountType: 1,
      validateType: 17,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要手机验证码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 3
    auth.account.value = '13800138000'
    auth.countryCode.value = 86
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()

    await auth.sendChallengeCode()
    await succeedGt4()

    expect(backend.sendSmsCode).toHaveBeenCalledWith({
      phone: '13800138000',
      countryCode: 86,
      codeType: 1,
      gt4DTO: gt4Fields,
    })
    auth.challengeValue.value = '654321'
    await auth.submitChallenge()
    expect(backend.login).toHaveBeenLastCalledWith({
      loginType: 1,
      phone: '13800138000',
      countryCode: 86,
      validateToken: 'challenge-token',
    })
    wrapper.unmount()
  })

  it('手机密码 challenge 映射回手机密码登录', async () => {
    const { auth, backend, wrapper } = setupAuth()
    const pending: PendingValidation = {
      countryCode: 86,
      account: '138****8000',
      accountType: 1,
      validateType: 20,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要手机登录密码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 1
    auth.account.value = '13800138000'
    auth.validateValue.value = '123456'
    await auth.submitLogin()
    auth.challengeValue.value = 'plain-password'

    await auth.submitChallenge()

    expect(backend.login).toHaveBeenLastCalledWith({
      loginType: 3,
      phone: '13800138000',
      countryCode: 86,
      validateToken: 'challenge-token',
    })
    wrapper.unmount()
  })

  it('登录报告场景验证项缺失时补查待验证项', async () => {
    const { auth, backend, wrapper } = setupAuth()
    const emailCode: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    const emailPassword: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 21,
    }
    backend.login
      .mockResolvedValueOnce({
        status: 'challenge',
        code: 3114179,
        validateToken: 'challenge-token',
        message: '需要邮箱验证码',
        pending: [emailCode],
      })
      .mockRejectedValueOnce({
        kind: 'business',
        code: 3114169,
        msg: '该场景下验证项缺失',
      })
    backend.listPendingValidations
      .mockResolvedValueOnce([emailCode])
      .mockResolvedValueOnce([emailPassword])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    auth.challengeValue.value = '654321'

    await auth.submitChallenge()

    expect(backend.listPendingValidations).toHaveBeenLastCalledWith({
      validateToken: 'challenge-token',
    })
    expect(auth.challengePending.value).toEqual([emailPassword])
    expect(auth.selectedChallengeType.value).toBe(21)
    expect(auth.notice.value).toContain('该场景下验证项缺失')
    expect(auth.error.value).toBe('')
    auth.challengeValue.value = 'plain-password'
    await auth.submitChallenge()
    expect(backend.login).toHaveBeenLastCalledWith({
      loginType: 4,
      email: 'operator@example.com',
      countryCode: 0,
      validateToken: 'challenge-token',
    })
    wrapper.unmount()
  })

  it('待验证项补查为空时保留原始缺失错误', async () => {
    const { auth, backend, wrapper } = setupAuth()
    backend.login.mockRejectedValueOnce({
      kind: 'business',
      code: 3114169,
      msg: '该场景下验证项缺失',
    })
    backend.listPendingValidations.mockResolvedValueOnce([])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'

    await auth.submitLogin()

    expect(backend.listPendingValidations).toHaveBeenCalledWith({
      validateToken: 'issued-token',
    })
    expect(auth.error.value).toContain('该场景下验证项缺失')
    expect(auth.error.value).not.toMatch(/311\d+/)
    wrapper.unmount()
  })

  it('合并响应与补查项并去重，且补查失败时保留响应项', async () => {
    const first: PendingValidation = {
      countryCode: 86,
      account: '138****8000',
      accountType: 1,
      validateType: 18,
    }
    const second: PendingValidation = {
      account: 'op***@example.com',
      accountType: 2,
      validateType: 23,
    }
    const challenge: LoginResult = {
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要二次验证',
      pending: [first],
    }
    const merged = setupAuth()
    merged.backend.login.mockResolvedValueOnce(challenge)
    merged.backend.listPendingValidations.mockResolvedValueOnce([first, second])
    merged.auth.account.value = '13800138000'
    merged.auth.validateValue.value = '123456'

    await merged.auth.submitLogin()

    expect(merged.auth.challengePending.value).toEqual([first, second])
    merged.wrapper.unmount()

    const failed = setupAuth()
    failed.backend.login.mockResolvedValueOnce(challenge)
    failed.backend.listPendingValidations.mockRejectedValueOnce(new Error('pending unavailable'))
    failed.auth.account.value = '13800138000'
    failed.auth.validateValue.value = '123456'

    await failed.auth.submitLogin()

    expect(failed.auth.challengePending.value).toEqual([first])
    expect(failed.auth.selectedChallengeType.value).toBe(18)
    expect(failed.auth.error.value).toContain('pending unavailable')
    failed.wrapper.unmount()
  })

  it('主验证返回剩余项时停止登录并保留业务提示', async () => {
    const { auth, backend, gt4, wrapper } = setupAuth()
    const remaining: PendingValidation[] = [{
      account: 'op***@example.com',
      accountType: 2,
      validateType: 23,
    }]
    backend.verifyValidations.mockResolvedValueOnce({
      validateModelVOS: remaining,
      businessProcessing: [{ businessCode: 9001, businessMsg: '设备已变更' }],
    })
    auth.account.value = '13800138000'
    auth.validateValue.value = '123456'

    await auth.submitLogin()

    expect(backend.login).not.toHaveBeenCalled()
    expect(auth.challengePending.value).toEqual(remaining)
    expect(auth.businessProcessing.value).toEqual([
      { businessCode: 9001, businessMsg: '设备已变更' },
    ])
    expect(gt4.destroy).toHaveBeenCalled()
    wrapper.unmount()
  })

  it('二次验证仍有待办项时不重试登录', async () => {
    const { auth, backend, wrapper } = setupAuth()
    const initial: PendingValidation = { validateType: 18, account: 'masked' }
    const remaining: PendingValidation[] = [{ validateType: 19, account: 'masked' }]
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要二次验证',
      pending: [initial],
    })
    backend.listPendingValidations.mockResolvedValueOnce([initial])
    auth.account.value = '13800138000'
    auth.validateValue.value = '123456'
    await auth.submitLogin()
    auth.challengeValue.value = 'trade-value'
    backend.verifyValidations.mockResolvedValueOnce({
      validateModelVOS: remaining,
      businessProcessing: [{ businessCode: 9002, businessMsg: '通知' }],
    })

    await auth.submitChallenge()

    expect(backend.login).toHaveBeenCalledTimes(1)
    expect(auth.challengePending.value).toEqual(remaining)
    expect(auth.selectedChallengeType.value).toBe(19)
    expect(auth.businessProcessing.value).toEqual([
      { businessCode: 9002, businessMsg: '通知' },
    ])
    wrapper.unmount()
  })

  it('认证界面错误只展示用户可读文案，不包含业务码', async () => {
    const { auth, backend, wrapper } = setupAuth()
    backend.issueValidationToken.mockRejectedValueOnce({
      kind: 'business',
      code: 3110002,
      msg: '验证码错误',
      title: '认证失败',
      params: ['2'],
      data: { remaining: 2 },
    })
    auth.account.value = '13800138000'
    auth.validateValue.value = '123456'

    await auth.submitLogin()

    expect(auth.error.value).toContain('验证码错误')
    expect(auth.error.value).not.toMatch(/311\d+/)
    expect(auth.error.value).not.toContain('remaining')
    wrapper.unmount()
  })

  it('登录成功且缺少账号摘要时按 uid 合成展示账号', async () => {
    const { auth, onLogin, wrapper } = setupAuth()
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'

    await auth.submitLogin()

    expect(onLogin).toHaveBeenCalledWith({
      account: {
        uid: '42',
        displayAccount: '',
        loginType: 4,
        hasSavedPassword: false,
        isCurrent: true,
      },
      groups: [],
      warnings: [],
    })
    wrapper.unmount()
  })

  it('登录成功时把账号摘要和 warnings 交给 onLogin', async () => {
    const { auth, backend, onLogin, wrapper } = setupAuth()
    backend.login.mockResolvedValueOnce({
      status: 'success',
      uid: '42',
      groups: [],
      account: {
        uid: '42',
        displayAccount: 'a@example.com',
        loginType: 4,
        hasSavedPassword: true,
        isCurrent: true,
      },
      warnings: ['本次无法安全保存登录信息'],
    })
    auth.loginMethod.value = 4
    auth.account.value = 'a@example.com'
    auth.validateValue.value = 'plain-password'

    await auth.submitLogin()

    expect(onLogin).toHaveBeenCalledWith({
      account: {
        uid: '42',
        displayAccount: 'a@example.com',
        loginType: 4,
        hasSavedPassword: true,
        isCurrent: true,
      },
      groups: [],
      warnings: ['本次无法安全保存登录信息'],
    })
    wrapper.unmount()
  })

  it('resetAuthForm 在 challenge 状态后清理瞬态认证状态并保留选中账号', async () => {
    const { auth, backend, wrapper } = setupAuth()

    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'operator@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })

    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要二次验证',
      pending: [pending],
    } satisfies LoginResult)
    backend.listPendingValidations.mockResolvedValueOnce([])

    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()

    expect(auth.validateToken.value).toBe('challenge-token')
    expect(auth.notice.value).toBe('需要二次验证')
    expect(auth.challengePending.value).toEqual([pending])

    auth.businessProcessing.value = [{ businessCode: 9001, businessMsg: 'x' }]
    auth.error.value = 'some error'
    auth.challengeValue.value = '123'
    auth.busy.value = 'some busy'

    auth.resetAuthForm()

    expect(auth.selectedAccountUid.value).toBe('42')
    expect(auth.validateToken.value).toBe('')
    expect(auth.validateValue.value).toBe('')
    expect(auth.notice.value).toBe('')
    expect(auth.error.value).toBe('')
    expect(auth.businessProcessing.value).toEqual([])
    expect(auth.challengePending.value).toEqual([])
    expect(auth.selectedChallengeType.value).toBeNull()
    expect(auth.challengeValue.value).toBe('')
    expect(auth.busy.value).toBeNull()

    wrapper.unmount()
  })

  it('selectSavedAccount 只回填账号摘要，不写入密码明文', () => {
    const { auth, wrapper } = setupAuth()
    auth.validateValue.value = 'should-be-cleared'
    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })

    expect(auth.selectedAccountUid.value).toBe('42')
    expect(auth.account.value).toBe('a@example.com')
    expect(auth.loginMethod.value).toBe(4)
    expect(auth.passwordMode.value).toBe('saved')
    expect(auth.validateValue.value).toBe('')
    expect(JSON.stringify({
      uid: auth.selectedAccountUid.value,
      account: auth.account.value,
      loginMethod: auth.loginMethod.value,
      validateValue: auth.validateValue.value,
    })).not.toContain('saved-secret')
    wrapper.unmount()
  })

  it('选择保存账号时不把密码明文放入前端', async () => {
    const { auth, backend, wrapper } = setupAuth()
    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    expect(auth.passwordMode.value).toBe('saved')
    await auth.submitLogin()
    expect(backend.verifyValidations).toHaveBeenCalledWith(expect.objectContaining({
      pendingValidateDTOS: [expect.objectContaining({ savedPasswordUid: '42' })],
    }))
    wrapper.unmount()
  })

  it('改写密码后切换到手动模式并提交 validateValue', async () => {
    const { auth, backend, wrapper } = setupAuth()
    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    expect(auth.passwordMode.value).toBe('saved')

    auth.validateValue.value = 'typed-password'
    expect(auth.passwordMode.value).toBe('manual')
    await auth.submitLogin()

    expect(backend.verifyValidations).toHaveBeenCalledWith(expect.objectContaining({
      pendingValidateDTOS: [expect.objectContaining({ validateValue: 'typed-password' })],
    }))
    const dto = backend.verifyValidations.mock.calls[0]?.[0].pendingValidateDTOS[0]
    expect(dto).not.toHaveProperty('savedPasswordUid')
    wrapper.unmount()
  })

  it('切换登录方式后不再使用已保存密码', async () => {
    const { auth, backend, wrapper } = setupAuth()
    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    expect(auth.passwordMode.value).toBe('saved')

    auth.loginMethod.value = 3
    expect(auth.passwordMode.value).not.toBe('saved')
    expect(auth.selectedAccountUid.value).toBeNull()

    auth.account.value = '13800138000'
    auth.validateValue.value = 'phone-password'
    await auth.submitLogin()

    expect(backend.verifyValidations).toHaveBeenCalled()
    const dto = backend.verifyValidations.mock.calls[0]?.[0].pendingValidateDTOS[0]
    expect(dto).not.toHaveProperty('savedPasswordUid')
    expect(dto).toEqual(expect.objectContaining({ validateValue: 'phone-password' }))
    wrapper.unmount()
  })

  it('resetChallenge 清空令牌、待办、补全目标和倒计时，不删除已选账号', async () => {
    const { auth, backend, wrapper } = setupAuth()
    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'operator@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要二次验证',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    auth.supplementedTarget.value = 'operator@example.com'
    auth.resendSeconds.value = 40
    auth.challengeValue.value = '123456'

    auth.resetChallenge()

    expect(auth.validateToken.value).toBe('')
    expect(auth.challengePending.value).toEqual([])
    expect(auth.challengeValue.value).toBe('')
    expect(auth.supplementedTarget.value).toBe('')
    expect(auth.resendSeconds.value).toBe(0)
    expect(auth.challengeStep.value).toBe(0)
    expect(auth.completedChallengeKeys.value).toEqual([])
    expect(auth.selectedAccountUid.value).toBe('42')
    expect(auth.account.value).toBe('operator@example.com')
    wrapper.unmount()
  })

  it('进入邮箱验证码挑战时不启动 GT4，发送后才初始化并进入 60 秒倒计时', async () => {
    vi.useFakeTimers()
    const { auth, backend, gt4, succeedGt4, wrapper } = setupAuth()
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱验证码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    await flushPromises()

    expect(gt4.initialize).not.toHaveBeenCalled()
    expect(auth.challengeStep.value).toBe(1)

    await auth.sendChallengeCode()
    await succeedGt4()
    await flushPromises()

    expect(gt4.initialize).toHaveBeenCalledTimes(1)
    expect(backend.sendEmailCode).toHaveBeenCalledTimes(1)
    expect(auth.resendSeconds.value).toBe(60)

    await auth.sendChallengeCode()
    expect(backend.sendEmailCode).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(1000)
    expect(auth.resendSeconds.value).toBe(59)
    await vi.advanceTimersByTimeAsync(59_000)
    expect(auth.resendSeconds.value).toBe(0)

    wrapper.unmount()
    await vi.advanceTimersByTimeAsync(5_000)
  })

  it('脱敏目标必须补全且前后缀一致，补全值不写入账号', async () => {
    const { auth, backend, gt4, succeedGt4, wrapper } = setupAuth()
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱验证码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 3
    auth.account.value = '13800138000'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()

    await auth.sendChallengeCode()
    expect(backend.sendEmailCode).not.toHaveBeenCalled()
    expect(auth.error.value).toContain('完整')

    auth.supplementedTarget.value = 'wrong@other.com'
    await auth.sendChallengeCode()
    expect(backend.sendEmailCode).not.toHaveBeenCalled()
    expect(auth.error.value).toContain('前后缀')

    auth.supplementedTarget.value = 'operator@example.com'
    await auth.sendChallengeCode()
    await succeedGt4()
    await flushPromises()

    expect(gt4.show).toHaveBeenLastCalledWith('operator@example.com', expect.any(Function))
    expect(backend.sendEmailCode).toHaveBeenCalledWith({
      email: 'operator@example.com',
      codeType: 1,
      gt4DTO: gt4Fields,
    })
    expect(auth.account.value).toBe('13800138000')
    expect(auth.selectedAccountUid.value).toBeNull()
    wrapper.unmount()
  })

  it('进入登录密码挑战时自动复用一次，已复用后展示输入且不重试', async () => {
    const { auth, backend, wrapper } = setupAuth()
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 21,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱登录密码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    await flushPromises()

    const reuseCalls = backend.verifyValidations.mock.calls.filter((call) =>
      call[0].pendingValidateDTOS.some((item: { reuseLoginPassword?: boolean }) => item.reuseLoginPassword),
    )
    expect(reuseCalls).toHaveLength(1)
    expect(reuseCalls[0]?.[0]).toEqual({
      validateToken: 'challenge-token',
      pendingValidateDTOS: [expect.objectContaining({
        validateType: 21,
        reuseLoginPassword: true,
      })],
    })
    expect(auth.error.value).toBe('')
    expect(auth.challengePending.value).toEqual([pending])

    await flushPromises()
    const reuseAfterWait = backend.verifyValidations.mock.calls.filter((call) =>
      call[0].pendingValidateDTOS.some((item: { reuseLoginPassword?: boolean }) => item.reuseLoginPassword),
    )
    expect(reuseAfterWait).toHaveLength(1)
    wrapper.unmount()
  })

  it('PasswordAlreadyReused 英文标识同样停止自动复用', async () => {
    const { auth, backend, wrapper } = setupAuth()
    backend.verifyValidations.mockImplementation(async (request: {
      pendingValidateDTOS: Array<{ reuseLoginPassword?: boolean }>
    }) => {
      if (request.pendingValidateDTOS.some((item) => item.reuseLoginPassword)) {
        throw { kind: 'other', message: 'PasswordAlreadyReused: already consumed' }
      }
      return { validateModelVOS: [], businessProcessing: [] }
    })
    const pending: PendingValidation = {
      countryCode: 86,
      account: '138****8000',
      accountType: 1,
      validateType: 20,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要手机登录密码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 3
    auth.account.value = '13800138000'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    await flushPromises()

    expect(auth.challengePending.value).toEqual([pending])
    expect(backend.verifyValidations.mock.calls.filter((call) =>
      call[0].pendingValidateDTOS.some((item: { reuseLoginPassword?: boolean }) => item.reuseLoginPassword),
    )).toHaveLength(1)
    wrapper.unmount()
  })

  it('连续挑战推进步数并记录已完成项', async () => {
    const { auth, backend, wrapper } = setupAuth()
    const initial: PendingValidation = { validateType: 18, account: 'masked' }
    const remaining: PendingValidation[] = [{ validateType: 19, account: 'masked' }]
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要二次验证',
      pending: [initial],
    })
    backend.listPendingValidations.mockResolvedValueOnce([initial])
    auth.account.value = '13800138000'
    auth.validateValue.value = '123456'
    await auth.submitLogin()
    expect(auth.challengeStep.value).toBe(1)

    auth.challengeValue.value = 'trade-value'
    backend.verifyValidations.mockResolvedValueOnce({
      validateModelVOS: remaining,
      businessProcessing: [],
    })
    await auth.submitChallenge()

    expect(auth.challengeStep.value).toBe(2)
    expect(auth.challengePending.value).toEqual(remaining)
    expect(auth.completedChallengeKeys.value.length).toBeGreaterThan(0)
    wrapper.unmount()
  })

  it('resetChallenge 之后忽略过期的自动复用响应', async () => {
    const { auth, backend, wrapper } = setupAuth()
    let finishReuse: ((value: {
      validateModelVOS: PendingValidation[]
      businessProcessing: unknown[]
    }) => void) | undefined
    backend.verifyValidations.mockImplementation(async (request: {
      pendingValidateDTOS: Array<{ reuseLoginPassword?: boolean }>
    }) => {
      if (request.pendingValidateDTOS.some((item) => item.reuseLoginPassword)) {
        return new Promise((resolve) => {
          finishReuse = resolve
        })
      }
      return { validateModelVOS: [], businessProcessing: [] }
    })
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 21,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱登录密码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    const loginPromise = auth.submitLogin()
    await flushPromises()
    expect(auth.challengePending.value).toEqual([pending])

    auth.resetChallenge()
    expect(auth.challengePending.value).toEqual([])
    expect(auth.validateToken.value).toBe('')

    finishReuse?.({
      validateModelVOS: [{ validateType: 19, account: 'stale' }],
      businessProcessing: [],
    })
    await loginPromise
    await flushPromises()

    expect(auth.challengePending.value).toEqual([])
    expect(auth.validateToken.value).toBe('')
    expect(auth.selectedChallengeType.value).toBeNull()
    wrapper.unmount()
  })

  it('主账号已是完整邮箱时不要求补全，但仍校验脱敏前后缀', async () => {
    const { auth, backend, succeedGt4, wrapper } = setupAuth()
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱验证码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()

    expect(auth.needsSupplementedTarget.value).toBe(false)
    await auth.sendChallengeCode()
    await succeedGt4()
    await flushPromises()
    expect(backend.sendEmailCode).toHaveBeenCalledWith({
      email: 'operator@example.com',
      codeType: 1,
      gt4DTO: gt4Fields,
    })
    wrapper.unmount()
  })

  it('主账号与脱敏前后缀不一致时不得直接发送', async () => {
    const { auth, backend, wrapper } = setupAuth()
    const pending: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱验证码',
      pending: [pending],
    })
    backend.listPendingValidations.mockResolvedValueOnce([pending])
    auth.loginMethod.value = 4
    auth.account.value = 'wrong@other.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()

    expect(auth.needsSupplementedTarget.value).toBe(true)
    await auth.sendChallengeCode()
    expect(backend.sendEmailCode).not.toHaveBeenCalled()
    expect(auth.error.value).toContain('完整')
    wrapper.unmount()
  })

  it('切换二次验证方式会清空倒计时和临时输入', async () => {
    vi.useFakeTimers()
    const { auth, backend, succeedGt4, wrapper } = setupAuth()
    const emailCode: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }
    const google: PendingValidation = {
      account: 'op***@example.com',
      accountType: 7,
      validateType: 19,
    }
    backend.login.mockResolvedValueOnce({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要二次验证',
      pending: [emailCode, google],
    })
    backend.listPendingValidations.mockResolvedValueOnce([emailCode, google])
    auth.loginMethod.value = 4
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    await auth.sendChallengeCode()
    await succeedGt4()
    await flushPromises()
    auth.challengeValue.value = '654321'
    auth.supplementedTarget.value = 'operator@example.com'
    expect(auth.resendSeconds.value).toBe(60)

    auth.selectedChallengeType.value = 19
    expect(auth.resendSeconds.value).toBe(0)
    expect(auth.challengeValue.value).toBe('')
    expect(auth.supplementedTarget.value).toBe('')
    wrapper.unmount()
  })

  it('修改账号后不再使用已保存密码', async () => {
    const { auth, backend, wrapper } = setupAuth()
    auth.selectSavedAccount({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    expect(auth.passwordMode.value).toBe('saved')

    auth.account.value = 'other@example.com'
    expect(auth.passwordMode.value).not.toBe('saved')
    expect(auth.selectedAccountUid.value).toBeNull()

    auth.validateValue.value = 'typed-password'
    await auth.submitLogin()

    expect(backend.verifyValidations).toHaveBeenCalled()
    const dto = backend.verifyValidations.mock.calls[0]?.[0].pendingValidateDTOS[0]
    expect(dto).not.toHaveProperty('savedPasswordUid')
    expect(dto).toEqual(expect.objectContaining({ validateValue: 'typed-password' }))
    wrapper.unmount()
  })
})
