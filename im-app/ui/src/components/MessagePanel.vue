<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'

import type { GroupDto, MessageDto } from '../types/im'
import { decodeMessageContent, formatMessageTime } from '../utils/message'

const props = defineProps<{
  group: GroupDto | null
  messages: MessageDto[]
  loading: boolean
}>()

const viewport = ref<HTMLElement | null>(null)

watch(
  () => props.messages.length,
  async () => {
    await nextTick()
    viewport.value?.scrollTo({ top: viewport.value.scrollHeight, behavior: 'smooth' })
  },
)
</script>

<template>
  <section class="message-panel" aria-label="消息监控">
    <header class="message-header">
      <div v-if="group">
        <p class="eyebrow">LIVE MESSAGE STREAM</p>
        <h2>{{ group.name || `群组 ${group.group_id}` }}</h2>
      </div>
      <div v-else>
        <p class="eyebrow">CHANNEL NOT SELECTED</p>
        <h2>消息流</h2>
      </div>
      <div class="stream-meta">
        <span><i class="pulse-dot"></i>只读采集</span>
        <span>{{ messages.length }} 条已载入</span>
      </div>
    </header>

    <div ref="viewport" class="message-viewport" aria-live="polite">
      <div v-if="loading" class="panel-empty">
        <span class="loader-grid" aria-hidden="true"><i></i><i></i><i></i><i></i></span>
        <p>正在读取本地历史记录</p>
      </div>
      <div v-else-if="!group" class="panel-empty">
        <span class="empty-glyph" aria-hidden="true">⌁</span>
        <strong>选择左侧群组以检查消息流</strong>
        <p>实时事件仅追加到当前选中的群组视图</p>
      </div>
      <div v-else-if="messages.length === 0" class="panel-empty">
        <span class="empty-glyph" aria-hidden="true">Ø</span>
        <strong>暂无已存储消息</strong>
        <p>开启群监控并连接聊天链路后等待新消息</p>
      </div>
      <ol v-else class="message-log">
        <li v-for="message in messages" :key="message.msg_id">
          <time :datetime="new Date(message.send_time < 10_000_000_000 ? message.send_time * 1000 : message.send_time).toISOString()">
            {{ formatMessageTime(message.send_time) }}
          </time>
          <span class="sender-id">UID {{ message.send_uid }}</span>
          <p>{{ decodeMessageContent(message.content_b64) }}</p>
          <span class="message-type">T{{ message.msg_type }}</span>
        </li>
      </ol>
    </div>

    <footer class="message-footer">
      <span>消息内容按后端 DTO 的 Base64 契约解码显示</span>
      <span v-if="group">CHANNEL / {{ group.group_id }}</span>
    </footer>
  </section>
</template>
