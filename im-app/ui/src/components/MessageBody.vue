<script setup lang="ts">
import { convertFileSrc } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'

import { api } from '../services/tauri'
import type { MessageDto } from '../types/im'
import { decodeMessageContent } from '../utils/message'

const props = defineProps<{ message: MessageDto }>()

const localUrl = ref('')
const loading = ref(false)
const error = ref('')

const content = computed(() => props.message.decoded_content)
const kindLabel = computed(() => {
  switch (content.value?.kind) {
    case 'text': return '文本'
    case 'image': return '图片'
    case 'audio': return '音频'
    case 'video': return '视频'
    case 'file': return '文件'
    default: return `类型 ${props.message.msg_type}`
  }
})

watch(
  () => props.message.msg_id,
  () => {
    localUrl.value = ''
    error.value = ''
    loading.value = false
  },
)

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '大小未知'
  if (value < 1024) return `${value} B`
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`
}

/**
 * 让 Rust 下载并解密附件。图片优先请求缩略图，其他媒体请求主附件；
 * 远端 URL 和 fileKey 不进入 DOM，WebView 只读取受 asset protocol 限制的本地缓存。
 */
async function loadAttachment() {
  if (loading.value || localUrl.value) return
  loading.value = true
  error.value = ''
  try {
    const result = await api.downloadMessageAttachment(
      props.message.msg_id,
      content.value?.kind === 'image',
    )
    localUrl.value = convertFileSrc(result.path)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="message-body" :class="`message-body--${content?.kind ?? 'unknown'}`">
    <template v-if="content?.kind === 'text'">
      <p>{{ content.text }}</p>
    </template>

    <template v-else-if="content?.kind === 'image'">
      <img v-if="localUrl" :src="localUrl" :alt="`${message.group_name || message.group_id}中的图片`" />
      <button v-else type="button" class="media-load" :disabled="loading" @click="loadAttachment">
        {{ loading ? '图片解密中…' : '加载图片' }}
      </button>
      <small>{{ content.width }}×{{ content.height }} · {{ formatBytes(content.file_size) }}</small>
    </template>

    <template v-else-if="content?.kind === 'audio'">
      <audio v-if="localUrl" :src="localUrl" controls preload="metadata"></audio>
      <button v-else type="button" class="media-load" :disabled="loading" @click="loadAttachment">
        {{ loading ? '音频解密中…' : `加载音频 · ${content.duration}s` }}
      </button>
    </template>

    <template v-else-if="content?.kind === 'video'">
      <video v-if="localUrl" :src="localUrl" controls preload="metadata"></video>
      <button v-else type="button" class="media-load" :disabled="loading" @click="loadAttachment">
        {{ loading ? '视频解密中…' : `加载视频 · ${content.duration}s` }}
      </button>
      <small>{{ content.width }}×{{ content.height }} · {{ formatBytes(content.file_size) }}</small>
    </template>

    <template v-else-if="content?.kind === 'file'">
      <strong>{{ content.name || '未命名文件' }}</strong>
      <small>{{ content.mime_type || '未知类型' }} · {{ formatBytes(content.file_size) }}</small>
      <a v-if="localUrl" :href="localUrl" :download="content.name || 'attachment'">保存文件</a>
      <button v-else type="button" class="media-load" :disabled="loading" @click="loadAttachment">
        {{ loading ? '文件解密中…' : '解密文件' }}
      </button>
    </template>

    <template v-else>
      <p>{{ message.decode_error || decodeMessageContent(message.content_b64) }}</p>
    </template>

    <span class="message-kind">{{ kindLabel }}</span>
    <small v-if="error" class="media-error" role="alert">{{ error }}</small>
  </div>
</template>
