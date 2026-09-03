// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { computed, ref } from 'vue'
import { describe, expect, it, vi } from 'vitest'

import type { PendingValidation } from '../types/im'
import LoginPanel from './LoginPanel.vue'
import panelSource from './LoginPanel.vue?raw'

/** 挂载登录面板并注入最小认证桩。 */
function mountPanel(auth: ReturnType<typeof authStub>) {
  return mount(LoginPanel, {
    props: {
      auth: auth as never,
      accounts: [],
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
  const selectedChallengeType = ref<number | null>(18)
  const toggleOtherMethods = () => {
    otherMethodsOpen.value = !otherMethodsOpen.value
  }
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
  }
}

describe('LoginPanel', () => {
  it('源码不含隐藏的协议介绍', () => {
    expect(panelSource).not.toContain('v-if="false"')
    expect(panelSource).not.toContain('login-intro')
    expect(panelSource).not.toContain('protocol-track')
    expect(panelSource).not.toContain('countryCode=')
    expect(panelSource).not.toContain('validateToken')
  })

  it('默认使用邮箱密码并折叠其他方式', async () => {
    const auth = authStub()
    const wrapper = mountPanel(auth)

    expect(auth.loginMethod.value).toBe(4)
    expect(wrapper.find('input[type="email"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('其他登录方式')
    expect(wrapper.text()).not.toContain('手机号验证码')
    const submit = wrapper.get('.login-submit').element
    const other = wrapper.get('[data-test="toggle-other-methods"]').element
    expect(submit.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBe(Node.DOCUMENT_POSITION_FOLLOWING)
  })

  it('主登录表单展示认证错误', async () => {
    const auth = authStub()
    auth.error.value = '登录密码不正确'
    const wrapper = mountPanel(auth)

    expect(wrapper.get('[role="alert"]').text()).toBe('登录密码不正确')
    expect(wrapper.find('.challenge-step').exists()).toBe(false)
  })

  it('shows server pending validation details and submits the selected value', async () => {
    const auth = authStub(true)
    const wrapper = mountPanel(auth)

    expect(wrapper.text()).toContain('138****8000')
    expect(wrapper.text()).toContain('op***@example.com')
    expect(wrapper.text()).toContain('交易密码')
    expect(wrapper.text()).toContain('设备已变更')
    expect(wrapper.get('[data-test="challenge-value"]').attributes('type')).toBe('password')
    auth.challengeValue.value = 'trade-value'
    await wrapper.vm.$nextTick()
    await wrapper.get('[data-test="challenge-submit"]').trigger('click')

    expect(auth.submitChallenge).toHaveBeenCalledTimes(1)
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

    expect(wrapper.get('[data-test="challenge-send-code"]').text()).toContain('发送邮箱验证码')
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
})
