<script setup lang="ts">
import { computed } from 'vue'

import { isEmojiOnly, tokenizeMessageText } from '../utils/emoji'

/**
 * 只用文本节点渲染消息正文：已知方括号别名转为 Unicode Emoji，其余原文原样输出。
 * 禁止 `v-html`；潜在 HTML 与未知别名一律作为文本节点，避免被解析执行。
 */
const props = defineProps<{
  /** 消息原文，可含方括号别名、原生 Emoji 或未转义标签文本。 */
  text: string
}>()

const tokens = computed(() => tokenizeMessageText(props.text))
const emojiOnly = computed(() => isEmojiOnly(tokens.value))
</script>

<template>
  <p class="message-text" :class="{ 'message-text--emoji-only': emojiOnly }">
    <template v-for="(token, index) in tokens" :key="index">
      <span v-if="token.kind === 'emoji'" class="inline-emoji" :aria-label="token.source">{{ token.value }}</span>
      <span v-else>{{ token.value }}</span>
    </template>
  </p>
</template>
