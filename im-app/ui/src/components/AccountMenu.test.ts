// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AccountSummary } from '../types/im'
import AccountMenu from './AccountMenu.vue'

const account42: AccountSummary = {
  uid: '42',
  displayAccount: 'a@example.com',
  loginType: 4,
  hasSavedPassword: true,
  isCurrent: true,
}

const account84: AccountSummary = {
  uid: '84',
  displayAccount: 'b@example.com',
  loginType: 4,
  hasSavedPassword: false,
  isCurrent: false,
}

describe('AccountMenu', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('显示当前账号并阻止重复切换', async () => {
    const switchAccount = vi.fn(() => new Promise(() => {}))
    const wrapper = mount(AccountMenu, {
      props: { current: account42, accounts: [account42, account84], switching: false, switchAccount },
    })
    expect(wrapper.text()).toContain('a@example.com')
    await wrapper.get('[data-test="account-84"]').trigger('click')
    expect(switchAccount).toHaveBeenCalledWith('84')
    await wrapper.setProps({ switching: true })
    expect(wrapper.get('[data-test="account-84"]').attributes('disabled')).toBeDefined()
  })

  it('切换中显示提示并禁用全部账号动作', async () => {
    const switchAccount = vi.fn()
    const wrapper = mount(AccountMenu, {
      props: {
        current: account42,
        accounts: [account42, account84],
        switching: true,
        switchAccount,
      },
    })

    expect(wrapper.text()).toContain('正在切换账号')
    expect(wrapper.get('[data-test="account-84"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-test="add-account"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-test="logout"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-test="remove-account"]').attributes('disabled')).toBeDefined()
  })

  it('用户 ID 仅作为次要信息展示', () => {
    const wrapper = mount(AccountMenu, {
      props: {
        current: account42,
        accounts: [account42],
        switching: false,
        switchAccount: vi.fn(),
      },
    })

    expect(wrapper.text()).toContain('用户 ID')
    expect(wrapper.text()).toContain('42')
    expect(wrapper.text()).not.toMatch(/UID\s*\/\s*42/)
  })

  it('退出登录发出 logout 事件', async () => {
    const wrapper = mount(AccountMenu, {
      props: {
        current: account42,
        accounts: [account42],
        switching: false,
        switchAccount: vi.fn(),
      },
    })

    await wrapper.get('[data-test="logout"]').trigger('click')
    expect(wrapper.emitted('logout')).toHaveLength(1)
  })

  it('添加账号发出 addAccount 事件', async () => {
    const wrapper = mount(AccountMenu, {
      props: {
        current: account42,
        accounts: [account42, account84],
        switching: false,
        switchAccount: vi.fn(),
      },
    })

    await wrapper.get('[data-test="add-account"]').trigger('click')
    expect(wrapper.emitted('addAccount')).toHaveLength(1)
  })

  it('移除此账号需确认后才发出 removeAccount', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValueOnce(false)
    const wrapper = mount(AccountMenu, {
      props: {
        current: account42,
        accounts: [account42, account84],
        switching: false,
        switchAccount: vi.fn(),
      },
    })

    await wrapper.get('[data-test="remove-account"]').trigger('click')
    expect(confirm).toHaveBeenCalled()
    expect(wrapper.emitted('removeAccount')).toBeUndefined()

    confirm.mockReturnValueOnce(true)
    await wrapper.get('[data-test="remove-account"]').trigger('click')
    expect(wrapper.emitted('removeAccount')).toEqual([['42']])
  })
})
