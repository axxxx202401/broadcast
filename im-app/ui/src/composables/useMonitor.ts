import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { api } from '../services/tauri'
import type { ConnectionStatus, GroupDto, MessageDto } from '../types/im'
import { isCurrentMessageRequest, mergeMessages } from '../utils/message'
import { errorMessage, normalizeConnectionStatus } from '../utils/protocol'

/**
 * 管理监控页会话、群组、消息历史和聊天连接状态。
 *
 * `pending` 只阻止本组合式函数经 `run` 发起的动作重叠，不构成后端事务或全局并发保证；
 * 消息历史另用请求编号防止陈旧响应覆盖当前群组。连接状态以事件和后端快照共同推进。
 *
 * @returns 页面状态、筛选与计数派生值，以及登录接收、群组、连接和退出操作。
 */
export function useMonitor() {
  // 会话、群组与消息均为页面本地状态；退出时无论远程结果如何都会清空。
  const loggedIn = ref(false)
  const uid = ref<string | null>(null)
  const groups = ref<GroupDto[]>([])
  const selectedGroupId = ref<string | null>(null)
  const messages = ref<MessageDto[]>([])
  const messagesLoading = ref(false)
  const search = ref('')
  const connectionStatus = ref<ConnectionStatus>('disconnected')
  const pending = ref<string | null>(null)
  const error = ref('')
  const warning = ref('')
  const unlisteners: UnlistenFn[] = []
  let messageRequestId = 0
  let connectionStatusVersion = 0
  let connectionStatusTimer: ReturnType<typeof setTimeout> | null = null

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
   * connecting 状态下每 500ms 继续轮询；version 与 uid 共同使旧会话的结果失效。
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
      connectionStatus.value = normalized
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

  /** 接受登录结果，切换本地会话并使登录前的消息历史请求失效。 */
  function acceptLogin(nextGroups: GroupDto[], nextUid: string) {
    messageRequestId += 1
    groups.value = nextGroups
    uid.value = nextUid
    loggedIn.value = true
    selectedGroupId.value = null
    messages.value = []
    messagesLoading.value = false
    syncConnectionStatus(nextUid)
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

  /** 选择群组并加载历史；messageRequestId 防止旧群组响应覆盖新选择。 */
  async function selectGroup(groupId: string) {
    const requestId = ++messageRequestId
    selectedGroupId.value = groupId
    messages.value = []
    messagesLoading.value = true
    error.value = ''
    try {
      const history = await api.getMessages(groupId)
      if (isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
        // 历史返回期间可能已收到实时消息，合并而非覆盖可保留两条来源。
        messages.value = mergeMessages(messages.value, history)
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
   * 请求远程退出并始终清理本地会话。
   * 远程失败只写入 warning，因为聊天链路是否完全断开无法由前端确认。
   */
  const logout = () =>
    run('logout', async () => {
      try {
        await api.logout()
      } catch (reason) {
        warning.value = `已退出，但断开聊天链路时出现问题：${errorMessage(reason)}`
      } finally {
        connectionStatusVersion += 1
        loggedIn.value = false
        uid.value = null
        groups.value = []
        messages.value = []
        selectedGroupId.value = null
        messageRequestId += 1
        messagesLoading.value = false
        connectionStatus.value = 'disconnected'
      }
    })

  let mounted = true

  onMounted(() => {
    // connection_status 驱动连接状态；new_message 只合并当前选中群组的实时消息。
    void Promise.allSettled([
      listen<string>('connection_status', ({ payload }) => {
        connectionStatusVersion += 1
        const status = normalizeConnectionStatus(payload)
        connectionStatus.value = status
        if (status === 'connecting') syncConnectionStatus()
      }),
      listen<MessageDto>('new_message', ({ payload }) => {
        if (payload.group_id === selectedGroupId.value) {
          messages.value = mergeMessages(messages.value, [payload])
        }
      }),
    ]).then((results) => {
      // 监听注册可能在卸载后才完成；此时立即调用返回的 unlisten，避免遗留订阅。
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
    // 同时停止状态轮询并释放所有成功注册的 Tauri 事件监听器。
    mounted = false
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
    selectedGroup,
    /** 当前群组的历史与实时消息合并结果。 */
    messages,
    messagesLoading,
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
    toggleGroup,
    connect,
    disconnect,
    logout,
  }
}
