<script setup lang="ts">
import { computed } from 'vue'
import type { ConnectionStatus } from '../types/im'

const props = defineProps<{ status: ConnectionStatus }>()

// 将断开、连接中、已连接三态映射为面向操作员的状态文案；样式类沿用原始状态值。
const label = computed(() => ({
  disconnected: '链路离线',
  connecting: '链路建立中',
  connected: '链路在线',
})[props.status])
</script>

<template>
  <span class="status-badge" :class="`status-${status}`" role="status">
    <i aria-hidden="true"></i>{{ label }}
  </span>
</template>
