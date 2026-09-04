<script setup lang="ts">
import { computed } from 'vue'
import type { ConnectionStatus } from '../types/im'

const props = defineProps<{ status: ConnectionStatus }>()

/** 连接三态的普通用户文案；样式类仍使用协议状态值。 */
const connectionLabels = {
  disconnected: '已断开',
  connecting: '连接中',
  connected: '已连接',
} as const

const label = computed(() => connectionLabels[props.status])
</script>

<template>
  <span class="status-badge" :class="`status-${status}`" role="status">
    <i aria-hidden="true"></i>{{ label }}
  </span>
</template>
