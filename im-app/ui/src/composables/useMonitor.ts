import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Channel } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { api } from '../services/tauri'
import type { ConnectionStatus, GroupDto, MessageCursor, MessageDto } from '../types/im'
import {
  isCurrentMessageRequest,
  MessageIndex,
  type MessageMergeResult,
  type MessageTrimStrategy,
} from '../utils/message'
import { errorMessage, normalizeConnectionStatus } from '../utils/protocol'

/**
 * 管理监控页会话、群组、消息历史和聊天连接状态。
 *
 * `pending` 只阻止本组合式函数经 `run` 发起的动作重叠，不构成后端事务或全局并发保证；
   * 消息历史另用请求编号防止陈旧响应覆盖当前范围。连接状态以事件和后端快照共同推进；
 * 部分异步失败路径不具备与成功路径相同的会话门禁，详见对应流程注释。
 *
 * @returns 页面状态、筛选、不受搜索影响的监控群 ID 与计数派生值，以及登录接收、群组、连接和退出操作。
 */
export function useMonitor() {
  // 会话、群组与消息均为页面本地状态；已取得执行权的退出流程会在 finally 中清空。
  const loggedIn = ref(false)
  const uid = ref<string | null>(null)
  const groups = ref<GroupDto[]>([])
  const selectedGroupId = ref<string | null>(null)
  const messages = ref<MessageDto[]>([])
  // 索引跨历史与实时批次持久存在；仅在查询范围切换或会话清理时整体重置。
  const messageIndex = new MessageIndex()
  // 旧页请求期间实时消息暂存于独立的有界去重索引，避免提前改变虚拟列表首尾。
  const bufferedRealtimeIndex = new MessageIndex()
  const messagesLoading = ref(false)
  const loadingOlder = ref(false)
  const hasOlder = ref(false)
  const nextMessageCursor = ref<MessageCursor | null>(null)
  const olderRequestToken = ref<number | null>(null)
  const search = ref('')
  const connectionStatus = ref<ConnectionStatus>('disconnected')
  const pending = ref<string | null>(null)
  const error = ref('')
  const warning = ref('')
  const unlisteners: UnlistenFn[] = []
  let messageRequestId = 0
  let connectionStatusVersion = 0
  let connectionStatusTimer: ReturnType<typeof setTimeout> | null = null
  let messageChannel: Channel<MessageDto[]> | null = null
  let bufferedRealtimeRequestId: number | null = null
  let olderRequestSequence = 0
  let activeOlderRequest: {
    token: number
    requestId: number
    groupId: string | null
  } | null = null

  /**
   * 原位合并索引后只复制并发布一次 Vue 状态。
   * 索引负责 O(1) 有序尾部追加或 O(log n) 乱序定位，响应式数组不参与中间步骤。
   */
  function mergeAndPublishMessages(
    incoming: MessageDto[],
    trimStrategy: MessageTrimStrategy = 'keep-latest',
  ): MessageMergeResult {
    const result = messageIndex.mergeWithResult(incoming, trimStrategy)
    if (!result.changed) return result
    messages.value = messageIndex.snapshot()
    return result
  }

  /** 丢弃旧页请求期间积累的实时消息及其范围代次。 */
  function clearBufferedRealtimeMessages() {
    bufferedRealtimeIndex.clear()
    bufferedRealtimeRequestId = null
  }

  /** 比较消息与游标是否表示同一个复合排序键。 */
  function matchesCursor(message: MessageDto | undefined, cursor: MessageCursor): boolean {
    return message?.msg_id === cursor.msgId && message.send_time === cursor.sendTime
  }

  /**
   * 合并一个实际实时批次，并在 keep-latest 实际裁剪时重新开放历史分页。
   *
   * 裁剪数量由 MessageIndex 基于批内唯一 ID 的实际删除返回，因此空窗口一次接收 10k
   * 也能识别前 9000 条已离开视图；未裁剪时保留后端给出的 `hasOlder=false`。
   */
  function mergeRealtimeAndPublish(incoming: MessageDto[]) {
    const result = mergeAndPublishMessages(incoming, 'keep-latest')
    if (!result.changed) return
    const oldestAfter = messages.value[0]
    if (result.trimmed > 0 && oldestAfter) {
      hasOlder.value = true
      nextMessageCursor.value = {
        sendTime: oldestAfter.send_time,
        msgId: oldestAfter.msg_id,
      }
      return
    }

    const cursor = nextMessageCursor.value
    if (!hasOlder.value || !cursor) return
    if (matchesCursor(messageIndex.get(cursor.msgId), cursor)) return
    if (oldestAfter) {
      nextMessageCursor.value = {
        sendTime: oldestAfter.send_time,
        msgId: oldestAfter.msg_id,
      }
    }
  }

  /** 在范围门禁仍有效时，以一次 Vue 赋值发布当前请求缓冲的实时消息。 */
  function publishBufferedRealtimeMessages(requestId: number, groupId: string | null) {
    if (
      bufferedRealtimeRequestId !== requestId
      || !isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)
    ) {
      clearBufferedRealtimeMessages()
      return
    }
    const buffered = bufferedRealtimeIndex.snapshot()
    clearBufferedRealtimeMessages()
    mergeRealtimeAndPublish(buffered)
  }

  /** 清空当前查询范围的索引，并以一次赋值发布空视图。 */
  function clearMessages() {
    messageIndex.clear()
    messages.value = []
  }

  /** 清除当前范围的分页边界；切群、重载和退出均使旧游标立即失效。 */
  function resetMessagePagination() {
    clearBufferedRealtimeMessages()
    activeOlderRequest = null
    olderRequestToken.value = null
    nextMessageCursor.value = null
    hasOlder.value = false
    loadingOlder.value = false
  }

  /** 当前选中的完整群组；未选择或群组已移除时为 null。 */
  const selectedGroup = computed(
    () => groups.value.find((group) => group.group_id === selectedGroupId.value) ?? null,
  )
  const filteredGroups = computed(() => {
    const query = search.value.trim().toLocaleLowerCase()
    if (!query) return groups.value
    return groups.value.filter(
      (group) =>
        group.name.toLocaleLowerCase().includes(query) || String(group.group_id).includes(query),
    )
  })
  /**
   * 正在监控的群 ID，直接来自完整 `groups`，不受侧栏 `search` 筛选影响。
   * `monitored !== 0` 视为监控中；顺序与群列表一致。
   */
  const monitoredGroupIds = computed(() =>
    groups.value
      .filter(({ monitored }) => monitored !== 0)
      .map(({ group_id }) => group_id),
  )
  const monitoredCount = computed(() => groups.value.filter((group) => group.monitored !== 0).length)
  const connectDisabled = computed(
    () => pending.value !== null || connectionStatus.value === 'connecting',
  )

  /** 运行一个互斥页面动作并统一维护 pending/error/warning。 */
  async function run(key: string, operation: () => Promise<void>) {
    if (pending.value) return
    pending.value = key
    error.value = ''
    warning.value = ''
    try {
      await operation()
    } catch (reason) {
      error.value = errorMessage(reason)
    } finally {
      pending.value = null
    }
  }

  /**
   * 从后端同步连接快照。
   * connecting 状态下每 500ms 继续轮询。成功分支以 mounted、uid 和 version 拒绝
   * 不再匹配当前会话的结果；Promise 拒绝分支没有同样门禁，logout 或卸载后仍可能写 warning。
   */
  function syncConnectionStatus(expectedUid: string | null = uid.value) {
    const statusVersion = connectionStatusVersion
    void api.getConnectionStatus().then((status) => {
      if (
        !mounted ||
        uid.value !== expectedUid ||
        connectionStatusVersion !== statusVersion
      ) return
      const normalized = normalizeConnectionStatus(status)
      if (connectionStatus.value === 'connecting' && normalized === 'disconnected') {
        connectionStatusTimer = setTimeout(
          () => syncConnectionStatus(expectedUid),
          500,
        )
        return
      }
      const previousStatus = connectionStatus.value
      connectionStatus.value = normalized
      if (normalized === 'connected' && previousStatus !== 'connected' && loggedIn.value) {
        void loadMessages(selectedGroupId.value)
      }
      if (normalized === 'connecting') {
        connectionStatusTimer = setTimeout(
          () => syncConnectionStatus(expectedUid),
          500,
        )
      }
    }).catch((reason) => {
      warning.value = `连接状态同步失败：${errorMessage(reason)}`
    })
  }

  /** 接受登录结果，切换本地会话并递增请求号，使登录前的消息历史响应不再满足写入条件。 */
  function acceptLogin(nextGroups: GroupDto[], nextUid: string) {
    messageRequestId += 1
    groups.value = nextGroups
    uid.value = nextUid
    loggedIn.value = true
    selectedGroupId.value = null
    clearMessages()
    messagesLoading.value = false
    syncConnectionStatus(nextUid)
    void loadMessages(null)
  }

  /** 从后端读取群组列表。 */
  const fetchGroups = () =>
    run('fetch', async () => {
      groups.value = await api.fetchGroups()
    })

  /** 请求后端刷新并返回群组列表。 */
  const refreshGroups = () =>
    run('refresh', async () => {
      groups.value = await api.refreshGroups()
    })

  /** 加载全部或单群历史；messageRequestId 防止旧范围响应覆盖新选择。 */
  async function loadMessages(groupId: string | null) {
    const requestId = ++messageRequestId
    selectedGroupId.value = groupId
    clearMessages()
    resetMessagePagination()
    messagesLoading.value = true
    error.value = ''
    try {
      const history = await api.getMessages(groupId ?? undefined, undefined, 200)
      if (isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
        // 历史返回期间可能已收到实时消息，合并而非覆盖可保留两条来源。
        mergeAndPublishMessages(history.messages)
        nextMessageCursor.value = history.nextCursor
        hasOlder.value = history.hasMore
      }
    } catch (reason) {
      if (isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
        error.value = errorMessage(reason)
      }
    } finally {
      if (isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
        messagesLoading.value = false
      }
    }
  }

  /**
   * 在当前范围内读取更早一页。
   *
   * `loadingOlder` 防止同一游标并发重放；请求捕获范围代次和群组，切群、重载或退出后，
   * 迟到的成功、失败和 finally 均不得修改新范围的消息、游标及加载状态。失败保留现有
   * 索引与游标，允许用户再次触发。
   */
  async function loadOlderMessages() {
    const cursor = nextMessageCursor.value
    if (activeOlderRequest || loadingOlder.value || !hasOlder.value || !cursor) return

    const requestId = messageRequestId
    const groupId = selectedGroupId.value
    const token = ++olderRequestSequence
    activeOlderRequest = { token, requestId, groupId }
    olderRequestToken.value = token
    clearBufferedRealtimeMessages()
    bufferedRealtimeRequestId = requestId
    loadingOlder.value = true
    error.value = ''
    try {
      const history = await api.getMessages(groupId ?? undefined, cursor, 200)
      if (!isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
        return
      }
      // 历史页可能与实时批次重叠；向上翻页超限时裁掉尾部新消息，保留可继续浏览的旧窗口。
      mergeAndPublishMessages(history.messages, 'keep-earliest')
      nextMessageCursor.value = history.nextCursor
      hasOlder.value = history.hasMore
    } catch (reason) {
      if (isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
        // 失败不改变原消息和游标；实时缓冲等待面板完成无新增握手后再发布。
        error.value = errorMessage(reason)
      }
    } finally {
      if (
        activeOlderRequest?.token === token
        && isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)
      ) {
        loadingOlder.value = false
      }
    }
  }

  /**
   * 接受 MessagePanel 完成锚点恢复后的显式握手。
   *
   * token、请求代次和群组必须全部匹配当前活动轮次；陈旧面板事件只被忽略。通过门禁后
   * 才以一次响应式赋值发布实时缓冲，并释放下一轮历史请求。
   */
  function handleOlderSettled(token: number) {
    const active = activeOlderRequest
    if (
      !active
      || active.token !== token
      || olderRequestToken.value !== token
      || !isCurrentMessageRequest(
        active.requestId,
        messageRequestId,
        active.groupId,
        selectedGroupId.value,
      )
    ) return

    publishBufferedRealtimeMessages(active.requestId, active.groupId)
    activeOlderRequest = null
    olderRequestToken.value = null
  }

  /** 选择单群；再次点击当前群组时取消筛选并恢复全部消息。 */
  async function selectGroup(groupId: string) {
    await loadMessages(selectedGroupId.value === groupId ? null : groupId)
  }

  /** 显式恢复全部受监控群组消息。 */
  async function showAllMessages() {
    await loadMessages(null)
  }

  /** 切换指定群组的监控状态，并在远程成功后更新本地列表。 */
  const toggleGroup = (group: GroupDto) =>
    run(`toggle-${group.group_id}`, async () => {
      const monitored = group.monitored === 0
      await api.toggleMonitor(group.group_id, monitored)
      groups.value = groups.value.map((item) =>
        item.group_id === group.group_id ? { ...item, monitored: monitored ? 1 : 0 } : item,
      )
    })

  /** 请求建立聊天连接；最终连接状态仍由事件或状态同步确认。 */
  const connect = () =>
    run('connect', async () => {
      await api.connectChat()
    })

  /** 请求断开聊天连接，并在远程成功后标记本地状态为 disconnected。 */
  const disconnect = () =>
    run('disconnect', async () => {
      await api.disconnectChat()
      connectionStatus.value = 'disconnected'
    })

  /**
   * 只清理前端会话视图，不请求退出、不删除 Token。
   * 供添加账号进入登录页时与后端 `pause_session` 配合使用。
   */
  function detachLocalSession() {
    connectionStatusVersion += 1
    loggedIn.value = false
    uid.value = null
    groups.value = []
    clearMessages()
    resetMessagePagination()
    selectedGroupId.value = null
    messageRequestId += 1
    messagesLoading.value = false
    connectionStatus.value = 'disconnected'
  }

  /**
   * 尝试取得 `run` 的 pending 执行权后请求远程退出。
   * 若已有动作占用 pending，本次调用直接返回，不请求后端也不清理本地状态；若已开始执行，
   * 即使远程 logout 失败也会在 finally 清理本地会话。成功路径展示后端 warnings；
   * 退出未确认的用户文案原样展示，其余拒绝值仍以 warning 披露链路状态不确定。
   */
  const logout = () =>
    run('logout', async () => {
      try {
        const result = await api.logout()
        if (result.warnings.length) {
          warning.value = result.warnings.join('\n')
        }
      } catch (reason) {
        const text = errorMessage(reason)
        warning.value = text === '本次无法确认已退出，请重试'
          ? text
          : `已退出，但断开聊天链路时出现问题：${text}`
      } finally {
        detachLocalSession()
      }
    })

  let mounted = true

  onMounted(() => {
    messageChannel = new Channel<MessageDto[]>()
    messageChannel.onmessage = (batch) => {
      // 后端保持批内协议顺序；单群视图只筛选当前群，再一次性合并，避免逐条触发响应式更新。
      const visible = selectedGroupId.value === null
        ? batch
        : batch.filter((message) => message.group_id === selectedGroupId.value)
      if (visible.length === 0) return
      if (activeOlderRequest) {
        // 缓冲索引自身限制为 MAX_MESSAGES，并按 ID 去重；请求结束前不触碰可见索引。
        if (bufferedRealtimeRequestId !== messageRequestId) {
          clearBufferedRealtimeMessages()
          bufferedRealtimeRequestId = messageRequestId
        }
        bufferedRealtimeIndex.merge(visible, 'keep-latest')
        return
      }
      mergeRealtimeAndPublish(visible)
    }
    void api.registerMessageChannel(messageChannel).catch((reason) => {
      error.value = `实时消息通道注册失败：${errorMessage(reason)}`
    })
    /*
     * connection_status 和 message_keys_ready 保留低频全局事件；高频实时消息使用上方
     * Channel，在全部模式接收所有群，在单群模式过滤。allSettled 会等两个
     * listen 注册都 settle 后才进入 then 并保存成功项的 unlisten；若一个成功而另一个长期
     * pending，卸载时可能尚未取得成功项的 unlisten，存在延迟释放或订阅泄漏窗口。
     */
    void Promise.allSettled([
      listen<string>('connection_status', ({ payload }) => {
        connectionStatusVersion += 1
        const status = normalizeConnectionStatus(payload)
        const previousStatus = connectionStatus.value
        connectionStatus.value = status
        if (status === 'connected' && previousStatus !== 'connected' && loggedIn.value) {
          void loadMessages(selectedGroupId.value)
        }
        if (status === 'connecting') syncConnectionStatus()
      }),
      listen('message_keys_ready', () => {
        if (loggedIn.value) void loadMessages(selectedGroupId.value)
      }),
    ]).then((results) => {
      // 两项均 settle 后若组件已经卸载，此处才调用成功注册项返回的 unlisten。
      for (const result of results) {
        if (result.status === 'rejected') {
          error.value = `事件监听失败：${errorMessage(result.reason)}`
        } else if (mounted) {
          unlisteners.push(result.value)
        } else {
          result.value()
        }
      }
    })
  })

  onBeforeUnmount(() => {
    // 停止状态轮询，并释放此时已保存到 unlisteners 的监听；尚卡在 allSettled 中的项不在其中。
    mounted = false
    activeOlderRequest = null
    olderRequestToken.value = null
    clearBufferedRealtimeMessages()
    messageChannel = null
    if (connectionStatusTimer) clearTimeout(connectionStatusTimer)
    for (const unlisten of unlisteners) unlisten()
  })

  return {
    /** 当前是否持有本地登录会话。 */
    loggedIn,
    uid,
    /** 当前群组列表及其筛选结果。 */
    groups,
    filteredGroups,
    /** 正在监控的群 ID；不受侧栏搜索影响。 */
    monitoredGroupIds,
    selectedGroup,
    /** 当前群组的历史与实时消息合并结果。 */
    messages,
    messagesLoading,
    loadingOlder,
    hasOlder,
    nextMessageCursor,
    olderRequestToken,
    search,
    /** 事件与轮询共同维护的聊天连接状态。 */
    connectionStatus,
    monitoredCount,
    pending,
    error,
    warning,
    connectDisabled,
    acceptLogin,
    fetchGroups,
    refreshGroups,
    selectGroup,
    showAllMessages,
    loadOlderMessages,
    handleOlderSettled,
    toggleGroup,
    connect,
    disconnect,
    logout,
    detachLocalSession,
  }
}
