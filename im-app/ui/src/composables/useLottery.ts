import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { api } from '../services/tauri'
import type { DrawItem, LotteryConfig } from '../services/tauri'
import { errorMessage } from '../utils/protocol'

/**
 * 管理当前账号的开奖配置与开奖历史面板状态。
 *
 * - 挂载时加载已持久化配置，并监听后端统一开奖轮询事件。
 * - 周期请求只由后端执行；前端仅在用户手动刷新时主动调用一次。
 * - 历史只显示最新两条（本期 / 上期），配置变更立即生效于消息匹配。
 */
export function useLottery(loggedIn?: { value: boolean }) {
  const config = ref<LotteryConfig>({
    api_url: '',
    current_issues: [],
  })
  const drawHistory = ref<DrawItem[]>([])
  const loading = ref(false)
  const error = ref('')

  /** 当前关注的期号列表；未配置时为空数组。 */
  const currentIssues = computed(() => config.value.current_issues)

  /** 加载当前账号的开奖配置。后端会在 DB 无记录时返回构建期注入的默认 API URL。 */
  async function loadConfig() {
    try {
      const result = await api.getLotteryConfig()
      config.value = result
    } catch (reason) {
      error.value = `加载开奖配置失败：${errorMessage(reason)}`
    }
  }

  /** 保存开奖配置（期号列表从 drawHistory 提取）并立即重新拉取历史。 */
  async function saveConfig(api_url: string, current_issues: number[]) {
    try {
      await api.setLotteryConfig(api_url, current_issues)
      await loadConfig()
      await fetchHistory()
    } catch (reason) {
      error.value = `保存配置失败：${errorMessage(reason)}`
    }
  }

  /**
   * 从远端拉取开奖历史并更新显示；同时将最新期号同步回后端 DB。
   *
   * URL 来源优先级：DB > 构建期默认值。只要后端返回了数据，就写库；
   * URL 未配置时静默跳过，不展示错误。
   */
  async function fetchHistory() {
    loading.value = true
    error.value = ''
    try {
      const items = await api.fetchLotteryHistory()
      applyDrawHistory(items)
      // 手动刷新命令已在后端同步 URL 与 current_issues，这里只重读最终配置。
      if (items.length > 0) await loadConfig()
    } catch (reason) {
      const msg = errorMessage(reason)
      if (!msg.includes('URL not configured')) {
        error.value = `拉取开奖历史失败：${msg}`
      }
    } finally {
      loading.value = false
    }
  }

  /**
   * 挂载或登录后读取配置，并用已保存期号提供事件到达前的轻量占位数据。
   * 完整号码和值只来自后端 `lottery_history_updated` 事件。
   */
  async function prefetchWithDefault() {
    await loadConfig()
    if (config.value.current_issues.length > 0) {
      const issues = config.value.current_issues.slice(0, 20)
      drawHistory.value = issues.map(issue => ({
        preDrawIssue: issue,
        preDrawTime: '', // DB 没有保存时间，留空
        preDrawCode: '',
        sumNum: 0,
        sumBigSmall: -1,
        sumSingleDouble: -1,
      }))
    }
  }

  /** 应用后端单次轮询结果，并让本地匹配期号与该批数据保持一致。 */
  function applyDrawHistory(items: DrawItem[]) {
    drawHistory.value = items.slice(0, 20)
    config.value = {
      ...config.value,
      current_issues: items.map(item => item.preDrawIssue),
    }
  }

  let historyUnlisten: Promise<UnlistenFn> | null = null

  /** 登录后（含恢复登录成功）触发一次拉取；未登录时静默跳过。 */
  function runPrefetch() {
    if (loggedIn?.value !== true) return
    void prefetchWithDefault()
  }

  onMounted(() => {
    historyUnlisten = listen<DrawItem[]>('lottery_history_updated', ({ payload }) => {
      applyDrawHistory(payload)
    })
    historyUnlisten.catch((reason) => {
      console.error('Failed to listen for lottery history updates:', errorMessage(reason))
    })
    void runPrefetch()
  })

  if (loggedIn) {
    watch(
      () => loggedIn.value,
      (val) => {
        if (val) {
          void runPrefetch()
        }
      },
    )
  }

  onBeforeUnmount(() => {
    historyUnlisten?.then((unlisten) => unlisten()).catch(() => {})
    historyUnlisten = null
  })

  return {
    config,
    drawHistory,
    currentIssues,
    loading,
    error,
    loadConfig,
    saveConfig,
    fetchHistory,
  }
}
