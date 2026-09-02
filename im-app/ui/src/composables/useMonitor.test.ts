// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { GroupDto, MessageDto } from '../types/im'
import { useMonitor } from './useMonitor'

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
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
vi.mock('../services/tauri', () => ({
  api: {
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
  content_b64: btoa(id),
  send_time: sendTime,
  content_md5: id,
  stored_at: null,
})

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
    mocks.getConnectionStatus.mockResolvedValue('disconnected')
    mocks.listen.mockImplementation(
      (event: string, handler: (event: { payload: unknown }) => void) => {
        eventHandlers.set(event, handler)
        return Promise.resolve(event === 'connection_status' ? statusUnlisten : messageUnlisten)
      },
    )
  })

  it('refreshes the authoritative connection status after login', async () => {
    mocks.getConnectionStatus.mockResolvedValueOnce('connected')
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    expect(monitor.connectionStatus.value).toBe('connected')
    wrapper.unmount()
  })

  it('recovers a connecting event from the backend status snapshot', async () => {
    mocks.getConnectionStatus.mockResolvedValueOnce('connected')
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    eventHandlers.get('connection_status')?.({ payload: 'connecting' })
    await flushPromises()

    expect(monitor.connectionStatus.value).toBe('connected')
    wrapper.unmount()
  })

  it('ignores stale history and merges current history with realtime messages', async () => {
    const first = deferred<MessageDto[]>()
    const second = deferred<MessageDto[]>()
    mocks.getMessages.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()

    const staleRequest = monitor.selectGroup('7')
    const currentRequest = monitor.selectGroup('8')
    first.resolve([message('70', '7', 10)])
    await staleRequest
    expect(monitor.messages.value).toEqual([])

    eventHandlers.get('new_message')?.({ payload: message('82', '8', 20) })
    second.resolve([message('81', '8', 10)])
    await currentRequest

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['81', '82'])
    wrapper.unmount()
  })

  it('unsubscribes every Tauri event listener on component unmount', async () => {
    const { wrapper } = mountMonitor()
    await flushPromises()

    wrapper.unmount()

    expect(statusUnlisten).toHaveBeenCalledTimes(1)
    expect(messageUnlisten).toHaveBeenCalledTimes(1)
  })

  it('surfaces IPC failures and clears the loading state', async () => {
    mocks.getMessages.mockRejectedValueOnce('database unavailable')
    const { monitor, wrapper } = mountMonitor()

    await monitor.selectGroup('7')

    expect(monitor.error.value).toBe('database unavailable')
    expect(monitor.messagesLoading.value).toBe(false)
    wrapper.unmount()
  })

  it('keeps connect disabled while backend reports automatic reconnecting', async () => {
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    eventHandlers.get('connection_status')?.({ payload: 'connecting' })

    expect(monitor.connectionStatus.value).toBe('connecting')
    expect(monitor.connectDisabled.value).toBe(true)
    wrapper.unmount()
  })

  it('does not overwrite backend connecting state when connect IPC rejects', async () => {
    mocks.connectChat.mockRejectedValueOnce(new Error('initial connect failed'))
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()
    eventHandlers.get('connection_status')?.({ payload: 'connecting' })

    await monitor.connect()

    expect(monitor.connectionStatus.value).toBe('connecting')
    expect(monitor.error.value).toBe('initial connect failed')
    wrapper.unmount()
  })

  it('clears all local session state when logout IPC rejects and shows a warning', async () => {
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
