<script setup lang="ts">
import { computed } from 'vue'

import type { MessageDto } from '../types/im'
import { formatMessageTime } from '../utils/message'
import MessageBody from './MessageBody.vue'

/**
 * 内容优先的单条消息卡片。
 *
 * 视觉顺序固定为：可选群来源 → 发送人与时间 → 正文。元信息弱化，正文由 `MessageBody` 渲染。
 * 卡片自身的 margin / padding / border 放在 `article` 上，避免落到虚拟列表 `li` 引起外边距折叠。
 */
const props = defineProps<{
  /** 当前行对应的消息。 */
  message: MessageDto
  /** 全部群消息时为真，显示群名称与 `#group_id`；单群视图不重复群来源。 */
  showGroup: boolean
}>()

/** `send_time` 小于 1e10 按 Unix 秒，否则按毫秒，与 `formatMessageTime` 同一启发式。 */
const isoTime = computed(() => {
  const milliseconds = props.message.send_time < 10_000_000_000
    ? props.message.send_time * 1000
    : props.message.send_time
  return new Date(milliseconds).toISOString()
})

/** 仅广播消息展示状态；普通消息的默认值 0 不额外占用视觉空间。 */
const broadcastStatus = computed(() => {
  if (props.message.broadcast_status === 1) return { text: '发送成功', className: 'is-success' }
  if (props.message.broadcast_status === 2) return { text: '发送失败', className: 'is-failed' }
  if (props.message.msg_id.startsWith('-')) return { text: '发送中', className: 'is-sending' }
  return null
})
</script>

<template>
  <article class="message-card">
    <div v-if="showGroup" class="message-source">
      {{ message.group_name || `群 ${message.group_id}` }} <small>#{{ message.group_id }}</small>
    </div>
    <div class="message-meta">
      <span class="message-sender">用户 {{ message.send_uid }}</span>
      <span
        v-if="broadcastStatus"
        :class="['broadcast-status', broadcastStatus.className]"
      >{{ broadcastStatus.text }}</span>
      <time :datetime="isoTime">{{ formatMessageTime(message.send_time) }}</time>
    </div>
    <div class="message-content">
      <MessageBody :message="message" />
    </div>
  </article>
</template>

<style scoped>
.broadcast-status {
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
}

.broadcast-status.is-sending {
  color: var(--accent);
  background: rgba(240, 180, 70, 0.12);
}

.broadcast-status.is-success {
  color: var(--success);
  background: rgba(63, 185, 80, 0.12);
}

.broadcast-status.is-failed {
  color: var(--danger);
  background: rgba(248, 81, 73, 0.12);
}
</style>
