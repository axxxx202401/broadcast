// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AccountSummary, RestoreSessionResult } from './types/im'

let onLoginCb: ((payload: { account: AccountSummary; groups: unknown[]; warnings: string[] }) => void) | null = null

const mocks = vi.hoisted(() => ({
  restoreSession: vi.fn(),
  listAccounts: vi.fn(),
  switchAccount: vi.fn(),
  removeAccount: vi.fn(),
  selectSavedAccount: vi.fn(),
  resetAuthForm: vi.fn(),
  monitor: {
    loggedIn: { value: true },
    warning: { value: '' },
    error: { value: '' },
    connectionStatus: { value: 'connected' },
    uid: { value: '42' },
    pending: { value: null },
    connectDisabled: { value: false },
    filteredGroups: { value: [] },
    groups: { value: [] },
    monitoredCount: { value: 0 },
    selectedGroup: { value: null },
    search: { value: '' },
    messages: { value: [] },
    messagesLoading: { value: false },
    hasOlder: { value: true },
    loadingOlder: { value: false },
    olderRequestToken: { value: 7 },
    disconnect: vi.fn(),
    connect: vi.fn(),
    logout: vi.fn(),
    selectGroup: vi.fn(),
    showAllMessages: vi.fn(),
    toggleGroup: vi.fn(),
    refreshGroups: vi.fn(),
    fetchGroups: vi.fn(),
    acceptLogin: vi.fn(),
    loadOlderMessages: vi.fn(),
    handleOlderSettled: vi.fn(),
  },
}))

vi.mock('./services/tauri', () => ({
  api: {
    restoreSession: mocks.restoreSession,
    listAccounts: mocks.listAccounts,
    switchAccount: mocks.switchAccount,
    removeAccount: mocks.removeAccount,
  },
}))
vi.mock('./composables/useMonitor', () => ({
  useMonitor: () => mocks.monitor,
}))
vi.mock('./composables/useAuth', () => ({
  useAuth: (cb: unknown) => {
    onLoginCb = cb as any
    return {
      selectSavedAccount: mocks.selectSavedAccount,
      resetAuthForm: mocks.resetAuthForm,
      destroyGt4: vi.fn(),
    }
  },
}))

import { api } from './services/tauri'
import App from './App.vue'
import LoginPanel from './components/LoginPanel.vue'

/** 创建可控 Promise，用于断言恢复完成前不会闪现登录页。 */
function promiseWithResolvers<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

const restoredAccount: AccountSummary = {
  uid: '42',
  displayAccount: 'a@example.com',
  loginType: 4,
  hasSavedPassword: true,
  isCurrent: true,
}

/** 恢复测试使用 LoginPanel 桩，避免真实面板读取不完整的 useAuth mock。 */
function mountApp() {
  return mount(App, {
    global: {
      stubs: {
        LoginPanel: {
          name: 'LoginPanel',
          template: '<div class="login-panel-stub" />',
        },
      },
    },
  })
}

describe('App 启动恢复', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    onLoginCb = null
    mocks.monitor.loggedIn.value = true
    mocks.monitor.warning.value = ''
    mocks.restoreSession.mockResolvedValue({
      status: 'success',
      account: restoredAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    mocks.listAccounts.mockResolvedValue([])
  })

  it('恢复完成前只显示启动状态', async () => {
    const deferred = promiseWithResolvers<RestoreSessionResult>()
    vi.mocked(api.restoreSession).mockReturnValueOnce(deferred.promise)
    const wrapper = mountApp()
    expect(wrapper.text()).toContain('正在恢复上次登录')
    expect(wrapper.findComponent(LoginPanel).exists()).toBe(false)
    deferred.resolve({ status: 'noAccount' })
    await flushPromises()
    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    wrapper.unmount()
  })

  it('恢复成功后接受登录并隐藏登录页', async () => {
    const groups = [{
      group_id: '7',
      name: '群 7',
      pic: '',
      host_id: null,
      member_count: 1,
      created_at: 0,
      monitored: 1,
      updated_at: 0,
    }]
    vi.mocked(api.restoreSession).mockResolvedValueOnce({
      status: 'success',
      account: restoredAccount,
      groups,
      warnings: [],
    })
    const wrapper = mountApp()
    await flushPromises()

    expect(wrapper.findComponent(LoginPanel).exists()).toBe(false)
    expect(wrapper.find('.operations-shell').exists()).toBe(true)
    expect(mocks.monitor.acceptLogin).toHaveBeenCalledWith(groups, '42')
    wrapper.unmount()
  })

  it('恢复 success 会清空 monitor.warning（即使 warnings 为空）', async () => {
    mocks.monitor.warning.value = '旧警告'
    vi.mocked(api.restoreSession).mockResolvedValueOnce({
      status: 'success',
      account: restoredAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)

    const wrapper = mountApp()
    await flushPromises()

    expect(mocks.monitor.warning.value).toBe('')
    wrapper.unmount()
  })

  it('onLogin success 会清空 monitor.warning（warnings 为空）', async () => {
    const wrapper = mountApp()
    await flushPromises()

    mocks.monitor.warning.value = '旧警告'
    onLoginCb?.({
      account: restoredAccount,
      groups: [],
      warnings: [],
    })
    await flushPromises()

    expect(mocks.monitor.warning.value).toBe('')
    wrapper.unmount()
  })

  it('需要登录时展示登录页并回填已保存账号', async () => {
    vi.mocked(api.restoreSession).mockResolvedValueOnce({
      status: 'needsLogin',
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
    })
    const wrapper = mountApp()
    await flushPromises()

    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(wrapper.find('.operations-shell').exists()).toBe(false)
    expect(mocks.selectSavedAccount).toHaveBeenCalledWith({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    wrapper.unmount()
  })

  it('可重试失败不闪现主界面，重试会再次恢复，其他账号进入登录页', async () => {
    vi.mocked(api.restoreSession).mockResolvedValueOnce({
      status: 'retryable',
      uid: '42',
      message: '网络连接失败，请重试',
    })
    const wrapper = mountApp()
    await flushPromises()

    expect(wrapper.findComponent(LoginPanel).exists()).toBe(false)
    expect(wrapper.find('.operations-shell').exists()).toBe(false)
    expect(wrapper.text()).toContain('正在恢复上次登录')
    expect(wrapper.text()).toContain('网络连接失败，请重试')
    expect(wrapper.text()).toContain('重试')
    expect(wrapper.text()).toContain('使用其他账号')

    vi.mocked(api.restoreSession).mockResolvedValueOnce({
      status: 'retryable',
      uid: '42',
      message: '网络连接失败，请重试',
    })
    await wrapper.get('[data-test="retry-restore"]').trigger('click')
    await flushPromises()
    expect(api.restoreSession).toHaveBeenCalledTimes(2)
    expect(wrapper.findComponent(LoginPanel).exists()).toBe(false)
    expect(wrapper.find('.operations-shell').exists()).toBe(false)

    await wrapper.get('[data-test="use-other-account"]').trigger('click')
    await flushPromises()
    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(wrapper.find('.operations-shell').exists()).toBe(false)
    wrapper.unmount()
  })
})

describe('App 消息分页接线', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.monitor.loggedIn.value = true
    mocks.restoreSession.mockResolvedValue({
      status: 'success',
      account: restoredAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    mocks.listAccounts.mockResolvedValue([])
  })

  it('把分页状态和请求代次传给消息面板并转发双向握手事件', async () => {
    const wrapper = mount(App, {
      global: {
        stubs: {
          GroupSidebar: true,
          LoginPanel: true,
          StatusBadge: true,
          MessagePanel: {
            name: 'MessagePanel',
            props: ['hasOlder', 'loadingOlder', 'olderRequestToken'],
            emits: ['load-older', 'older-settled'],
            template: `
              <div>
                <button class="load-older" @click="$emit('load-older')" />
                <button class="older-settled" @click="$emit('older-settled', olderRequestToken)" />
              </div>
            `,
          },
        },
      },
    })
    await flushPromises()

    const panel = wrapper.getComponent({ name: 'MessagePanel' })
    expect(panel.props('hasOlder')).toBe(true)
    expect(panel.props('loadingOlder')).toBe(false)
    expect(panel.props('olderRequestToken')).toBe(7)
    await wrapper.get('.load-older').trigger('click')
    expect(mocks.monitor.loadOlderMessages).toHaveBeenCalledOnce()
    await wrapper.get('.older-settled').trigger('click')
    expect(mocks.monitor.handleOlderSettled).toHaveBeenCalledWith(7)
    wrapper.unmount()
  })
})
