import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { api } from '../services/tauri'
import type { DrawItem, LotteryConfig } from '../services/tauri'
import { errorMessage } from '../utils/protocol'

/** 开奖历史轮询间隔（毫秒）。 */
const POLL_INTERVAL_MS = 30_000

/**
 * 管理当前账号的开奖配置与开奖历史面板状态。
 *
 * - 挂载时自动加载配置并拉取一次历史。
 * - 每 30 秒轮询一次，并在收到含"开奖"的消息时额外触发一次。
 * - 历史只显示最新两条（本期 / 上期），配置变更立即生效于消息匹配。
 */
export function useLottery(loggedIn?: { value: boolean }) {
  const config = ref<LotteryConfig>({
    api_url: 'https://go124.com/api/hash/get28HistoryList/10091',
    current_issues: [],
  })
  const drawHistory = ref<DrawItem[]>([])
  const loading = ref(false)
  const error = ref('')

  /** 当前关注的期号列表；未配置时为空数组。 */
  const currentIssues = computed(() => config.value.current_issues)

  /** 加载当前账号的开奖配置。 */
  async function loadConfig() {
    try {
      config.value = await api.getLotteryConfig()
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

  /** 从远端拉取开奖历史并更新显示。URL 未配置时静默跳过。 */
  async function fetchHistory() {
    loading.value = true
    error.value = ''
    try {
      const items = await api.fetchLotteryHistory()
      drawHistory.value = items.slice(0, 2)
    } catch (reason) {
      // URL 未配置属于正常初始状态，不展示错误。
      const msg = errorMessage(reason)
      if (!msg.includes('URL not configured')) {
        error.value = `拉取开奖历史失败：${msg}`
      }
    } finally {
      loading.value = false
    }
  }

  /** 挂载时先拉取开奖历史，再以实际期号保存到后端，避免空数组覆盖已有配置。
   * 若 DB 中已有非空 config，直接跳过，不做重复保存。 */
  async function prefetchWithDefault(_current_issues: number[]) {
    const defaultUrl = config.value.api_url
    if (!defaultUrl) {
      await loadConfig()
      void fetchHistory()
      return
    }
    try {
      // 已有非空 config 时直接跳过，不重复保存。
      if (config.value.current_issues.length > 0) return
      // 先拉取历史，拿到实际期号后再保存，绝不传空数组。
      await fetchHistory()
      if (drawHistory.value.length > 0) {
        const issues = drawHistory.value.map(item => item.preDrawIssue)
        await api.setLotteryConfig(defaultUrl, issues)
        await loadConfig()
      }
    } catch (_e) {
      // 保存失败（如未登录）：静默跳过，等登录后 watch 再触发。
    }
  }

  let timer: ReturnType<typeof setTimeout> | null = null

  function schedulePoll() {
    if (timer) clearTimeout(timer)
    timer = setTimeout(async () => {
      await fetchHistory()
      schedulePoll()
    }, POLL_INTERVAL_MS)
  }

  /** 登录后（含恢复登录成功）触发一次拉取；未登录时静默跳过。 */
  function runPrefetch() {
    if (loggedIn?.value !== true) return
    void prefetchWithDefault([]).then(schedulePoll)
  }

  onMounted(() => {
    void runPrefetch()
  })

  if (loggedIn) {
    watch(
      () => loggedIn.value,
      (val) => {
        if (val) void runPrefetch()
      },
    )
  }

  onBeforeUnmount(() => {
    if (timer) clearTimeout(timer)
  })

  return {
    config,
    drawHistory,
    currentIssues,
    loading,
    error,
    saveConfig,
    fetchHistory,
    /** 当收到含"开奖"的消息时手动触发一次刷新。 */
    refreshOnDrawMessage: fetchHistory,
  }
}
