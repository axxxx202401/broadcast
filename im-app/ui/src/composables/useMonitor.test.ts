// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { GroupDto, MessageDto } from '../types/im'
import { useMonitor } from './useMonitor'

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  channelHandlers: [] as Array<(message: MessageDto) => void>,
  registerMessageChannel: vi.fn(),
  getMessages: vi.fn(),
  fetchGroups: vi.fn(),
  refreshGroups: vi.fn(),
  toggleMonitor: vi.fn(),
  connectChat: vi.fn(),
  disconnectChat: vi.fn(),
  getConnectionStatus: vi.fn(),
  logout: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }))
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ listen: mocks.listen }),
}))
vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    set onmessage(handler: (message: MessageDto) => void) {
      mocks.channelHandlers.push(handler)
    }
  },
}))
vi.mock('../services/tauri', () => ({
  api: {
    registerMessageChannel: mocks.registerMessageChannel,
    getMessages: mocks.getMessages,
    fetchGroups: mocks.fetchGroups,
    refreshGroups: mocks.refreshGroups,
    toggleMonitor: mocks.toggleMonitor,
    connectChat: mocks.connectChat,
    disconnectChat: mocks.disconnectChat,
    getConnectionStatus: mocks.getConnectionStatus,
    logout: mocks.logout,
  },
}))

const group = (id: string): GroupDto => ({
  group_id: id,
  name: `群 ${id}`,
  pic: '',
  host_id: null,
  member_count: 1,
  created_at: 0,
  monitored: 1,
  updated_at: 0,
})

const message = (id: string, groupId: string, sendTime: number): MessageDto => ({
  msg_id: id,
  group_id: groupId,
  send_uid: '9007199254740993',
  msg_type: 1,
  group_name: `群 ${groupId}`,
  content_b64: btoa(id),
  decoded_content: null,
  decode_error: null,
  send_time: sendTime,
  content_md5: id,
  stored_at: null,
})

/** 创建可控 Promise，以精确安排历史消息响应的先后顺序。 */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

function mountMonitor() {
  let monitor!: ReturnType<typeof useMonitor>
  const wrapper = mount(
    defineComponent({
      setup() {
        monitor = useMonitor()
        return () => h('div')
      },
    }),
  )
  return { monitor, wrapper }
}

describe('useMonitor', () => {
  const eventHandlers = new Map<string, (event: { payload: unknown }) => void>()
  const statusUnlisten = vi.fn()
  const messageUnlisten = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
    eventHandlers.clear()
    mocks.channelHandlers.length = 0
    mocks.registerMessageChannel.mockResolvedValue(undefined)
    mocks.getConnectionStatus.mockResolvedValue('disconnected')
    mocks.getMessages.mockResolvedValue([])
    mocks.listen.mockImplementation(
      (event: string, handler: (event: { payload: unknown }) => void) => {
        eventHandlers.set(event, handler)
        return Promise.resolve(event === 'connection_status' ? statusUnlisten : messageUnlisten)
      },
    )
  })

  it('登录后同步后端连接状态快照', async () => {
    mocks.getConnectionStatus.mockResolvedValueOnce('connected')
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    expect(monitor.connectionStatus.value).toBe('connected')
    wrapper.unmount()
  })

  it('登录后默认加载全部消息并接收任意监控群实时消息', async () => {
    mocks.getMessages.mockResolvedValueOnce([message('70', '7', 10)])
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()
    mocks.channelHandlers[0]?.(message('80', '8', 20))

    expect(mocks.getMessages).toHaveBeenCalledWith(undefined)
    expect(mocks.registerMessageChannel).toHaveBeenCalledOnce()
    expect(monitor.selectedGroup.value).toBeNull()
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['70', '80'])
    wrapper.unmount()
  })

  it('再次点击当前群组恢复全部消息', async () => {
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    await monitor.selectGroup('7')
    await monitor.selectGroup('7')

    expect(monitor.selectedGroup.value).toBeNull()
    expect(mocks.getMessages).toHaveBeenLastCalledWith(undefined)
    wrapper.unmount()
  })

  it('收到 connecting 事件后以后端快照推进状态', async () => {
    mocks.getConnectionStatus.mockResolvedValueOnce('connected')
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    eventHandlers.get('connection_status')?.({ payload: 'connecting' })
    await flushPromises()

    expect(monitor.connectionStatus.value).toBe('connected')
    wrapper.unmount()
  })

  it('TCP 连接完成后重新加载当前范围以解密早到历史消息', async () => {
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()
    mocks.getMessages.mockClear()

    eventHandlers.get('connection_status')?.({ payload: 'connected' })
    await flushPromises()

    expect(mocks.getMessages).toHaveBeenCalledWith(undefined)
    wrapper.unmount()
  })

  it('本地密钥同步完成后重新加载当前消息范围', async () => {
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()
    mocks.getMessages.mockClear()

    eventHandlers.get('message_keys_ready')?.({ payload: null })
    await flushPromises()

    expect(mocks.getMessages).toHaveBeenCalledWith(undefined)
    wrapper.unmount()
  })

  it('忽略陈旧历史响应，并合并当前历史与实时消息', async () => {
    const first = deferred<MessageDto[]>()
    const second = deferred<MessageDto[]>()
    mocks.getMessages
      .mockResolvedValueOnce([])
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()

    const staleRequest = monitor.selectGroup('7')
    const currentRequest = monitor.selectGroup('8')
    first.resolve([message('70', '7', 10)])
    await staleRequest
    expect(monitor.messages.value).toEqual([])

    mocks.channelHandlers[0]?.(message('82', '8', 20))
    second.resolve([message('81', '8', 10)])
    await currentRequest

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['81', '82'])
    wrapper.unmount()
  })

  it('组件卸载时取消全部 Tauri 事件监听', async () => {
    const { wrapper } = mountMonitor()
    await flushPromises()

    wrapper.unmount()

    expect(statusUnlisten).toHaveBeenCalledTimes(1)
    expect(messageUnlisten).toHaveBeenCalledTimes(1)
  })

  it('呈现 IPC 错误并清除消息加载状态', async () => {
    mocks.getMessages.mockRejectedValueOnce('database unavailable')
    const { monitor, wrapper } = mountMonitor()

    await monitor.selectGroup('7')

    expect(monitor.error.value).toBe('database unavailable')
    expect(monitor.messagesLoading.value).toBe(false)
    wrapper.unmount()
  })

  it('后端报告自动重连时保持连接按钮禁用', async () => {
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    eventHandlers.get('connection_status')?.({ payload: 'connecting' })

    expect(monitor.connectionStatus.value).toBe('connecting')
    expect(monitor.connectDisabled.value).toBe(true)
    wrapper.unmount()
  })

  it('连接 IPC 拒绝时不覆盖后端 connecting 状态', async () => {
    mocks.connectChat.mockRejectedValueOnce(new Error('initial connect failed'))
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()
    eventHandlers.get('connection_status')?.({ payload: 'connecting' })

    await monitor.connect()

    expect(monitor.connectionStatus.value).toBe('connecting')
    expect(monitor.error.value).toBe('initial connect failed')
    wrapper.unmount()
  })

  it('退出 IPC 拒绝时仍清空本地会话并显示警告', async () => {
    mocks.logout.mockRejectedValueOnce(new Error('disconnect timed out'))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    monitor.messages.value = [message('70', '7', 10)]
    monitor.connectionStatus.value = 'connected'

    await monitor.logout()

    expect(monitor.loggedIn.value).toBe(false)
    expect(monitor.uid.value).toBeNull()
    expect(monitor.groups.value).toEqual([])
    expect(monitor.messages.value).toEqual([])
    expect(monitor.selectedGroup.value).toBeNull()
    expect(monitor.connectionStatus.value).toBe('disconnected')
    expect(monitor.warning.value).toContain('disconnect timed out')
    expect(monitor.error.value).toBe('')
    wrapper.unmount()
  })
})
