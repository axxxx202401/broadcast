<script setup lang="ts">
import { computed, ref } from 'vue'

import { useLottery } from '../composables/useLottery'
import type { DrawItem } from '../services/tauri'

const { config, drawHistory, currentIssue, loading, error, saveConfig, fetchHistory } = useLottery()

/** 是否展开配置编辑区。 */
const editing = ref(false)
/** 正在编辑的 URL 输入框。 */
const editUrl = ref(config.value.api_url)

/** 本期期号（drawHistory[0]），不存在时显示占位。 */
const currentDraw = computed<DrawItem | null>(() => drawHistory.value[0] ?? null)
/** 上期期号（drawHistory[1]），不存在时显示占位。 */
const previousDraw = computed<DrawItem | null>(() => drawHistory.value[1] ?? null)

function openEdit() {
  editUrl.value = config.value.api_url
  editing.value = true
}

async function confirmSave() {
  await saveConfig(editUrl.value.trim(), 0)
  editing.value = false
}

function cancelEdit() {
  editing.value = false
}
</script>

<template>
  <section class="lottery-panel" aria-label="开奖信息">
    <!-- 收起态：显示期号和快捷入口 -->
    <template v-if="!editing">
      <div class="lottery-header">
        <span class="lottery-label">开奖</span>
        <div class="lottery-issues">
          <div class="issue-block current">
            <span class="issue-label">本期</span>
            <strong v-if="currentDraw" class="issue-number">{{ currentDraw.pre_draw_issue }}</strong>
            <span v-else class="issue-placeholder">—</span>
            <small v-if="currentDraw" class="issue-time">{{ currentDraw.pre_draw_time }}</small>
            <small v-else class="issue-time"></small>
          </div>
          <div class="issue-divider" aria-hidden="true">|</div>
          <div class="issue-block previous">
            <span class="issue-label">上期</span>
            <strong v-if="previousDraw" class="issue-number">{{ previousDraw.pre_draw_issue }}</strong>
            <span v-else class="issue-placeholder">—</span>
            <small v-if="previousDraw" class="issue-time">{{ previousDraw.pre_draw_time }}</small>
            <small v-else class="issue-time"></small>
          </div>
        </div>
        <div class="lottery-actions">
          <button
            class="icon-button compact"
            type="button"
            title="刷新"
            :disabled="loading"
            @click="fetchHistory"
          >
            <span :class="{ spinning: loading }" aria-hidden="true">↻</span>
          </button>
          <button
            class="icon-button compact"
            type="button"
            title="修改配置"
            @click="openEdit"
          >
            <span aria-hidden="true">⚙</span>
          </button>
        </div>
      </div>
      <p v-if="error" class="lottery-error">{{ error }}</p>
    </template>

    <!-- 展开态：编辑表单 -->
    <template v-else>
      <div class="lottery-edit">
        <label class="edit-field">
          <span class="edit-label">API URL</span>
          <input
            v-model="editUrl"
            type="url"
            placeholder="https://go124.com/api/hash/get28HistoryList/10091"
          />
        </label>
        <div class="edit-actions">
          <button class="button ghost compact" type="button" @click="cancelEdit">取消</button>
          <button
            class="button primary compact"
            type="button"
            @click="confirmSave"
          >保存</button>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.lottery-panel {
  background: var(--panel-bg, var(--surface-2, #f5f5f7));
  border-bottom: 1px solid var(--border-color, #e2e2e6);
  padding: 8px 16px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.lottery-header {
  display: flex;
  align-items: center;
  gap: 10px;
}

.lottery-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary, #6e6e73);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  white-space: nowrap;
}

.lottery-issues {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
}

.issue-block {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.issue-label {
  font-size: 10px;
  color: var(--text-tertiary, #8e8e93);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.issue-number {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-primary, #1d1d1f);
  letter-spacing: 0.02em;
  line-height: 1.2;
}

.issue-block.current .issue-number {
  color: var(--accent-color, #007aff);
}

.issue-placeholder {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-tertiary, #8e8e93);
}

.issue-time {
  font-size: 11px;
  color: var(--text-tertiary, #8e8e93);
  line-height: 1.3;
}

.issue-divider {
  font-size: 14px;
  color: var(--text-tertiary, #8e8e93);
  user-select: none;
}

.lottery-actions {
  display: flex;
  gap: 4px;
}

.lottery-error {
  font-size: 12px;
  color: #ff3b30;
  margin: 0;
  padding-left: 4px;
}

/* 编辑表单 */
.lottery-edit {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.edit-field {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.edit-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary, #6e6e73);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.edit-field input {
  font-size: 13px;
  padding: 4px 8px;
  border: 1px solid var(--border-color, #e2e2e6);
  border-radius: 6px;
  background: var(--input-bg, #fff);
  color: var(--text-primary, #1d1d1f);
  outline: none;
}

.edit-field input:focus {
  border-color: var(--accent-color, #007aff);
}

.edit-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 2px;
}
</style>
