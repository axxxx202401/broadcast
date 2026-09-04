<script setup lang="ts">
import { useVirtualizer } from '@tanstack/vue-virtual'
import { computed, nextTick, ref, watch } from 'vue'
import type { VNodeRef } from 'vue'

import type { GroupDto, MessageDto } from '../types/im'
import { formatMessageTime } from '../utils/message'
import MessageBody from './MessageBody.vue'

// 父组件提供当前群组、分页状态和消息；面板负责虚拟窗口、顶部触发与前插锚点恢复。
const props = withDefaults(defineProps<{
  group: GroupDto | null
  messages: MessageDto[]
  loading: boolean
  hasOlder?: boolean
  loadingOlder?: boolean
  olderRequestToken?: number | null
}>(), {
  hasOlder: false,
  loadingOlder: false,
  olderRequestToken: null,
})
const emit = defineEmits<{
  /** 视口接近顶部且仍有历史时，请求父组件读取下一页。 */
  'load-older': []
  /** 当前历史轮次已完成锚点恢复或无新增/失败收尾，可以发布期间缓冲的实时消息。 */
  'older-settled': [token: number]
}>()

const viewport = ref<HTMLElement | null>(null)

// 加载态和空态把 count 归零，确保这两种状态不会生成虚拟行；消息键沿用协议 msg_id。
const virtualizerOptions = computed(() => ({
  count: props.loading ? 0 : props.messages.length,
  getScrollElement: () => viewport.value,
  estimateSize: () => 70,
  overscan: 8,
  getItemKey: (index: number) => props.messages[index]?.msg_id ?? index,
}))
const virtualizer = useVirtualizer<HTMLElement, HTMLLIElement>(virtualizerOptions)
const virtualItems = computed(() => virtualizer.value.getVirtualItems())
const totalSize = computed(() => virtualizer.value.getTotalSize())
const measureElement: VNodeRef = (element) => {
  if (element instanceof HTMLLIElement) virtualizer.value.measureElement(element)
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

/** 顶部阈值内只发出一次请求，直至父组件完成该轮加载。 */
function handleScroll() {
  const element = viewport.value
  if (
    !element
    || element.scrollTop > LOAD_OLDER_THRESHOLD
    || !props.hasOlder
    || props.loadingOlder
    || loadOlderRequested
  ) return

  prependAnchor = {
    messageId: props.messages[0]?.msg_id,
    totalSize: virtualizer.value.getTotalSize(),
    scrollOffset: element.scrollTop,
  }
  loadOlderRequested = true
  emit('load-older')
}

/** 按保存的消息 ID 恢复历史前插锚点；无新增、失败或锚点缺失时安全降级。 */
async function restorePrependAnchor(anchor: typeof prependAnchor) {
  if (!anchor || props.messages[0]?.msg_id === anchor.messageId) return
  await nextTick()
  const element = viewport.value
  if (!element) return
  const anchorIndex = anchor.messageId
    ? props.messages.findIndex(({ msg_id }) => msg_id === anchor.messageId)
    : -1
  if (anchorIndex >= 0) {
    virtualizer.value.scrollToIndex(anchorIndex, { align: 'start', behavior: 'auto' })
    await nextTick()
    const anchorStart = virtualizer.value.getOffsetForIndex(anchorIndex, 'start')?.[0]
    if (anchorStart !== undefined) {
      element.scrollTop = anchorStart + anchor.scrollOffset
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
</script>

<template>
  <section class="message-panel" aria-label="消息监控">
    <!-- 标题区随群组选择更新，并持续展示当前载入数量。 -->
    <header class="message-header">
      <div v-if="group">
        <p class="eyebrow">群消息</p>
        <h2>{{ group.name || `群组 ${group.group_id}` }}</h2>
      </div>
      <div v-else>
        <p class="eyebrow">全部监控群聊</p>
        <h2>全部群消息</h2>
      </div>
      <div class="stream-meta">
        <span><i class="pulse-dot"></i>正在接收</span>
        <span>{{ messages.length }} 条消息</span>
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
          <template v-if="messages[item.index]" :key="messages[item.index].msg_id">
            <time :datetime="new Date(messages[item.index].send_time < 10_000_000_000 ? messages[item.index].send_time * 1000 : messages[item.index].send_time).toISOString()">
              {{ formatMessageTime(messages[item.index].send_time) }}
            </time>
            <span v-if="!group" class="message-group">
              {{ messages[item.index].group_name || `群组 ${messages[item.index].group_id}` }} <small>#{{ messages[item.index].group_id }}</small>
            </span>
            <span class="sender-id">用户 {{ messages[item.index].send_uid }}</span>
            <MessageBody :message="messages[item.index]" />
          </template>
        </li>
      </ol>
    </div>

    <footer class="message-footer">
      <span v-if="group">群 ID：{{ group.group_id }}</span>
      <span v-else>全部监控群聊</span>
    </footer>
  </section>
</template>
