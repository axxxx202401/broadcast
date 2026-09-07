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
    console.log('[useLottery] loadConfig: 开始加载配置')
    try {
      const result = await api.getLotteryConfig()
      console.log('[useLottery] loadConfig: 后端返回结果', JSON.stringify(result))
      config.value = result
      console.log('[useLottery] loadConfig: config.value 已更新', {
        api_url: config.value.api_url,
        current_issues: config.value.current_issues
      })
    } catch (reason) {
      console.log('[useLottery] loadConfig: 加载失败', reason)
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
    console.log('[useLottery] fetchHistory: 开始拉取历史')
    loading.value = true
    error.value = ''
    try {
      const items = await api.fetchLotteryHistory()
      console.log('[useLottery] fetchHistory: API返回数据', { count: items.length, firstItem: items[0] })
      drawHistory.value = items.slice(0, 20)
      console.log('[useLottery] fetchHistory: drawHistory 已更新', drawHistory.value.length, '条')
      // 同步最新期号到 DB，确保消息匹配使用最新期号列表。
      // 注意：此处不依赖前端 config.value.api_url，因为 fetchLotteryHistory
      // 后端已自行处理 DB → 默认值的 fallback。items 非空说明 URL 可用。
      if (items.length > 0) {
        const issues = items.map(item => item.preDrawIssue)
        console.log('[useLottery] fetchHistory: 准备写库', { url: config.value.api_url, issues_count: issues.length })
        // 用后端已解析的 URL（DB 值或默认值）写库，避免用前端空字符串覆盖。
        const url = config.value.api_url
        await api.setLotteryConfig(url, issues)
        console.log('[useLottery] fetchHistory: 写库完成，重新加载配置')
        await loadConfig()
      }
    } catch (reason) {
      console.log('[useLottery] fetchHistory: 拉取失败', reason)
      const msg = errorMessage(reason)
      if (!msg.includes('URL not configured')) {
        error.value = `拉取开奖历史失败：${msg}`
      }
    } finally {
      loading.value = false
      console.log('[useLottery] fetchHistory: 完成')
    }
  }

  /**
   * 挂载/登录后首次触发：加载配置 → 拉取历史 → 写库（若有数据）。
   * 已登录且 DB 已有期号时直接跳过，避免重复 I/O。
   */
  async function prefetchWithDefault() {
    console.log('[useLottery] prefetchWithDefault: 开始执行')
    console.log('[useLottery] prefetchWithDefault: loggedIn=', loggedIn?.value)
    // 先加载配置（包含后端注入的默认值）。
    await loadConfig()
    console.log('[useLottery] prefetchWithDefault: 配置加载完成', {
      api_url: config.value.api_url,
      issues_count: config.value.current_issues.length
    })
    // DB 已有期号则无需重复拉取 API，但需要从 DB 重建 drawHistory。
    if (config.value.current_issues.length > 0) {
      console.log('[useLottery] prefetchWithDefault: DB已有期号，从DB重建drawHistory')
      // 从 current_issues 重建 drawHistory（按降序排列）
      const issues = config.value.current_issues.slice(0, 20)
      drawHistory.value = issues.map(issue => ({
        preDrawIssue: issue,
        preDrawTime: '', // DB 没有保存时间，留空
      }))
      console.log('[useLottery] prefetchWithDefault: drawHistory 已重建', drawHistory.value.length, '条')
      schedulePoll()
      return
    }
    console.log('[useLottery] prefetchWithDefault: DB无期号，开始拉取历史')
    void fetchHistory().then(() => {
      console.log('[useLottery] prefetchWithDefault: fetchHistory完成，开始轮询')
      schedulePoll()
    })
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
    void prefetchWithDefault()
  }

  onMounted(() => {
    console.log('[useLottery] onMounted: loggedIn=', loggedIn?.value)
    void runPrefetch()
  })

  if (loggedIn) {
    watch(
      () => loggedIn.value,
      (val) => {
        console.log('[useLottery] loggedIn changed to', val)
        if (val) {
          console.log('[useLottery] 登录成功，触发 prefetch')
          void runPrefetch()
        }
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
    loadConfig,
    saveConfig,
    fetchHistory,
    /** 当收到含"开奖"的消息时手动触发一次刷新。 */
    refreshOnDrawMessage: fetchHistory,
  }
}
