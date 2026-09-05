<script setup lang="ts">
import { useVirtualizer } from '@tanstack/vue-virtual'
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import type { VNodeRef } from 'vue'

import type { GroupDto, MessageDto } from '../types/im'
import LotteryPanel from './LotteryPanel.vue'
import MessageCard from './MessageCard.vue'
import MonitoredGroupSummary from './MonitoredGroupSummary.vue'

// 父组件提供当前群组、分页状态和消息；面板负责虚拟窗口、顶部触发与前插锚点恢复。
const props = withDefaults(defineProps<{
  group: GroupDto | null
  messages: MessageDto[]
  loading: boolean
  hasOlder?: boolean
  loadingOlder?: boolean
  olderRequestToken?: number | null
  /** 正在监控的群 ID；仅在全部群消息标题下展示，默认空数组。 */
  monitoredGroupIds?: string[]
  /** 群组总数，显示在标题栏统计区域。 */
  totalGroups?: number
  /** 当前监控中的群组数，显示在标题栏统计区域。 */
  monitoredCount?: number
  /** 父组件共享的开奖 composable；用于在消息区顶部嵌入迷你开奖面板。 */
  lottery?: {
    config: import('vue').Ref<{ api_url: string; current_issues: number[] }>
    drawHistory: import('vue').Ref<import('../services/tauri').DrawItem[]>
    loading: import('vue').Ref<boolean>
    error: import('vue').Ref<string>
    saveConfig: (url: string, issues: number[]) => Promise<void>
    fetchHistory: () => Promise<void>
  }
}>(), {
  hasOlder: false,
  loadingOlder: false,
  olderRequestToken: null,
  monitoredGroupIds: () => [],
  totalGroups: 0,
  monitoredCount: 0,
})
const emit = defineEmits<{
  /** 视口接近顶部且仍有历史时，请求父组件读取下一页。 */
  'load-older': []
  /** 当前历史轮次已完成锚点恢复或无新增/失败收尾，可以发布期间缓冲的实时消息。 */
  'older-settled': [token: number]
}>()

const viewport = ref<HTMLElement | null>(null)

/**
 * 按解密正文类型给出保守行高估算。
 * 虚拟列表在 `measureElement` 完成真实测量前用该值占位；图片/视频与长文本用更高估算，
 * 避免历史前插时用统一行高把 `msg_id` 锚点算偏。
 */
function estimateMessageHeight(message: MessageDto | undefined): number {
  if (!message?.decoded_content) return 96
  switch (message.decoded_content.kind) {
    case 'image':
    case 'video': return 220
    case 'audio':
    case 'file': return 112
    case 'text': return Math.min(220, 76 + Math.floor(message.decoded_content.text.length / 48) * 22)
  }
}

/**
 * 优先用 ResizeObserver 的 borderBox，否则读元素当前盒。
 * 不走默认实现里“无 entry 则返回缓存”的路径，媒体撑高后即使回调稍晚也能读到新高度。
 */
function measureMessageRow(element: HTMLLIElement, entry: ResizeObserverEntry | undefined): number {
  const box = entry?.borderBoxSize?.[0]
  if (box) return Math.round(box.blockSize)
  return Math.round(element.getBoundingClientRect().height)
}

// 加载态和空态把 count 归零，确保这两种状态不会生成虚拟行；消息键沿用协议 msg_id。
const virtualizerOptions = computed(() => ({
  count: props.loading ? 0 : props.messages.length,
  getScrollElement: () => viewport.value,
  estimateSize: (index: number) => estimateMessageHeight(props.messages[index]),
  overscan: 8,
  getItemKey: (index: number) => props.messages[index]?.msg_id ?? index,
  measureElement: measureMessageRow,
  // 非零初值避免首帧 `outerSize === 0` 时不算可视范围，媒体重测与锚点恢复才能挂到行。
  initialRect: { width: 800, height: 600 },
}))
const virtualizer = useVirtualizer<HTMLElement, HTMLLIElement>(virtualizerOptions)
const virtualItems = computed(() => virtualizer.value.getVirtualItems())
const totalSize = computed(() => virtualizer.value.getTotalSize())

const measureElement: VNodeRef = (element) => {
  if (element instanceof HTMLElement && element.tagName === 'LI') {
    virtualizer.value.measureElement(element as HTMLLIElement)
  }
}

const AUTO_SCROLL_THRESHOLD = 80
const LOAD_OLDER_THRESHOLD = 80

/** 发起向上加载时的首条消息锚点、行内偏移和总高度回退信息。 */
let prependAnchor: {
  messageId: string | undefined
  totalSize: number
  scrollOffset: number
} | null = null
let loadOlderRequested = false
let observedOlderToken: number | null = null
let olderSettleCycle = 0

/**
 * 顶部阈值内只发出一次请求，直至父组件完成该轮加载。
 * 已请求但用户仍停在顶部时，只刷新行内偏移，避免程序化滚底留下的 `scrollOffset = 0` 污染锚点。
 */
function handleScroll() {
  const element = viewport.value
  if (
    !element
    || element.scrollTop > LOAD_OLDER_THRESHOLD
    || !props.hasOlder
    || props.loadingOlder
  ) return

  prependAnchor = {
    messageId: prependAnchor?.messageId ?? props.messages[0]?.msg_id,
    totalSize: prependAnchor?.totalSize ?? virtualizer.value.getTotalSize(),
    scrollOffset: element.scrollTop,
  }
  if (loadOlderRequested) return
  loadOlderRequested = true
  emit('load-older')
}

/**
 * 按保存的消息 ID 定位锚点行，再用实测 `start + scrollOffset` 恢复滚动。
 * 不走 `scrollToIndex`：其对齐 reconcile 会抹掉行内偏移。无新增、失败或锚点缺失时安全降级。
 */
async function restorePrependAnchor(anchor: typeof prependAnchor) {
  if (!anchor || props.messages[0]?.msg_id === anchor.messageId) return
  await nextTick()
  const element = viewport.value
  if (!element) return
  const anchorIndex = anchor.messageId
    ? props.messages.findIndex(({ msg_id }) => msg_id === anchor.messageId)
    : -1
  if (anchorIndex >= 0) {
    const anchorStart = virtualizer.value.getOffsetForIndex(anchorIndex, 'start')?.[0]
    if (anchorStart !== undefined) {
      virtualizer.value.scrollToOffset(anchorStart + anchor.scrollOffset, {
        align: 'start',
        behavior: 'auto',
      })
    } else {
      element.scrollTop += anchor.scrollOffset
    }
  } else {
    const insertedSize = Math.max(0, virtualizer.value.getTotalSize() - anchor.totalSize)
    element.scrollTop = anchor.scrollOffset + insertedSize
  }
}

/**
 * 单一协调 watcher 管理历史轮次：观察 true 时锁定 token，观察 false 后恢复锚点并发出
 * `older-settled`。父级以 token 拒绝陈旧握手，因此切群竞态不会发布旧范围缓冲。
 */
watch(
  () => [props.loadingOlder, props.olderRequestToken] as const,
  async ([loadingOlder, token]) => {
    if (loadingOlder) {
      observedOlderToken = token
      olderSettleCycle += 1
      return
    }
    const settlingToken = observedOlderToken
    if (settlingToken === null) return
    const settlingCycle = olderSettleCycle
    const anchor = prependAnchor
    await nextTick()
    await restorePrependAnchor(anchor)
    if (settlingCycle !== olderSettleCycle) return
    loadOlderRequested = false
    observedOlderToken = null
    if (prependAnchor === anchor) prependAnchor = null
    emit('older-settled', settlingToken)
  },
)

/**
 * 首批非空消息定位到底部；后续仅在更新前仍接近底部时跟随新消息。
 * 使用 `auto` 避免实时批次连续到达时累积平滑滚动动画。
 */
watch(
  () => [
    props.loading,
    props.messages.length,
    props.messages[0]?.msg_id,
    props.messages.at(-1)?.msg_id,
  ] as const,
  async ([loading, count, _firstMessageId, lastMessageId], previous) => {
    if (loading || count === 0) return

    const [wasLoading, previousCount, , previousLastMessageId] = previous ?? [true, 0]
    const isInitialLoad = wasLoading || previousCount === 0
    if (!isInitialLoad) {
      // 实时窗口达到上限后 length 固定，必须以尾 ID 变化识别新消息；历史前插期间则由
      // 锚点 watcher 独占滚动恢复，即使尾部同时因裁剪变化也绝不自动滚底。
      if (lastMessageId === previousLastMessageId || prependAnchor || props.loadingOlder) return
    }

    const element = viewport.value
    const wasNearBottom = element
      ? element.scrollHeight - element.scrollTop - element.clientHeight <= AUTO_SCROLL_THRESHOLD
      : false
    if (!isInitialLoad && !wasNearBottom) return

    await nextTick()
    virtualizer.value.scrollToIndex(count - 1, { align: 'end', behavior: 'auto' })
  },
  { immediate: true },
)

/** 新尾消息高亮持续时间；到期后从集合删除，虚拟行重挂载不会再次点亮。 */
const NEW_HIGHLIGHT_MS = 1200
/** 当前仍在高亮窗口内的 msg_id；必须整体替换 Set，原地 mutate 不会触发视图更新。 */
const highlightedIds = ref(new Set<string>())
const highlightTimers = new Map<string, ReturnType<typeof setTimeout>>()

/** 将一条新尾消息加入高亮集合，并在 1.2s 后移除该 ID。 */
function markNewTailMessage(msgId: string) {
  const pending = highlightTimers.get(msgId)
  if (pending !== undefined) clearTimeout(pending)

  const next = new Set(highlightedIds.value)
  next.add(msgId)
  highlightedIds.value = next

  highlightTimers.set(msgId, setTimeout(() => {
    highlightTimers.delete(msgId)
    const remaining = new Set(highlightedIds.value)
    remaining.delete(msgId)
    highlightedIds.value = remaining
  }, NEW_HIGHLIGHT_MS))
}

/** 卸载时清掉未触发的高亮定时器，避免对已销毁实例写回 Set。 */
function clearHighlightTimers() {
  for (const timer of highlightTimers.values()) clearTimeout(timer)
  highlightTimers.clear()
  highlightedIds.value = new Set()
}

onUnmounted(clearHighlightTimers)

/**
 * 比较上一次尾部 `msg_id`：仅非初次、非历史前插的新尾 ID 进入高亮集合。
 * 初次载入、loading 结束后的首批、以及 `prependAnchor` / `loadingOlder` 期间的尾变化一律忽略。
 */
watch(
  () => [
    props.loading,
    props.messages.length,
    props.messages.at(-1)?.msg_id,
  ] as const,
  ([loading, count, lastMessageId], previous) => {
    if (loading || count === 0 || lastMessageId === undefined) return

    const [wasLoading, previousCount, previousLastMessageId] = previous ?? [true, 0, undefined]
    const isInitialLoad = wasLoading || previousCount === 0
    if (isInitialLoad) return
    if (prependAnchor || props.loadingOlder) return
    if (lastMessageId === previousLastMessageId) return

    markNewTailMessage(lastMessageId)
  },
)
</script>

<template>
  <section class="message-panel" aria-label="消息监控">
    <!-- 标题区随群组选择更新，并持续展示当前载入数量。 -->
    <header class="message-header">
      <!-- 第1行：标题 -->
      <div class="message-header-title">
        <h2>{{ group ? (`${group.name} (${group.group_id})`) : '全部群消息' }}</h2>
      </div>
      <!-- 第2行：群 ID 列表，全部群消息模式展示。 -->
      <MonitoredGroupSummary v-if="!group" :group-ids="monitoredGroupIds" class="header-group-ids" />
      <!-- 第3行：左侧统计信息（两行），右侧开奖信息（两行），并排显示。 -->
      <div class="message-header-row">
        <div class="stream-meta">
          <div class="meta-group">
            <span>群组总数 <strong>{{ totalGroups }}</strong></span>
            <span class="meta-divider">·</span>
            <span>监控中 <strong class="metric-live">{{ monitoredCount }}</strong></span>
          </div>
          <div class="meta-group">
            <span><i class="pulse-dot"></i>正在接收</span>
            <span class="meta-divider">·</span>
            <span>{{ messages.length }} 条消息</span>
          </div>
        </div>
        <!-- 开奖信息：紧凑竖排，与统计信息并排显示在标题栏右侧。 -->
        <LotteryPanel v-if="lottery" class="header-lottery" :lottery="lottery" />
      </div>
    </header>

    <!-- 内容区按优先级呈现加载中、未选择群组、已选但为空、消息列表四种状态。 -->
    <div ref="viewport" class="message-viewport" aria-live="polite" @scroll="handleScroll">
      <div v-if="loading" class="panel-empty">
        <span class="loader-grid" aria-hidden="true"><i></i><i></i><i></i><i></i></span>
        <p>正在读取本地历史记录</p>
      </div>
      <div v-else-if="messages.length === 0" class="panel-empty">
        <span class="empty-glyph" aria-hidden="true">Ø</span>
        <strong>暂无已存储消息</strong>
        <p>选择需要监控的群后，新消息会显示在这里</p>
      </div>
      <!-- 状态条覆盖在虚拟容器顶部，不参与列表高度和虚拟行索引。 -->
      <div v-else class="history-status" role="status">
        {{ loadingOlder ? '正在加载更早消息…' : hasOlder ? '向上滚动加载更早消息' : '已到最早消息' }}
      </div>
      <!-- 虚拟容器保留完整滚动高度，仅挂载可视区及 overscan 范围内的语义列表项。 -->
      <ol
        v-if="!loading && messages.length > 0"
        class="message-log"
        :class="{ 'all-groups': !group }"
        :style="{ height: `${totalSize}px` }"
      >
        <li
          v-for="item in virtualItems"
          :key="messages[item.index].msg_id"
          :ref="measureElement"
          :data-index="item.index"
          :style="{ transform: `translateY(${item.start}px)` }"
        >
          <!-- 行内只挂卡片；边距与边框在 article 上，避免绝对定位 li 外边距折叠。 -->
          <MessageCard
            v-if="messages[item.index]"
            :message="messages[item.index]"
            :show-group="!group"
            :class="{ 'message-card--new': highlightedIds.has(messages[item.index].msg_id) }"
          />
        </li>
      </ol>
    </div>

    <footer class="message-footer">
      <span v-if="group">群 ID：{{ group.group_id }}</span>
      <span v-else>全部监控群聊</span>
    </footer>
  </section>
</template>
