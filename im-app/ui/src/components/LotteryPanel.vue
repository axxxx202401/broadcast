<script setup lang="ts">
import { computed, ref } from 'vue'

import { useLottery } from '../composables/useLottery'
import type { DrawItem } from '../services/tauri'

const props = withDefaults(defineProps<{
  lottery?: {
    config: { value: { api_url: string; current_issues: number[] } }
    drawHistory: { value: DrawItem[] }
    loading: { value: boolean }
    error: { value: string }
    loadConfig: () => Promise<void>
    saveConfig: (url: string, issues: number[]) => Promise<void>
    fetchHistory: () => Promise<void>
  }
}>(), {})

const src = props.lottery ?? useLottery()

// 解包 prop 中的 ref，使模板可直接使用（与独立调用 useLottery() 行为一致）。
const config = computed(() => src.config.value)

const drawHistory = computed(() => src.drawHistory.value)
const loading = computed(() => src.loading.value)
const error = computed(() => src.error.value)
const loadConfig = src.loadConfig
const saveConfig = src.saveConfig
const fetchHistory = src.fetchHistory

/** 是否展开配置编辑区。 */
const editing = ref(false)
const editUrl = ref('')

const currentDraw = computed<DrawItem | null>(() => drawHistory.value[0] ?? null)
const previousDraw = computed<DrawItem | null>(() => drawHistory.value[1] ?? null)

async function openEdit() {
    await loadConfig()
    editUrl.value = config.value?.api_url ?? ''
    editing.value = true
  }

async function confirmSave() {
  // 保存所有历史期号，用于消息匹配。
  const issues = drawHistory.value.map(item => item.preDrawIssue)
  await saveConfig(editUrl.value.trim(), issues)
  editing.value = false
}

function cancelEdit() {
  editing.value = false
}
</script>

<template>
  <!-- 内嵌模式：紧凑竖排，与消息区标题栏融为一条 -->
  <div v-if="!editing" class="lottery-strip" role="status" aria-label="开奖信息">
    <div class="lottery-row">
      <span class="issue">
        <em class="issue-since">本期期号</em>
        <strong class="issue-num">{{ currentDraw?.preDrawIssue ?? '—' }}</strong>
        <!-- <span class="issue-time">{{ currentDraw?.preDrawTime ?? '' }}</span> -->
      </span>
      <button
        class="lottery-btn"
        type="button"
        title="刷新"
        :disabled="loading"
        @click="fetchHistory"
      >
        <span :class="{ spinning: loading }" aria-hidden="true">↻</span>
      </button>
    </div>
    <div v-if="previousDraw" class="lottery-row">
      <span class="issue">
        <em class="issue-since">上期期号</em>
        <strong class="issue-num">{{ previousDraw?.preDrawIssue ?? '—' }}</strong>
        <!-- <span class="issue-time">{{ previousDraw?.preDrawTime ?? '' }}</span> -->
      </span>
      <button class="lottery-btn" type="button" title="配置 API" @click="openEdit">
        <span aria-hidden="true">⚙</span>
      </button>
    </div>
    <span v-if="error" class="lottery-err">{{ error }}</span>
  </div>

  <!-- 编辑表单 -->
  <div v-else class="lottery-edit">
    <label class="edit-row">
      <span class="edit-label">API URL</span>
      <input v-model="editUrl" type="url" />
    </label>
    <div class="edit-actions">
      <button class="btn-ghost" type="button" @click="cancelEdit">取消</button>
      <button class="btn-primary" type="button" @click="confirmSave">保存</button>
    </div>
  </div>
</template>

<style scoped>
.lottery-strip {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 11px;
  color: var(--text-secondary);
}

.lottery-row {
  display: flex;
  align-items: center;
  gap: 4px;
}

.issue {
  display: inline-flex;
  align-items: baseline;
  gap: 3px;
}

.issue-since {
  font-style: normal;
  font-size: 12px;
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.issue-num {
  font-family: "IBM Plex Mono", monospace;
  font-size: 12px;
  font-weight: 700;
  color: var(--success);
}

.issue-time {
  font-size: 10px;
  color: var(--text-tertiary);
}

.lottery-err {
  color: var(--danger);
  font-size: 10px;
}

.lottery-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
  flex: 0 0 auto;
  margin-left: auto;
  transition: background 0.15s, color 0.15s;
}

.lottery-btn:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}

.lottery-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 编辑表单 */
.lottery-edit {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px 12px;
  background: var(--bg-elevated);
  border-top: 1px solid var(--border-subtle);
}

.edit-row {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.edit-label {
  font-size: 10px;
  font-weight: 600;
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.edit-row input {
  font-size: 12px;
  padding: 3px 8px;
  border: 1px solid var(--border-medium);
  border-radius: var(--radius);
  background: var(--bg-surface);
  color: var(--text-primary);
  outline: none;
  font-family: "IBM Plex Mono", monospace;
  transition: border-color 180ms ease, box-shadow 180ms ease;
}

.edit-row input::placeholder {
  color: var(--text-tertiary);
}

.edit-row input:hover {
  border-color: var(--border-subtle);
}

.edit-row input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(240, 180, 70, 0.12);
}

[data-theme="light"] .edit-row input:focus {
  box-shadow: 0 0 0 3px rgba(196, 154, 47, 0.2);
}

.edit-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.btn-ghost,
.btn-primary {
  font-size: 11px;
  padding: 3px 10px;
  border-radius: var(--radius);
  border: none;
  cursor: pointer;
  font-weight: 500;
  transition: background 0.15s;
}

.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
}

.btn-ghost:hover {
  background: var(--bg-elevated-2);
  color: var(--text-primary);
}

.btn-primary {
  background: var(--accent);
  color: #18140c;
  font-weight: 600;
}

.btn-primary:hover {
  background: var(--accent-soft);
}
</style>
