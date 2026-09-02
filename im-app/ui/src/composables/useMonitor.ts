import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { api } from '../services/tauri'
import type { ConnectionStatus, GroupDto, MessageDto } from '../types/im'
import { isCurrentMessageRequest, mergeMessages } from '../utils/message'
import { errorMessage, normalizeConnectionStatus } from '../utils/protocol'

export function useMonitor() {
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

  const fetchGroups = () =>
    run('fetch', async () => {
      groups.value = await api.fetchGroups()
    })

  const refreshGroups = () =>
    run('refresh', async () => {
      groups.value = await api.refreshGroups()
    })

  async function selectGroup(groupId: string) {
    const requestId = ++messageRequestId
    selectedGroupId.value = groupId
    messages.value = []
    messagesLoading.value = true
    error.value = ''
    try {
      const history = await api.getMessages(groupId)
      if (isCurrentMessageRequest(requestId, messageRequestId, groupId, selectedGroupId.value)) {
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

  const toggleGroup = (group: GroupDto) =>
    run(`toggle-${group.group_id}`, async () => {
      const monitored = group.monitored === 0
      await api.toggleMonitor(group.group_id, monitored)
      groups.value = groups.value.map((item) =>
        item.group_id === group.group_id ? { ...item, monitored: monitored ? 1 : 0 } : item,
      )
    })

  const connect = () =>
    run('connect', async () => {
      await api.connectChat()
    })

  const disconnect = () =>
    run('disconnect', async () => {
      await api.disconnectChat()
      connectionStatus.value = 'disconnected'
    })

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
    mounted = false
    if (connectionStatusTimer) clearTimeout(connectionStatusTimer)
    for (const unlisten of unlisteners) unlisten()
  })

  return {
    loggedIn,
    uid,
    groups,
    filteredGroups,
    selectedGroup,
    messages,
    messagesLoading,
    search,
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
