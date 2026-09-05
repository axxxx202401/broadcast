// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import type { AccountSummary, GroupDto, RestoreSessionResult } from './types/im'

let onLoginCb: ((payload: { account: AccountSummary; groups: unknown[]; warnings: string[] }) => void) | null = null

const mocks = vi.hoisted(() => ({
  restoreSession: vi.fn(),
  listAccounts: vi.fn(),
    switchAccount: vi.fn(),
    pauseSession: vi.fn(),
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
    filteredGroups: { value: [] as GroupDto[] },
    groups: { value: [] as GroupDto[] },
    monitoredCount: { value: 0 },
    monitoredGroupIds: { value: [] as string[] },
    selectedGroup: { value: null },
    search: { value: '' },
    messages: { value: [] },
    messagesLoading: { value: false },
    hasOlder: { value: true },
    loadingOlder: { value: false },
    olderRequestToken: { value: 7 },
    showMatchedOnly: { value: true },
    filteredMessages: { value: [] },
    disconnect: vi.fn(),
    connect: vi.fn(),
    logout: vi.fn(),
    detachLocalSession: vi.fn(),
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
    pauseSession: mocks.pauseSession,
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
      selectedAccountUid: ref<string | null>(null),
    }
  },
}))

import { api } from './services/tauri'
import App from './App.vue'
import LoginPanel from './components/LoginPanel.vue'

/**
 * 用可控 `MediaQueryList` 替身模拟 `(max-width: 900px)`。
 * 已登录工作区测试默认宽屏，避免现有双栏断言被窄屏抽屉影响。
 */
function mockMatchMedia(matches: boolean) {
  const media = {
    matches,
    media: '(max-width: 900px)',
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }
  vi.stubGlobal('matchMedia', vi.fn(() => media))
  return media
}

/** 消息面板测量依赖 ResizeObserver；jsdom 不提供实现。 */
function stubResizeObserver() {
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  })
}

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

const drawerGroup: GroupDto = {
  group_id: '13537',
  name: '运营群',
  pic: '',
  host_id: null,
  member_count: 3,
  created_at: 0,
  monitored: 1,
  updated_at: 0,
}

beforeEach(() => {
  mockMatchMedia(false)
})

/** 恢复测试使用 LoginPanel 桩，避免真实面板读取不完整的 useAuth mock。 */
function mountApp() {
  return mount(App, {
    global: {
      stubs: {
        LoginPanel: {
          name: 'LoginPanel',
          props: ['auth', 'accounts', 'selectedAccountUid', 'canReturn'],
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
    mocks.listAccounts.mockResolvedValue([restoredAccount])
    mocks.monitor.logout.mockImplementation(async () => {
      mocks.monitor.loggedIn.value = false
    })
    mocks.monitor.detachLocalSession.mockImplementation(() => {
      mocks.monitor.loggedIn.value = false
    })
    mocks.pauseSession.mockResolvedValue({ uid: '42' })
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

describe('App 头部账号菜单', () => {
  const otherAccount: AccountSummary = {
    uid: '84',
    displayAccount: 'b@example.com',
    loginType: 4,
    hasSavedPassword: false,
    isCurrent: false,
  }

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
    mocks.listAccounts.mockResolvedValue([restoredAccount, otherAccount])
    mocks.monitor.logout.mockImplementation(async () => {
      mocks.monitor.loggedIn.value = false
    })
    mocks.monitor.detachLocalSession.mockImplementation(() => {
      mocks.monitor.loggedIn.value = false
    })
    mocks.pauseSession.mockResolvedValue({ uid: '42' })
  })

  it('顶栏展示当前邮箱且不再显示 UID /', async () => {
    const wrapper = mountApp()
    await flushPromises()

    expect(wrapper.text()).toContain('a@example.com')
    expect(wrapper.text()).not.toMatch(/UID\s*\//)
    expect(wrapper.find('[data-test="account-menu"]').exists()).toBe(true)
    wrapper.unmount()
  })

  it('退出登录回到登录页并回填当前账号', async () => {
    const wrapper = mountApp()
    await flushPromises()

    await wrapper.get('[data-test="logout"]').trigger('click')
    await flushPromises()

    expect(mocks.monitor.logout).toHaveBeenCalled()
    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(mocks.selectSavedAccount).toHaveBeenCalledWith(restoredAccount)
    wrapper.unmount()
  })

  it('添加账号进入空白邮箱密码登录且保留账号列表', async () => {
    const wrapper = mountApp()
    await flushPromises()

    await wrapper.get('[data-test="add-account"]').trigger('click')
    await flushPromises()

    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(api.pauseSession).toHaveBeenCalled()
    expect(mocks.monitor.logout).not.toHaveBeenCalled()
    expect(mocks.monitor.detachLocalSession).toHaveBeenCalled()
    expect(mocks.resetAuthForm).toHaveBeenCalledWith({ preserveSelectedAccount: false })
    expect(wrapper.findComponent(LoginPanel).props('canReturn')).toBe(true)
    expect(api.listAccounts).toHaveBeenCalled()
    expect(wrapper.findComponent(LoginPanel).props('accounts')).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ uid: '42' }),
        expect.objectContaining({ uid: '84' }),
      ]),
    )
    wrapper.unmount()
  })

  it('添加账号登录页返回时用原账号 Token 恢复', async () => {
    mocks.switchAccount.mockResolvedValueOnce({
      status: 'success',
      account: restoredAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    const wrapper = mountApp()
    await flushPromises()
    await wrapper.get('[data-test="add-account"]').trigger('click')
    await flushPromises()

    wrapper.findComponent(LoginPanel).vm.$emit('back')
    await flushPromises()

    expect(api.switchAccount).toHaveBeenCalledWith('42')
    expect(mocks.monitor.acceptLogin).toHaveBeenCalled()
    wrapper.unmount()
  })

  it('移除当前账号确认后按 nextUid 选择最近使用账号而非列表首项', async () => {
    const accountA: AccountSummary = {
      uid: '1',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: true,
    }
    const accountB: AccountSummary = {
      uid: '2',
      displayAccount: 'b@example.com',
      loginType: 4,
      hasSavedPassword: false,
      isCurrent: false,
    }
    const accountC: AccountSummary = {
      uid: '3',
      displayAccount: 'c@example.com',
      loginType: 4,
      hasSavedPassword: false,
      isCurrent: false,
    }
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    mocks.restoreSession.mockResolvedValue({
      status: 'success',
      account: accountA,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    // 刷新列表把 C 放在首位；真正最近使用应为 B（nextUid）。
    mocks.listAccounts
      .mockResolvedValueOnce([accountA, accountC, accountB])
      .mockResolvedValueOnce([accountC, accountB])
    mocks.removeAccount.mockResolvedValue({ warnings: [], nextUid: '2' })

    const wrapper = mountApp()
    await flushPromises()

    await wrapper.get('[data-test="remove-account"]').trigger('click')
    await flushPromises()

    expect(api.removeAccount).toHaveBeenCalledWith('1')
    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(mocks.selectSavedAccount).toHaveBeenCalledWith(
      expect.objectContaining({ uid: '2', displayAccount: 'b@example.com' }),
    )
    expect(mocks.selectSavedAccount).not.toHaveBeenCalledWith(
      expect.objectContaining({ uid: '3' }),
    )
    wrapper.unmount()
  })

  it('移除唯一账号后进入空白登录表单', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    mocks.removeAccount.mockResolvedValue({ warnings: [], nextUid: null })
    mocks.listAccounts
      .mockResolvedValueOnce([restoredAccount])
      .mockResolvedValueOnce([])

    const wrapper = mountApp()
    await flushPromises()

    await wrapper.get('[data-test="remove-account"]').trigger('click')
    await flushPromises()

    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(mocks.resetAuthForm).toHaveBeenCalledWith({ preserveSelectedAccount: false })
    wrapper.unmount()
  })

  it('切换可重试失败后重试再次 switchAccount，文案为正在切换账号', async () => {
    const wrapper = mountApp()
    await flushPromises()

    mocks.switchAccount.mockResolvedValueOnce({
      status: 'retryable',
      uid: '84',
      message: '网络连接失败，请重试',
    } satisfies RestoreSessionResult)

    await wrapper.get('[data-test="account-84"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('正在切换账号')
    expect(wrapper.text()).toContain('网络连接失败，请重试')
    expect(wrapper.text()).not.toContain('正在恢复上次登录')

    mocks.switchAccount.mockResolvedValueOnce({
      status: 'success',
      account: otherAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)

    await wrapper.get('[data-test="retry-restore"]').trigger('click')
    await flushPromises()

    expect(api.switchAccount).toHaveBeenCalledTimes(2)
    expect(api.switchAccount).toHaveBeenNthCalledWith(2, '84')
    expect(api.restoreSession).toHaveBeenCalledTimes(1)
    expect(wrapper.find('.operations-shell').exists()).toBe(true)
    wrapper.unmount()
  })

  it('切换成功时接受新会话，NeedsLogin 时回填登录页', async () => {
    const wrapper = mountApp()
    await flushPromises()

    mocks.switchAccount.mockResolvedValueOnce({
      status: 'success',
      account: otherAccount,
      groups: [{
        group_id: '9',
        name: '群 9',
        pic: '',
        host_id: null,
        member_count: 1,
        created_at: 0,
        monitored: 1,
        updated_at: 0,
      }],
      warnings: [],
    } satisfies RestoreSessionResult)

    await wrapper.get('[data-test="account-84"]').trigger('click')
    await flushPromises()

    expect(api.switchAccount).toHaveBeenCalledWith('84')
    expect(mocks.monitor.acceptLogin).toHaveBeenCalledWith(
      expect.arrayContaining([expect.objectContaining({ group_id: '9' })]),
      '84',
    )

    mocks.switchAccount.mockResolvedValueOnce({
      status: 'needsLogin',
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
    } satisfies RestoreSessionResult)

    await wrapper.get('[data-test="account-42"]').trigger('click')
    await flushPromises()

    expect(wrapper.findComponent(LoginPanel).exists()).toBe(true)
    expect(mocks.selectSavedAccount).toHaveBeenCalledWith({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    wrapper.unmount()
  })
})

describe('App 消息分页接线', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.monitor.loggedIn.value = true
    mocks.monitor.monitoredGroupIds.value = []
    mocks.restoreSession.mockResolvedValue({
      status: 'success',
      account: restoredAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    mocks.listAccounts.mockResolvedValue([restoredAccount])
    mocks.monitor.logout.mockImplementation(async () => {
      mocks.monitor.loggedIn.value = false
    })
    mocks.monitor.detachLocalSession.mockImplementation(() => {
      mocks.monitor.loggedIn.value = false
    })
    mocks.pauseSession.mockResolvedValue({ uid: '42' })
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

  // App 必须传 monitoredGroupIds，禁止把受搜索影响的 filteredGroups 当作汇总数据源。
  it('把完整监控群 ID 传给消息面板而不使用侧栏筛选结果', async () => {
    mocks.monitor.monitoredGroupIds.value = ['101', '202']
    mocks.monitor.filteredGroups.value = [{
      group_id: '101',
      name: '运维群',
      pic: '',
      host_id: null,
      member_count: 1,
      created_at: 0,
      monitored: 1,
      updated_at: 0,
    }]
    const wrapper = mount(App, {
      global: {
        stubs: {
          GroupSidebar: true,
          LoginPanel: true,
          StatusBadge: true,
          MessagePanel: {
            name: 'MessagePanel',
            props: ['monitoredGroupIds'],
            template: '<div />',
          },
        },
      },
    })
    await flushPromises()

    const panel = wrapper.getComponent({ name: 'MessagePanel' })
    expect(panel.props('monitoredGroupIds')).toEqual(['101', '202'])
    wrapper.unmount()
  })
})

/**
 * 已登录主界面必须挂载真实 GroupSidebar、MessagePanel、StatusBadge，
 * 才能把侧栏和消息区文案纳入断言；LoginPanel 仍用桩，避免不完整 useAuth mock。
 */
async function mountAuthenticatedApp() {
  const wrapper = mount(App, {
    global: {
      stubs: {
        LoginPanel: {
          name: 'LoginPanel',
          props: ['auth', 'accounts', 'selectedAccountUid', 'canReturn'],
          template: '<div class="login-panel-stub" />',
        },
      },
    },
  })
  await flushPromises()
  return wrapper
}

describe('App 普通用户文案', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.monitor.loggedIn.value = true
    mocks.monitor.warning.value = ''
    mocks.monitor.error.value = ''
    mocks.monitor.connectionStatus.value = 'connected'
    mocks.monitor.selectedGroup.value = null
    mocks.monitor.messages.value = []
    mocks.monitor.filteredGroups.value = []
    mocks.monitor.monitoredGroupIds.value = []
    mocks.restoreSession.mockResolvedValue({
      status: 'success',
      account: restoredAccount,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    mocks.listAccounts.mockResolvedValue([restoredAccount])
    vi.stubGlobal('ResizeObserver', class {
      observe() {}
      unobserve() {}
      disconnect() {}
    })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('主界面不显示开发术语', async () => {
    const wrapper = await mountAuthenticatedApp()
    const visible = wrapper.text()
    for (const forbidden of [
      'ALL CHANNELS',
      'ALL MONITORED CHANNELS',
      'LIVE MESSAGE STREAM',
      'CHANNEL /',
      'UID /',
      '链路在线',
      '断开链路',
      '正文和附件由 Rust 解密',
      'NO MATCH',
    ]) {
      expect(visible).not.toContain(forbidden)
    }
    wrapper.unmount()
  })
})

describe('App 窄屏群列表抽屉', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockMatchMedia(true)
    stubResizeObserver()
    mocks.monitor.loggedIn.value = true
    mocks.monitor.warning.value = ''
    mocks.monitor.error.value = ''
    mocks.monitor.connectionStatus.value = 'connected'
    mocks.monitor.selectedGroup.value = null
    mocks.monitor.messages.value = []
    mocks.monitor.filteredGroups.value = [drawerGroup]
    mocks.monitor.groups.value = [drawerGroup]
    mocks.monitor.monitoredCount.value = 1
    mocks.monitor.monitoredGroupIds.value = [drawerGroup.group_id]
    mocks.restoreSession.mockResolvedValue({
      status: 'success',
      account: restoredAccount,
      groups: [drawerGroup],
      warnings: [],
    } satisfies RestoreSessionResult)
    mocks.listAccounts.mockResolvedValue([restoredAccount])
  })

  afterEach(() => {
    mocks.monitor.filteredGroups.value = []
    mocks.monitor.groups.value = []
    mocks.monitor.monitoredCount.value = 0
    mocks.monitor.monitoredGroupIds.value = []
  })

  it('窄屏 collapse-btn 存在且默认抽屉关闭', async () => {
    const wrapper = await mountAuthenticatedApp()
    const collapseBtn = wrapper.find('.collapse-btn')
    expect(collapseBtn.exists()).toBe(true)
    expect(wrapper.find('.sidebar-mask').exists()).toBe(false)
    expect(wrapper.find('#group-sidebar-drawer').exists()).toBe(true)
    wrapper.unmount()
  })

  it('遮罩点击和 Escape 关闭抽屉', async () => {
    const wrapper = await mountAuthenticatedApp()
    const collapseBtn = wrapper.find('.collapse-btn')

    // 点击 collapse-btn 打开抽屉
    await collapseBtn.trigger('click')
    expect(wrapper.find('.sidebar-mask').exists()).toBe(true)

    // 点击遮罩关闭
    await wrapper.get('.sidebar-mask').trigger('click')
    expect(wrapper.find('.sidebar-mask').exists()).toBe(false)

    // 再次打开，用 Escape 关闭
    await collapseBtn.trigger('click')
    expect(wrapper.find('.sidebar-mask').exists()).toBe(true)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.sidebar-mask').exists()).toBe(false)
    wrapper.unmount()
  })

  it('选择群或全部群消息后关闭抽屉', async () => {
    const wrapper = await mountAuthenticatedApp()
    const collapseBtn = wrapper.find('.collapse-btn')

    // 点击 collapse-btn 打开抽屉
    await collapseBtn.trigger('click')
    expect(wrapper.find('.sidebar-mask').exists()).toBe(true)

    // 点击群选项，应选中并关闭抽屉
    await wrapper.get('.group-select').trigger('click')
    expect(mocks.monitor.selectGroup).toHaveBeenCalledWith('13537')
    expect(wrapper.find('.sidebar-mask').exists()).toBe(false)

    // 再次打开，点击全部群消息
    await collapseBtn.trigger('click')
    expect(wrapper.find('.sidebar-mask').exists()).toBe(true)
    await wrapper.get('.all-messages').trigger('click')
    expect(mocks.monitor.showAllMessages).toHaveBeenCalled()
    expect(wrapper.find('.sidebar-mask').exists()).toBe(false)
    wrapper.unmount()
  })

  it('关闭时抽屉内容不可获得焦点', async () => {
    const wrapper = await mountAuthenticatedApp()
    const drawer = wrapper.get('#group-sidebar-drawer')
    const inert = drawer.attributes('inert') !== undefined
    const hidden = drawer.attributes('aria-hidden') === 'true'
    expect(inert || hidden).toBe(true)
    if (!inert) {
      for (const el of drawer.findAll('button, input, a, [tabindex]')) {
        expect(el.attributes('tabindex')).toBe('-1')
      }
    }
    wrapper.unmount()
  })

  it('顶栏显示日夜主题切换按钮', async () => {
    const wrapper = await mountAuthenticatedApp()
    const themeBtn = wrapper.find('button[aria-label="切换日夜主题"]')
    expect(themeBtn.exists()).toBe(true)
    expect(themeBtn.find('span').text()).toContain('☾')
    wrapper.unmount()
  })
})
