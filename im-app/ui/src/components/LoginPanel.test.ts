// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { computed, ref } from 'vue'
import { describe, expect, it, vi } from 'vitest'

import type { PendingValidation, ValidateType } from '../types/im'
import LoginPanel from './LoginPanel.vue'

function authStub(withChallenge = false) {
  const loginMethod = ref<1 | 2 | 3 | 4>(1)
  const challengePending = ref<PendingValidation[]>(withChallenge ? [
    { countryCode: 86, account: '138****8000', accountType: 1, validateType: 18 as const },
    { account: 'op***@example.com', accountType: 2, validateType: 23 as const },
  ] : [])
  const selectedChallengeType = ref<ValidateType | null>(18)
  return {
    loginMethod,
    account: ref(''),
    countryCode: ref(86),
    validateValue: ref(''),
    validateToken: ref('challenge-token'),
    secondMac: ref(''),
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
  }
}

describe('LoginPanel', () => {
  // 确认四种登录选项存在，并抽查手机验证码切换到手机密码后的字段与提示。
  it('offers four login methods without manual GT4 fields', async () => {
    const auth = authStub()
    const wrapper = mount(LoginPanel, { props: { auth: auth as never } })
    const method = wrapper.get('[data-test="login-method"]')

    expect(method.findAll('option').map((option) => option.attributes('value'))).toEqual([
      '1', '2', '3', '4',
    ])
    expect(wrapper.text()).not.toContain('lotNumber')
    expect(wrapper.get('[data-test="send-code"]').attributes('disabled')).toBeUndefined()

    auth.loginMethod.value = 3
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="send-code"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('登录密码')
    expect(wrapper.text()).toContain('客户端会按服务端规则自动加密')
  })

  // 二次挑战必须隐藏令牌明文，并按服务端类型选择密码字段和提交路径。
  it('shows server pending validation details and submits the selected value', async () => {
    const auth = authStub(true)
    const wrapper = mount(LoginPanel, { props: { auth: auth as never } })

    expect(wrapper.text()).toContain('138****8000')
    expect(wrapper.text()).toContain('ValidateType 18')
    expect(wrapper.text()).toContain('op***@example.com')
    expect(wrapper.text()).toContain('9001')
    expect(wrapper.text()).toContain('设备已变更')
    expect(wrapper.text()).not.toContain('challenge-token')
    expect(wrapper.get('[data-test="validate-token"]').attributes('type')).toBe('password')
    expect(wrapper.get('[data-test="challenge-value"]').attributes('type')).toBe('password')
    expect(wrapper.text()).toContain('交易密码')
    expect(wrapper.text()).toContain('客户端会按服务端规则自动加密')
    auth.challengeValue.value = 'trade-value'
    await wrapper.vm.$nextTick()
    await wrapper.get('[data-test="challenge-submit"]').trigger('click')

    expect(auth.submitChallenge).toHaveBeenCalledTimes(1)
  })

  // 仅模拟 ready、loading、error 等响应式状态，验证发送按钮可用及 IDLE 提示；不覆盖 GT4 实例销毁或重建。
  it('keeps retry available after a prior GT4 instance was destroyed', async () => {
    const auth = authStub()
    auth.account.value = '13800138000'
    auth.gt4Ready.value = false
    auth.gt4Loading.value = false
    const wrapper = mount(LoginPanel, { props: { auth: auth as never } })

    expect(wrapper.get('[data-test="send-code"]').attributes('disabled')).toBeUndefined()
    expect(wrapper.get('[data-test="gt4-status"]').attributes('role')).toBe('status')
    expect(wrapper.get('[data-test="gt4-status"]').text()).toContain('IDLE')
    expect(wrapper.get('[data-test="gt4-status"]').text()).not.toContain('ERROR')
  })

  it('offers GT4 code sending for an email-code challenge', async () => {
    const auth = authStub()
    auth.challengePending.value = [{
      account: 'op***@example.com',
      accountType: 7,
      validateType: 16,
    }]
    auth.selectedChallengeType.value = 16
    const wrapper = mount(LoginPanel, { props: { auth: auth as never } })

    expect(wrapper.get('[data-test="challenge-send-code"]').text()).toContain('发送邮箱验证码')
    await wrapper.get('[data-test="challenge-send-code"]').trigger('click')
    expect(auth.sendChallengeCode).toHaveBeenCalledTimes(1)
  })

  // 生命周期契约：组件卸载时应调用 destroyGt4 清理入口。
  it('destroys the GT4 instance when the login panel unmounts', () => {
    const auth = authStub()
    const wrapper = mount(LoginPanel, { props: { auth: auth as never } })

    wrapper.unmount()

    expect(auth.destroyGt4).toHaveBeenCalledTimes(1)
  })
})
