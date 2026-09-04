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
</script>

<template>
  <article class="message-card">
    <div v-if="showGroup" class="message-source">
      {{ message.group_name || `群 ${message.group_id}` }} <small>#{{ message.group_id }}</small>
    </div>
    <div class="message-meta">
      <span class="message-sender">用户 {{ message.send_uid }}</span>
      <time :datetime="isoTime">{{ formatMessageTime(message.send_time) }}</time>
    </div>
    <div class="message-content">
      <MessageBody :message="message" />
    </div>
  </article>
</template>
