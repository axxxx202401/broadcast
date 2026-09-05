// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { computed, defineComponent, h, ref } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useAuth } from '../composables/useAuth'
import type { Gt4Fields, LoginResult, PendingValidation } from '../types/im'
import LoginPanel from './LoginPanel.vue'
import panelSource from './LoginPanel.vue?raw'

const gt4Fields: Gt4Fields = {
  lotNumber: 'lot',
  captchaOutput: 'output',
  passToken: 'pass',
  genTime: 'time',
}

/** 挂载登录面板并注入最小认证桩。 */
function mountPanel(auth: ReturnType<typeof authStub>, accounts: Array<{
  uid: string
  displayAccount: string
  loginType: 1 | 2 | 3 | 4
  hasSavedPassword: boolean
  isCurrent: boolean
}> = []) {
  return mount(LoginPanel, {
    props: {
      auth: auth as never,
      accounts,
      selectedAccountUid: auth.selectedAccountUid.value,
    },
  })
}

function authStub(withChallenge = false) {
  const loginMethod = ref<1 | 2 | 3 | 4>(4)
  const selectedAccountUid = ref<string | null>(null)
  const passwordMode = ref<'empty' | 'saved' | 'manual'>('empty')
  const otherMethodsOpen = ref(false)
  const challengePending = ref<PendingValidation[]>(withChallenge ? [
    { countryCode: 86, account: '138****8000', accountType: 1, validateType: 18 as const },
    { account: 'op***@example.com', accountType: 2, validateType: 23 as const },
  ] : [])
  const selectedChallengeType = ref<number | null>(withChallenge ? 18 : null)
  const toggleOtherMethods = () => {
    otherMethodsOpen.value = !otherMethodsOpen.value
  }
  const resetChallenge = vi.fn(() => {
    challengePending.value = []
    selectedChallengeType.value = null
  })
  return {
    loginMethod,
    selectedAccountUid,
    account: ref(''),
    countryCode: ref(86),
    validateValue: ref(''),
    passwordMode,
    otherMethodsOpen,
    challengePending,
    selectedChallengeType,
    selectedChallenge: computed(() =>
      challengePending.value.find((item) => item.validateType === selectedChallengeType.value),
    ),
    challengeValue: ref(''),
    challengeStep: ref(withChallenge ? 1 : 0),
    resendSeconds: ref(0),
    completedChallengeKeys: ref<string[]>([]),
    supplementedTarget: ref(''),
    passwordReuseFailed: ref(false),
    passwordReuseAttempted: ref(false),
    needsSupplementedTarget: computed(() => false),
    businessProcessing: ref([
      { businessCode: 9001, businessMsg: '设备已变更' },
    ]),
    busy: ref<string | null>(null),
    error: ref(''),
    notice: ref(''),
    isCodeMode: computed(() => loginMethod.value <= 2),
    isChallengeCode: computed(() =>
      selectedChallengeType.value === 16 || selectedChallengeType.value === 17,
    ),
    accountReady: computed(() => true),
    gt4Loading: ref(false),
    gt4Ready: ref(true),
    gt4Error: ref(''),
    sendCode: vi.fn(),
    sendChallengeCode: vi.fn(),
    submitLogin: vi.fn(),
    submitChallenge: vi.fn(),
    destroyGt4: vi.fn(),
    toggleOtherMethods,
    resetChallenge,
    resetAuthForm: vi.fn(),
    selectSavedAccount: vi.fn(),
  }
}

const emailCodePending: PendingValidation = {
  account: 'op***@example.com',
  accountType: 7,
  validateType: 16,
}

/** 走真实 useAuth，进入邮箱验证码二次验证卡片。 */
async function enterEmailCodeChallenge() {
  const backend = {
    sendSmsCode: vi.fn().mockResolvedValue(undefined),
    sendEmailCode: vi.fn().mockResolvedValue(undefined),
    issueValidationToken: vi.fn().mockResolvedValue({
      validateToken: 'issued-token',
      validateTypes: [],
    }),
    verifyValidations: vi.fn().mockResolvedValue({
      validateModelVOS: [],
      businessProcessing: [],
    }),
    listPendingValidations: vi.fn().mockResolvedValue([emailCodePending]),
    login: vi.fn().mockResolvedValue({
      status: 'challenge',
      code: 3114179,
      validateToken: 'challenge-token',
      message: '需要邮箱验证码',
      pending: [emailCodePending],
    } satisfies LoginResult),
  }
  const gt4Ready = ref(true)
  const gt4 = {
    loading: ref(false),
    ready: gt4Ready,
    error: ref(''),
    initialize: vi.fn(async () => {
      gt4Ready.value = true
      return true
    }),
    show: vi.fn((_snapshot: string, _success: (snapshot: string, fields: Gt4Fields) => void | Promise<void>) => true),
    reset: vi.fn(),
    destroy: vi.fn(() => {
      gt4Ready.value = false
    }),
  }
  let auth!: ReturnType<typeof useAuth>
  const host = mount(defineComponent({
    setup() {
      auth = useAuth(() => {}, { api: backend, gt4 })
      return () => h(LoginPanel, {
        auth,
        accounts: [{
          uid: '42',
          displayAccount: 'operator@example.com',
          loginType: 4,
          hasSavedPassword: true,
          isCurrent: false,
        }],
        selectedAccountUid: auth.selectedAccountUid.value,
      })
    },
  }))
  auth.loginMethod.value = 4
  auth.account.value = 'operator@example.com'
  auth.validateValue.value = 'plain-password'
  await auth.submitLogin()
  await flushPromises()
  const wrapper = host.findComponent(LoginPanel)
  return { auth, backend, gt4, host, wrapper }
}

describe('LoginPanel', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('源码不含隐藏的协议介绍', () => {
    expect(panelSource).not.toContain('v-if="false"')
    expect(panelSource).not.toContain('login-intro')
    expect(panelSource).not.toContain('protocol-track')
    expect(panelSource).not.toContain('countryCode=')
    expect(panelSource).not.toContain('validateToken')
    expect(panelSource).not.toContain('ValidateType')
    expect(panelSource).not.toContain('LOCAL IPC')
    expect(panelSource).not.toContain('GT4 READY')
  })

  it('默认使用邮箱密码并展示单行四种登录 tab', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)

    expect(auth.loginMethod.value).toBe(4)
    expect(wrapper.find('input[type="email"]').exists()).toBe(true)
    expect(wrapper.text()).not.toContain('其他登录方式')
    expect(wrapper.find('[data-test="toggle-other-methods"]').exists()).toBe(false)
    const tabs = wrapper.findAll('[data-test="login-method-tab"]')
    expect(tabs).toHaveLength(4)
    expect(tabs.map((tab) => tab.text())).toEqual([
      '邮箱密码',
      '邮箱验证码',
      '手机密码',
      '手机验证码',
    ])
    expect(wrapper.find('.login-method-tab.is-active').text()).toBe('邮箱密码')
    const submit = wrapper.get('.login-submit').element
    const firstTab = tabs[0]!.element
    expect(firstTab.compareDocumentPosition(submit) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBe(Node.DOCUMENT_POSITION_FOLLOWING)
  })

  it('可从手机密码 tab 切回邮箱密码', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)
    await wrapper.findAll('[data-test="login-method-tab"]')
      .find((tab) => tab.text() === '手机密码')!
      .trigger('click')
    expect(auth.loginMethod.value).toBe(3)
    expect(wrapper.find('.account-row.is-phone').exists()).toBe(true)

    await wrapper.findAll('[data-test="login-method-tab"]')
      .find((tab) => tab.text() === '邮箱密码')!
      .trigger('click')
    expect(auth.loginMethod.value).toBe(4)
    expect(wrapper.find('input[type="email"]').exists()).toBe(true)
    expect(wrapper.find('.account-row.is-phone').exists()).toBe(false)
    expect(wrapper.find('.login-method-tab.is-active').text()).toBe('邮箱密码')
  })

  it('邮箱模式账号框与密码框同宽对齐', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)
    const accountInput = wrapper.get('.account-cell input').element
    const passwordInput = wrapper.get('.secret-field input').element
    expect(accountInput.getBoundingClientRect().width).toBe(passwordInput.getBoundingClientRect().width)
  })

  it('切换 tab 时主面板高度不变', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)
    const measure = () => wrapper.get('.login-primary-panel').element.getBoundingClientRect().height

    auth.loginMethod.value = 1
    await wrapper.vm.$nextTick()
    const phoneCode = measure()

    for (const method of [4, 2, 3] as const) {
      auth.loginMethod.value = method
      await wrapper.vm.$nextTick()
      expect(measure()).toBe(phoneCode)
    }
  })

  it('验证码发送按钮嵌在输入框右侧', async () => {
    const auth = authStub()
    auth.loginMethod.value = 2
    const wrapper = mountPanel(auth)
    const field = wrapper.get('.secret-field.is-code')
    expect(field.find('input').exists()).toBe(true)
    expect(field.get('[data-test="send-code"]').text()).toContain('发送验证码')
  })

  it('密码模式不展示发送验证码按钮', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)
    expect(wrapper.find('[data-test="send-code"]').exists()).toBe(false)
    expect(wrapper.find('.field-control').exists()).toBe(false)
  })

  it('添加账号进入的登录页显示返回，普通登录页不显示', async () => {
    const auth = authStub()
    const withoutBack = mountPanel(auth)
    expect(withoutBack.find('[data-test="login-back"]').exists()).toBe(false)
    withoutBack.unmount()

    const withBack = mount(LoginPanel, {
      props: {
        auth: auth as never,
        accounts: [],
        selectedAccountUid: null,
        canReturn: true,
      },
    })
    expect(withBack.get('[data-test="login-back"]').text()).toBe('返回')
    await withBack.get('[data-test="login-back"]').trigger('click')
    expect(withBack.emitted('back')).toHaveLength(1)
    withBack.unmount()
  })

  it('主登录表单展示认证错误', async () => {
    const auth = authStub()
    auth.error.value = '登录密码不正确'
    const wrapper = mountPanel(auth)

    expect(wrapper.get('[role="alert"]').text()).toBe('登录密码不正确')
    expect(wrapper.find('.challenge-step').exists()).toBe(false)
  })

  it('仅 saved 模式展示已保存密码哨兵', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)
    expect(wrapper.find('.password-sentinel.is-visible').exists()).toBe(false)

    auth.passwordMode.value = 'saved'
    await wrapper.vm.$nextTick()
    expect(wrapper.get('.password-sentinel.is-visible').text()).toBe('已保存密码')

    auth.passwordMode.value = 'empty'
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.password-sentinel.is-visible').exists()).toBe(false)
  })

  it('二次验证隐藏协议字段并允许返回登录', async () => {
    const { auth, wrapper, host } = await enterEmailCodeChallenge()
    expect(wrapper.text()).toContain('还差一步，请确认是你本人')
    expect(wrapper.text()).not.toContain('validateToken')
    expect(wrapper.text()).not.toContain('ValidateType')
    await wrapper.get('[data-test="challenge-back"]').trigger('click')
    expect(auth.challengePending.value).toEqual([])
    expect(auth.validateToken.value).toBe('')
    host.unmount()
  })

  it('单一待验证方式直接展示，不渲染选项列表', async () => {
    const auth = authStub()
    auth.challengePending.value = [emailCodePending]
    auth.selectedChallengeType.value = 16
    auth.challengeStep.value = 1
    const wrapper = mountPanel(auth)

    expect(wrapper.text()).toContain('邮箱验证码')
    expect(wrapper.find('input[type="radio"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('改用其他验证方式')
    expect(wrapper.text()).toContain('安全验证第 1 步')
  })

  it('多种待验证方式提供可理解选项而不是协议枚举', async () => {
    const auth = authStub(true)
    const wrapper = mountPanel(auth)

    expect(wrapper.text()).toContain('交易密码')
    expect(wrapper.text()).toContain('改用其他验证方式')
    await wrapper.get('[data-test="challenge-switch"]').trigger('click')
    expect(wrapper.text()).toContain('Messenger 验证码')
    expect(wrapper.text()).not.toContain('ValidateType 18')
    expect(wrapper.text()).not.toContain('ValidateType 23')
    expect(wrapper.text()).not.toContain('9001')
    expect(wrapper.get('[data-test="challenge-value"]').attributes('type')).toBe('password')
    auth.challengeValue.value = 'trade-value'
    await wrapper.vm.$nextTick()
    await wrapper.get('[data-test="challenge-submit"]').trigger('click')
    expect(auth.submitChallenge).toHaveBeenCalledTimes(1)
  })

  it('连续挑战在同一卡片显示第 2 步', async () => {
    const auth = authStub()
    auth.challengePending.value = [{ validateType: 19, account: 'masked' }]
    auth.selectedChallengeType.value = 19
    auth.challengeStep.value = 2
    const wrapper = mountPanel(auth)

    expect(wrapper.text()).toContain('还差一步，请确认是你本人')
    expect(wrapper.text()).toContain('安全验证第 2 步')
    expect(wrapper.text()).toContain('谷歌验证码')
    expect(wrapper.find('input[type="email"]').exists()).toBe(false)
  })

  it('发送验证码后倒计时内禁止重发，卸载后不残留定时器', async () => {
    vi.useFakeTimers()
    const { auth, backend, gt4, host, wrapper } = await enterEmailCodeChallenge()
    let gt4Success: ((snapshot: string, fields: Gt4Fields) => void | Promise<void>) | undefined
    gt4.show.mockImplementation((
      _snapshot: string,
      success: (snapshot: string, fields: Gt4Fields) => void | Promise<void>,
    ) => {
      gt4Success = success
      return true
    })

    await wrapper.get('[data-test="challenge-send-code"]').trigger('click')
    await gt4Success?.('operator@example.com', gt4Fields)
    await flushPromises()

    expect(backend.sendEmailCode).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).toMatch(/60/)
    expect(wrapper.get('[data-test="challenge-send-code"]').attributes('disabled')).toBeDefined()

    await wrapper.get('[data-test="challenge-send-code"]').trigger('click')
    expect(backend.sendEmailCode).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(1000)
    expect(auth.resendSeconds.value).toBe(59)

    host.unmount()
    await vi.advanceTimersByTimeAsync(5_000)
  })

  it('提交失败时卡片不展示业务码', async () => {
    const backend = {
      sendSmsCode: vi.fn(),
      sendEmailCode: vi.fn(),
      issueValidationToken: vi.fn().mockRejectedValue({
        kind: 'business',
        code: 3110002,
        msg: '验证码错误',
        title: '认证失败',
      }),
      verifyValidations: vi.fn(),
      listPendingValidations: vi.fn(),
      login: vi.fn(),
    }
    const gt4 = {
      loading: ref(false),
      ready: ref(true),
      error: ref(''),
      initialize: vi.fn(async () => true),
      show: vi.fn(() => true),
      reset: vi.fn(),
      destroy: vi.fn(),
    }
    let auth!: ReturnType<typeof useAuth>
    const host = mount(defineComponent({
      setup() {
        auth = useAuth(() => {}, { api: backend, gt4 })
        return () => h(LoginPanel, {
          auth,
          accounts: [],
          selectedAccountUid: null,
        })
      },
    }))
    auth.account.value = 'operator@example.com'
    auth.validateValue.value = 'plain-password'
    await auth.submitLogin()
    await flushPromises()
    const wrapper = host.findComponent(LoginPanel)
    expect(wrapper.text()).toContain('验证码错误')
    expect(wrapper.text()).not.toContain('3110002')
    host.unmount()
  })

  it('主账号已完整时不展示补全输入', async () => {
    const { auth, wrapper, host } = await enterEmailCodeChallenge()
    expect(auth.needsSupplementedTarget.value).toBe(false)
    expect(wrapper.find('[data-test="challenge-supplement"]').exists()).toBe(false)
    host.unmount()
  })

  it('登录密码在已尝试复用后即使未标记失败也展示输入框', async () => {
    const auth = authStub()
    auth.challengePending.value = [{
      account: 'op***@example.com',
      accountType: 7,
      validateType: 21,
    }]
    auth.selectedChallengeType.value = 21
    auth.challengeStep.value = 1
    const wrapper = mountPanel(auth)
    expect(wrapper.find('[data-test="challenge-value"]').exists()).toBe(false)

    auth.passwordReuseAttempted.value = true
    auth.passwordReuseFailed.value = false
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-test="challenge-value"]').attributes('type')).toBe('password')
    expect(wrapper.get('[data-test="challenge-value"]').attributes('autocomplete')).toBe('current-password')
  })

  it('非登录密码验证使用 autocomplete=off', async () => {
    const auth = authStub()
    auth.challengePending.value = [{ validateType: 19, account: 'masked' }]
    auth.selectedChallengeType.value = 19
    auth.challengeStep.value = 1
    const wrapper = mountPanel(auth)
    expect(wrapper.get('[data-test="challenge-value"]').attributes('autocomplete')).toBe('off')
  })

  it('登录密码挑战在复用失败后才展示密码框', async () => {
    const auth = authStub()
    auth.challengePending.value = [{
      account: 'op***@example.com',
      accountType: 7,
      validateType: 21,
    }]
    auth.selectedChallengeType.value = 21
    auth.challengeStep.value = 1
    const wrapper = mountPanel(auth)
    expect(wrapper.find('[data-test="challenge-value"]').exists()).toBe(false)

    auth.passwordReuseFailed.value = true
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-test="challenge-value"]').attributes('type')).toBe('password')
  })

  // 仅模拟 ready、loading、error 等响应式状态，验证发送按钮可用及 IDLE 提示；不覆盖 GT4 实例销毁或重建。
  it('keeps retry available after a prior GT4 instance was destroyed', async () => {
    const auth = authStub()
    auth.loginMethod.value = 1
    auth.account.value = '13800138000'
    auth.gt4Ready.value = false
    auth.gt4Loading.value = false
    const wrapper = mountPanel(auth)

    expect(wrapper.get('[data-test="send-code"]').attributes('disabled')).toBeUndefined()
  })

  it('offers GT4 code sending for an email-code challenge', async () => {
    const auth = authStub()
    auth.challengePending.value = [{
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }]
    auth.selectedChallengeType.value = 16
    const wrapper = mountPanel(auth)

    expect(wrapper.get('[data-test="challenge-send-code"]').text()).toContain('发送')
    await wrapper.get('[data-test="challenge-send-code"]').trigger('click')
    expect(auth.sendChallengeCode).toHaveBeenCalledTimes(1)
  })

  // 生命周期契约：组件卸载时应调用 destroyGt4 清理入口。
  it('destroys the GT4 instance when the login panel unmounts', () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)

    wrapper.unmount()

    expect(auth.destroyGt4).toHaveBeenCalledTimes(1)
  })

  it('渲染 tab 滑动指示器元素', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)
    expect(wrapper.find('.login-tab-indicator').exists()).toBe(true)
  })
})
