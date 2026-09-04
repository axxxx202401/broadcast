// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h, watch } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { GroupDto, MessageDto, MessagePage } from '../types/im'
import { useMonitor } from './useMonitor'

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  channelHandlers: [] as Array<(messages: MessageDto[]) => void>,
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
    set onmessage(handler: (messages: MessageDto[]) => void) {
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

const group = (id: string, name = `群 ${id}`, monitored = 1): GroupDto => ({
  group_id: id,
  name,
  pic: '',
  host_id: null,
  member_count: 1,
  created_at: 0,
  monitored,
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

/** 构造后端消息页，默认表示已经到达最早记录。 */
const page = (
  messages: MessageDto[],
  nextCursor: MessagePage['nextCursor'] = null,
  hasMore = nextCursor !== null,
): MessagePage => ({ messages, nextCursor, hasMore })

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

/** 注入完整群列表后返回监控状态，用于断言不受搜索影响的派生值。 */
function setupMonitor(nextGroups: GroupDto[]) {
  const { monitor } = mountMonitor()
  monitor.groups.value = nextGroups
  return monitor
}

/** 模拟 MessagePanel 在锚点恢复后回传当前历史轮次 token。 */
function settleOlder(monitor: ReturnType<typeof useMonitor>) {
  const token = monitor.olderRequestToken.value
  expect(token).not.toBeNull()
  monitor.handleOlderSettled(token!)
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
    mocks.getMessages.mockResolvedValue(page([]))
    mocks.listen.mockImplementation(
      (event: string, handler: (event: { payload: unknown }) => void) => {
        eventHandlers.set(event, handler)
        return Promise.resolve(event === 'connection_status' ? statusUnlisten : messageUnlisten)
      },
    )
    mocks.logout.mockResolvedValue({ warnings: [] })
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
    mocks.getMessages.mockResolvedValueOnce(page([message('70', '7', 10)]))
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()
    mocks.channelHandlers[0]?.([
      message('80', '8', 20),
      message('81', '7', 21),
    ])

    expect(mocks.getMessages).toHaveBeenCalledWith(undefined, undefined, 200)
    expect(mocks.registerMessageChannel).toHaveBeenCalledOnce()
    expect(monitor.selectedGroup.value).toBeNull()
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['70', '80', '81'])
    wrapper.unmount()
  })

  it('再次点击当前群组恢复全部消息', async () => {
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    await monitor.selectGroup('7')
    await monitor.selectGroup('7')

    expect(monitor.selectedGroup.value).toBeNull()
    expect(mocks.getMessages).toHaveBeenLastCalledWith(undefined, undefined, 200)
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

    expect(mocks.getMessages).toHaveBeenCalledWith(undefined, undefined, 200)
    wrapper.unmount()
  })

  it('本地密钥同步完成后重新加载当前消息范围', async () => {
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()
    mocks.getMessages.mockClear()

    eventHandlers.get('message_keys_ready')?.({ payload: null })
    await flushPromises()

    expect(mocks.getMessages).toHaveBeenCalledWith(undefined, undefined, 200)
    wrapper.unmount()
  })

  it('忽略陈旧历史响应，并合并当前历史与实时消息', async () => {
    const first = deferred<MessagePage>()
    const second = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page([]))
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()

    const staleRequest = monitor.selectGroup('7')
    const currentRequest = monitor.selectGroup('8')
    first.resolve(page([message('70', '7', 10)]))
    await staleRequest
    expect(monitor.messages.value).toEqual([])

    mocks.channelHandlers[0]?.([message('82', '8', 20)])
    second.resolve(page([message('81', '8', 10)]))
    await currentRequest

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['81', '82'])
    wrapper.unmount()
  })

  it('10000条实时Channel批次过滤去重裁剪后只提交一次消息状态', async () => {
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()
    const commits: MessageDto[][] = []
    const stop = watch(monitor.messages, (value) => commits.push(value), { flush: 'sync' })
    const sort = vi.spyOn(Array.prototype, 'sort')

    mocks.channelHandlers[0]?.(
      Array.from({ length: 10_000 }, (_, index) =>
        message(String(10_000 - index), '7', 10_000 - index),
      ),
    )

    expect(commits).toHaveLength(1)
    expect(commits[0]).toHaveLength(1000)
    expect(commits[0]?.[0]?.msg_id).toBe('9001')
    expect(commits[0]?.at(-1)?.msg_id).toBe('10000')
    expect(new Set(commits[0]?.map(({ msg_id }) => msg_id)).size).toBe(1000)
    expect(monitor.hasOlder.value).toBe(true)
    expect(monitor.nextMessageCursor.value).toEqual({ sendTime: 9001, msgId: '9001' })
    expect(sort).not.toHaveBeenCalled()
    sort.mockRestore()
    stop()

    mocks.getMessages.mockResolvedValueOnce(page(
      Array.from({ length: 200 }, (_, index) =>
        message(String(8801 + index), '7', 8801 + index),
      ),
    ))
    await monitor.loadOlderMessages()
    expect(mocks.getMessages).toHaveBeenLastCalledWith(
      undefined,
      { sendTime: 9001, msgId: '9001' },
      200,
    )
    expect(monitor.messages.value[0]?.msg_id).toBe('8801')
    expect(monitor.messages.value).toHaveLength(1000)
    settleOlder(monitor)
    wrapper.unmount()
  })

  it('跨实时批次保留首次到达顺序而不重建索引', async () => {
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()

    mocks.channelHandlers[0]?.([
      message('first', '7', 10),
      message('second', '7', 20),
    ])
    mocks.channelHandlers[0]?.([message('first', '7', 30)])
    mocks.channelHandlers[0]?.([message('first', '7', 20)])

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['first', 'second'])
    wrapper.unmount()
  })

  it('历史批次与期间到达的实时消息合并后只提交一次历史结果', async () => {
    const history = deferred<MessagePage>()
    mocks.getMessages.mockReturnValueOnce(history.promise)
    const { monitor, wrapper } = mountMonitor()
    await flushPromises()
    const commits: MessageDto[][] = []
    const stop = watch(monitor.messages, (value) => commits.push(value), { flush: 'sync' })

    const request = monitor.selectGroup('7')
    commits.length = 0
    mocks.channelHandlers[0]?.([message('72', '7', 20)])
    commits.length = 0
    history.resolve(page([message('71', '7', 10)]))
    await request

    expect(commits).toHaveLength(1)
    expect(commits[0]?.map(({ msg_id }) => msg_id)).toEqual(['71', '72'])
    stop()
    wrapper.unmount()
  })

  it('切换群组时立即清空旧范围索引且迟到历史不污染新范围', async () => {
    const first = deferred<MessagePage>()
    const second = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page([message('all', '8', 1)]))
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['all'])

    const firstRequest = monitor.selectGroup('7')
    expect(monitor.messages.value).toEqual([])
    const secondRequest = monitor.selectGroup('8')
    mocks.channelHandlers[0]?.([message('live-8', '8', 20)])
    first.resolve(page([message('history-7', '7', 10)]))
    second.resolve(page([message('history-8', '8', 10)]))
    await Promise.all([firstRequest, secondRequest])

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual([
      'history-8',
      'live-8',
    ])
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

  it('按当前游标加载更早消息并与期间实时消息去重合并', async () => {
    const older = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page(
        [message('3', '7', 30), message('2', '7', 20)],
        { sendTime: 20, msgId: '2' },
      ))
      .mockReturnValueOnce(older.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    const request = monitor.loadOlderMessages()
    const duplicate = message('2', '7', 20)
    mocks.channelHandlers[0]?.([duplicate, message('4', '7', 40)])
    older.resolve(page([message('1', '7', 10), duplicate]))
    await request
    settleOlder(monitor)

    expect(mocks.getMessages).toHaveBeenLastCalledWith(
      undefined,
      { sendTime: 20, msgId: '2' },
      200,
    )
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['1', '2', '3', '4'])
    expect(monitor.hasOlder.value).toBe(false)
    expect(monitor.loadingOlder.value).toBe(false)
    wrapper.unmount()
  })

  it('加载旧页期间缓冲实时批次且仅在当前轮握手后单次发布', async () => {
    const older = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page(
        [message('2', '7', 20), message('3', '7', 30)],
        { sendTime: 20, msgId: '2' },
      ))
      .mockReturnValueOnce(older.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()
    const commits: string[][] = []
    const stop = watch(
      monitor.messages,
      (value) => commits.push(value.map(({ msg_id }) => msg_id)),
      { flush: 'sync' },
    )

    const request = monitor.loadOlderMessages()
    mocks.channelHandlers[0]?.([message('4', '7', 40)])
    expect(commits).toEqual([])
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['2', '3'])

    older.resolve(page([message('1', '7', 10)]))
    await request

    expect(commits).toEqual([['1', '2', '3']])
    settleOlder(monitor)
    expect(commits).toEqual([['1', '2', '3'], ['1', '2', '3', '4']])
    stop()
    wrapper.unmount()
  })

  it('陈旧的历史完成握手不能发布已切换范围的实时缓冲', async () => {
    const older = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page(
        [message('7', '7', 70)],
        { sendTime: 70, msgId: '7' },
      ))
      .mockReturnValueOnce(older.promise)
      .mockResolvedValueOnce(page([message('8', '8', 80)]))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()

    const request = monitor.loadOlderMessages()
    const staleToken = monitor.olderRequestToken.value!
    mocks.channelHandlers[0]?.([message('live-7', '7', 90)])
    older.resolve(page([message('6', '7', 60)]))
    await request
    await monitor.selectGroup('8')
    monitor.handleOlderSettled(staleToken)

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['8'])
    wrapper.unmount()
  })

  it('实时缓冲最多保留1000条去重消息', async () => {
    const older = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page(
        [message('1', '7', 1)],
        { sendTime: 1, msgId: '1' },
      ))
      .mockReturnValueOnce(older.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    const request = monitor.loadOlderMessages()
    const realtime = Array.from({ length: 1200 }, (_, index) =>
      message(String(index + 100), '7', index + 100),
    )
    mocks.channelHandlers[0]?.([...realtime, ...realtime])
    older.resolve(page([]))
    await request
    settleOlder(monitor)

    expect(monitor.messages.value).toHaveLength(1000)
    expect(new Set(monitor.messages.value.map(({ msg_id }) => msg_id)).size).toBe(1000)
    expect(monitor.messages.value[0]?.msg_id).toBe('300')
    expect(monitor.messages.value.at(-1)?.msg_id).toBe('1299')
    wrapper.unmount()
  })

  it('实时裁掉旧页边界后把游标重置为当前可见最老消息以避免缺口', async () => {
    const older = deferred<MessagePage>()
    const initialCursor = { sendTime: 200, msgId: '200' }
    mocks.getMessages
      .mockResolvedValueOnce(page(
        Array.from({ length: 1000 }, (_, index) =>
          message(String(index + 200), '7', index + 200),
        ),
        initialCursor,
      ))
      .mockReturnValueOnce(older.promise)
      .mockResolvedValueOnce(page([]))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    const request = monitor.loadOlderMessages()
    mocks.channelHandlers[0]?.(
      Array.from({ length: 200 }, (_, index) =>
        message(String(index + 1200), '7', index + 1200),
      ),
    )
    older.resolve(page(
      Array.from({ length: 200 }, (_, index) => message(String(index), '7', index)),
      { sendTime: 0, msgId: '0' },
    ))
    await request
    settleOlder(monitor)
    await monitor.loadOlderMessages()

    expect(monitor.messages.value[0]?.msg_id).toBe('200')
    expect(mocks.getMessages).toHaveBeenLastCalledWith(
      undefined,
      { sendTime: 200, msgId: '200' },
      200,
    )
    wrapper.unmount()
  })

  it('非历史加载期间实时裁剪也同步可见最老游标', async () => {
    mocks.getMessages
      .mockResolvedValueOnce(page(
        Array.from({ length: 1000 }, (_, index) =>
          message(String(index), '7', index),
        ),
        { sendTime: 0, msgId: '0' },
      ))
      .mockResolvedValueOnce(page([]))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    mocks.channelHandlers[0]?.([message('1000', '7', 1000)])
    await monitor.loadOlderMessages()

    expect(mocks.getMessages).toHaveBeenLastCalledWith(
      undefined,
      { sendTime: 1, msgId: '1' },
      200,
    )
    wrapper.unmount()
  })

  it('末页满1000条收到实时消息裁剪后重新开启历史并回退游标', async () => {
    mocks.getMessages
      .mockResolvedValueOnce(page(
        Array.from({ length: 1000 }, (_, index) =>
          message(String(index), '7', index),
        ),
        null,
        false,
      ))
      .mockResolvedValueOnce(page([]))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    mocks.channelHandlers[0]?.([message('1000', '7', 1000)])

    expect(monitor.hasOlder.value).toBe(true)
    expect(monitor.messages.value[0]?.msg_id).toBe('1')
    await monitor.loadOlderMessages()
    expect(mocks.getMessages).toHaveBeenLastCalledWith(
      undefined,
      { sendTime: 1, msgId: '1' },
      200,
    )
    wrapper.unmount()
  })

  it('满1000条时加载更早页保留旧窗口并推进下一游标', async () => {
    const initialCursor = { sendTime: 200, msgId: '200' }
    const nextCursor = { sendTime: 0, msgId: '0' }
    mocks.getMessages
      .mockResolvedValueOnce(page(
        Array.from({ length: 1000 }, (_, index) =>
          message(String(index + 200), '7', index + 200),
        ),
        initialCursor,
      ))
      .mockResolvedValueOnce(page(
        Array.from({ length: 200 }, (_, index) => message(String(index), '7', index)),
        nextCursor,
      ))
      .mockResolvedValueOnce(page([]))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    await monitor.loadOlderMessages()
    settleOlder(monitor)

    expect(monitor.messages.value).toHaveLength(1000)
    expect(monitor.messages.value[0]?.msg_id).toBe('0')
    expect(monitor.messages.value.at(-1)?.msg_id).toBe('999')
    expect(new Set(monitor.messages.value.map(({ msg_id }) => msg_id)).size).toBe(1000)

    await monitor.loadOlderMessages()
    expect(mocks.getMessages).toHaveBeenLastCalledWith(undefined, nextCursor, 200)
    wrapper.unmount()
  })

  it('更早页请求防重入且到达末页后不再请求', async () => {
    const older = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page([message('2', '7', 20)], { sendTime: 20, msgId: '2' }))
      .mockReturnValueOnce(older.promise)
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    const first = monitor.loadOlderMessages()
    const duplicate = monitor.loadOlderMessages()
    expect(mocks.getMessages).toHaveBeenCalledTimes(2)
    older.resolve(page([message('1', '7', 10)]))
    await Promise.all([first, duplicate])
    settleOlder(monitor)
    await monitor.loadOlderMessages()

    expect(mocks.getMessages).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })

  it('更早页失败时保留现有消息和游标以允许重试', async () => {
    const cursor = { sendTime: 20, msgId: '2' }
    mocks.getMessages
      .mockResolvedValueOnce(page([message('2', '7', 20)], cursor))
      .mockRejectedValueOnce(new Error('older unavailable'))
      .mockResolvedValueOnce(page([message('1', '7', 10)]))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')
    await flushPromises()

    await monitor.loadOlderMessages()
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['2'])
    expect(monitor.hasOlder.value).toBe(true)
    expect(monitor.error.value).toBe('older unavailable')

    settleOlder(monitor)
    await monitor.loadOlderMessages()
    settleOlder(monitor)
    expect(mocks.getMessages).toHaveBeenLastCalledWith(undefined, cursor, 200)
    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['1', '2'])
    wrapper.unmount()
  })

  it('切群后旧范围的更早页响应不得修改消息、游标或 hasOlder', async () => {
    const staleOlder = deferred<MessagePage>()
    mocks.getMessages
      .mockResolvedValueOnce(page([message('7', '7', 70)], { sendTime: 70, msgId: '7' }))
      .mockReturnValueOnce(staleOlder.promise)
      .mockResolvedValueOnce(page(
        [message('8', '8', 80)],
        { sendTime: 80, msgId: '8' },
      ))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7'), group('8')], '42')
    await flushPromises()

    const oldRequest = monitor.loadOlderMessages()
    mocks.channelHandlers[0]?.([message('live-7', '7', 90)])
    await monitor.selectGroup('8')
    staleOlder.resolve(page([], null, false))
    await oldRequest

    expect(monitor.messages.value.map(({ msg_id }) => msg_id)).toEqual(['8'])
    expect(monitor.hasOlder.value).toBe(true)
    await monitor.loadOlderMessages()
    expect(mocks.getMessages).toHaveBeenLastCalledWith(
      '8',
      { sendTime: 80, msgId: '8' },
      200,
    )
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

  it('退出成功时展示 warnings 并清空本地会话', async () => {
    mocks.logout.mockResolvedValueOnce({ warnings: ['本次无法完全清除登录信息'] })
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')

    await monitor.logout()

    expect(monitor.loggedIn.value).toBe(false)
    expect(monitor.uid.value).toBeNull()
    expect(monitor.warning.value).toBe('本次无法完全清除登录信息')
    expect(monitor.error.value).toBe('')
    wrapper.unmount()
  })

  // 汇总必须读完整 groups，避免侧栏搜索把未匹配但仍在监控的群从标题区抹掉。
  it('监控群 ID 不受侧栏搜索影响', () => {
    const monitor = setupMonitor([
      group('101', '运维群', 1),
      group('202', '研发群', 1),
      group('303', '其他群', 0),
    ])
    monitor.search.value = '运维'
    expect(monitor.monitoredGroupIds.value).toEqual(['101', '202'])
  })

  it('退出未确认的用户文案原样展示', async () => {
    mocks.logout.mockRejectedValueOnce(new Error('本次无法确认已退出，请重试'))
    const { monitor, wrapper } = mountMonitor()
    monitor.acceptLogin([group('7')], '42')

    await monitor.logout()

    expect(monitor.loggedIn.value).toBe(false)
    expect(monitor.warning.value).toBe('本次无法确认已退出，请重试')
    wrapper.unmount()
  })
})
