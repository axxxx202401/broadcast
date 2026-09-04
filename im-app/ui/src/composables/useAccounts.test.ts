// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AccountSummary, RestoreSessionResult } from '../types/im'
import { useAccounts } from './useAccounts'

/** 创建可控 Promise，用于验证启动恢复与切换的代次门禁。 */
function promiseWithResolvers<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

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
  isCurrent: true,
}

/** 挂载带可控账号 IPC 的组合式函数。 */
function setupAccounts() {
  const backend = {
    restoreSession: vi.fn(),
    listAccounts: vi.fn().mockResolvedValue([]),
    switchAccount: vi.fn(),
    removeAccount: vi.fn().mockResolvedValue({ warnings: [], nextUid: null }),
    pauseSession: vi.fn().mockResolvedValue({ uid: '42' }),
  }
  let accounts!: ReturnType<typeof useAccounts>
  const wrapper = mount(defineComponent({
    setup() {
      accounts = useAccounts({ api: backend })
      return () => h('div')
    },
  }))
  return { accounts, backend, wrapper }
}

describe('useAccounts', () => {
  beforeEach(() => vi.clearAllMocks())

  it('恢复成功后进入 ready 并记录当前账号摘要', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)

    const result = await accounts.restore()

    expect(result).toMatchObject({ status: 'success', account: account42 })
    expect(accounts.phase.value).toBe('ready')
    expect(accounts.selectedAccount.value).toEqual(account42)
    expect(accounts.retryableMessage.value).toBe('')
    wrapper.unmount()
  })

  it('需要登录时进入 needsLogin 并回填账号摘要', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'needsLogin',
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
    } satisfies RestoreSessionResult)

    await accounts.restore()

    expect(accounts.phase.value).toBe('needsLogin')
    expect(accounts.selectedAccount.value).toEqual({
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 4,
      hasSavedPassword: true,
      isCurrent: false,
    })
    expect(accounts.retryableMessage.value).toBe('')
    wrapper.unmount()
  })

  it('IPC 返回异常 loginType 时回退为邮箱密码而不崩溃', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'needsLogin',
      uid: '42',
      displayAccount: 'a@example.com',
      loginType: 99,
      hasSavedPassword: true,
    } as unknown as RestoreSessionResult)

    await accounts.restore()

    expect(accounts.phase.value).toBe('needsLogin')
    expect(accounts.selectedAccount.value?.loginType).toBe(4)
    wrapper.unmount()
  })

  it('无账号时进入 needsLogin 且不选择账号', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({ status: 'noAccount' })

    await accounts.restore()

    expect(accounts.phase.value).toBe('needsLogin')
    expect(accounts.selectedAccount.value).toBeNull()
    wrapper.unmount()
  })

  it('可重试失败保持恢复态，只展示用户文案', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'retryable',
      uid: '42',
      message: '网络连接失败，请重试',
    } satisfies RestoreSessionResult)

    await accounts.restore()

    expect(accounts.phase.value).toBe('recovering')
    expect(accounts.retryableMessage.value).toBe('网络连接失败，请重试')
    expect(accounts.phase.value).not.toBe('ready')
    expect(accounts.phase.value).not.toBe('needsLogin')
    wrapper.unmount()
  })

  it('IPC 抛错时按可重试处理，不泄露内部细节', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockRejectedValueOnce({
      kind: 'other',
      message: 'http 502 from /user/detail rust panic',
    })

    await accounts.restore()

    expect(accounts.phase.value).toBe('recovering')
    expect(accounts.retryableMessage.value).toBe('网络连接失败，请重试')
    expect(accounts.retryableMessage.value).not.toContain('502')
    expect(accounts.retryableMessage.value).not.toContain('rust')
    expect(accounts.retryableMessage.value).not.toContain('/user/detail')
    wrapper.unmount()
  })

  it('退出未确认的用户文案可以原样展示', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockRejectedValueOnce({
      kind: 'other',
      message: '本次无法确认已退出，请重试',
    })

    await accounts.restore()

    expect(accounts.retryableMessage.value).toBe('本次无法确认已退出，请重试')
    wrapper.unmount()
  })

  it('忽略过期的切换结果', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    const first = promiseWithResolvers<RestoreSessionResult>()
    const second = promiseWithResolvers<RestoreSessionResult>()
    backend.switchAccount
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)

    const older = accounts.switchAccount('42')
    const newer = accounts.switchAccount('84')
    first.resolve({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    })
    await flushPromises()

    expect(accounts.selectedAccount.value?.uid).not.toBe('42')
    expect(await older).toBeNull()

    second.resolve({
      status: 'success',
      account: account84,
      groups: [],
      warnings: [],
    })
    await flushPromises()

    expect(await newer).toMatchObject({ status: 'success', account: account84 })
    expect(accounts.selectedAccount.value).toEqual(account84)
    expect(accounts.phase.value).toBe('ready')
    wrapper.unmount()
  })

  it('重试会再次调用 restoreSession', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession
      .mockResolvedValueOnce({
        status: 'retryable',
        uid: '42',
        message: '网络连接失败，请重试',
      })
      .mockResolvedValueOnce({ status: 'noAccount' })

    await accounts.restore()
    await accounts.retryRestore()

    expect(backend.restoreSession).toHaveBeenCalledTimes(2)
    expect(accounts.phase.value).toBe('needsLogin')
    wrapper.unmount()
  })

  it('切换可重试失败后重试应再次 switchAccount 而非 restoreSession', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.switchAccount
      .mockResolvedValueOnce({
        status: 'retryable',
        uid: '84',
        message: '网络连接失败，请重试',
      })
      .mockResolvedValueOnce({
        status: 'success',
        account: account84,
        groups: [],
        warnings: [],
      })

    await accounts.switchAccount('84')
    expect(accounts.phase.value).toBe('recovering')
    expect(accounts.retryableMessage.value).toBe('网络连接失败，请重试')
    expect(accounts.lastAccountOp.value).toBe('switch')

    await accounts.retryRestore()

    expect(backend.switchAccount).toHaveBeenCalledTimes(2)
    expect(backend.switchAccount).toHaveBeenNthCalledWith(2, '84')
    expect(backend.restoreSession).not.toHaveBeenCalled()
    expect(accounts.phase.value).toBe('ready')
    expect(accounts.selectedAccount.value).toEqual(account84)
    wrapper.unmount()
  })

  it('使用其他账号进入登录态并作废进行中的恢复', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    const deferred = promiseWithResolvers<RestoreSessionResult>()
    backend.restoreSession.mockReturnValueOnce(deferred.promise)

    const pending = accounts.restore()
    accounts.useOtherAccount()
    deferred.resolve({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    })
    await flushPromises()

    expect(await pending).toBeNull()
    expect(accounts.phase.value).toBe('needsLogin')
    expect(accounts.selectedAccount.value).toBeNull()
    expect(accounts.retryableMessage.value).toBe('')
    wrapper.unmount()
  })

  it('添加账号暂停会话并记住可返回 UID，不进入 recovering', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    backend.listAccounts.mockResolvedValue([account42])
    await accounts.restore()

    await accounts.beginAddAccount()

    expect(backend.pauseSession).toHaveBeenCalledTimes(1)
    expect(accounts.phase.value).toBe('needsLogin')
    expect(accounts.selectedAccount.value).toBeNull()
    expect(accounts.returnToUid.value).toBe('42')
    expect(accounts.busy.value).toBeNull()
    wrapper.unmount()
  })

  it('返回添加账号前的会话会切换回记住的 UID 并清空返回目标', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    backend.switchAccount.mockResolvedValueOnce({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    await accounts.restore()
    await accounts.beginAddAccount()

    const result = await accounts.returnFromAddAccount()

    expect(backend.switchAccount).toHaveBeenCalledWith('42')
    expect(result).toMatchObject({ status: 'success', account: account42 })
    expect(accounts.returnToUid.value).toBeNull()
    expect(accounts.phase.value).toBe('ready')
    wrapper.unmount()
  })

  it('手动登录成功后清空添加账号的返回目标', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.restoreSession.mockResolvedValueOnce({
      status: 'success',
      account: account42,
      groups: [],
      warnings: [],
    } satisfies RestoreSessionResult)
    await accounts.restore()
    await accounts.beginAddAccount()

    accounts.applyManualLogin(account84)

    expect(accounts.returnToUid.value).toBeNull()
    wrapper.unmount()
  })

  it('移除账号只发送 uid 并刷新摘要列表', async () => {
    const { accounts, backend, wrapper } = setupAccounts()
    backend.listAccounts.mockResolvedValueOnce([])
    backend.removeAccount.mockResolvedValueOnce({
      warnings: ['本次无法完全清除登录信息'],
      nextUid: null,
    })

    await accounts.removeAccount('42')

    expect(backend.removeAccount).toHaveBeenCalledWith('42')
    expect(backend.listAccounts).toHaveBeenCalled()
    const payload = JSON.stringify(backend.removeAccount.mock.calls[0])
    expect(payload).not.toMatch(/password/i)
    expect(payload).not.toMatch(/token/i)
    wrapper.unmount()
  })
})
