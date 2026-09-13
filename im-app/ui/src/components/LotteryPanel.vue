<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

import { useLottery } from '../composables/useLottery'
import type { DrawItem, LotteryTemplate } from '../services/tauri'
import { api } from '../services/tauri'
import { errorMessage } from '../utils/protocol'

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

// ── 广播模板 ──────────────────────────────────────────────────────────────────

const template = ref<LotteryTemplate>({ template: '', enabled: false })
const templateEditing = ref(false)
const templateEditValue = ref('')
const templateTextarea = ref<HTMLTextAreaElement | null>(null)

// 编译期运行时配置（持久化 / 匹配开关）
const runtimeConfig = ref<{ persist_received_messages: boolean; match_lottery_messages: boolean }>({
  persist_received_messages: true,
  match_lottery_messages: true,
})

// 测试消息发送
const testGroupId = ref('')
const testText = ref('')
const sendingTest = ref(false)
const testResult = ref('')
const testResultOk = ref(true)

async function sendTestMessage() {
  const groupId = testGroupId.value.trim()
  const text = testText.value.trim()
  if (!groupId || !text) {
    testResult.value = '请填写群 ID 和消息内容'
    testResultOk.value = false
    return
  }
  sendingTest.value = true
  testResult.value = '发送成功，等待服务器回执 (2201)'
  try {
    const serverMsgId = await api.sendTestGroupMessage(parseInt(groupId, 10), text)
    testResult.value = `服务器已确认送达 (2201)，消息 ID: ${serverMsgId}`
    testResultOk.value = true
  } catch (e) {
    testResult.value = `发送失败: ${errorMessage(e)}`
    testResultOk.value = false
  } finally {
    sendingTest.value = false
  }
}

async function loadRuntimeConfig() {
  try {
    runtimeConfig.value = await api.getAppRuntimeConfig()
  } catch (e) {
    console.error('Failed to load runtime config:', errorMessage(e))
  }
}

async function loadTemplate() {
  try {
    template.value = await api.getLotteryTemplate()
    templateEditValue.value = template.value.template
  } catch (e) {
    console.error('Failed to load template:', errorMessage(e))
  }
}

async function saveTemplate() {
  try {
    await api.setLotteryTemplate(templateEditValue.value, template.value.enabled)
    await loadTemplate()
    templateEditing.value = false
  } catch (e) {
    console.error('Failed to save template:', errorMessage(e))
  }
}

/** 打开独立模板编辑对话框，并把焦点移入正文输入区。 */
async function openTemplateEdit() {
  templateEditValue.value = template.value.template
  templateEditing.value = true
  await nextTick()
  templateTextarea.value?.focus()
}

function cancelTemplateEdit() {
  templateEditValue.value = template.value.template
  templateEditing.value = false
}

/** Escape 仅关闭模板对话框，不影响页面上的其他编辑状态。 */
function handleTemplateDialogKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && templateEditing.value) {
    cancelTemplateEdit()
  }
}

async function toggleBroadcastEnabled(enabled: boolean) {
  template.value.enabled = enabled
  try {
    await api.setLotteryTemplate(template.value.template, enabled)
  } catch (e) {
    console.error('Failed to save template:', errorMessage(e))
    template.value.enabled = !enabled
  }
}

// 挂载时加载
onMounted(() => {
  void loadTemplate()
  void loadRuntimeConfig()
  window.addEventListener('keydown', handleTemplateDialogKeydown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleTemplateDialogKeydown)
})
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
    <div class="lottery-row">
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

  <!-- 广播控制面板 -->
  <div class="broadcast-section">
    <div class="broadcast-header">
      <!-- <span class="section-title">广播设置</span> -->
      <label class="toggle-label">
        <input
          class="toggle-input"
          data-test="lottery-broadcast-toggle"
          type="checkbox"
          role="switch"
          aria-label="自动广播"
          :checked="template.enabled"
          @change="toggleBroadcastEnabled(($event.target as HTMLInputElement)?.checked ?? false)"
        />
        <span class="toggle-slider" aria-hidden="true"></span>
        <span>自动广播</span>
      </label>
    </div>

    <button
      class="btn-ghost btn-sm"
      data-test="edit-lottery-template"
      type="button"
      @click="openTemplateEdit"
    >
      编辑模板
    </button>

    <!-- 测试发送 -->
    <!-- <div class="test-send-section">
      <div class="test-send-row">
        <input
          v-model="testGroupId"
          class="test-input"
          type="text"
          placeholder="群 ID"
          style="width: 140px"
        />
        <input
          v-model="testText"
          class="test-input"
          type="text"
          placeholder="测试消息内容"
          style="flex: 1"
        />
        <button
          class="btn-primary btn-sm"
          type="button"
          :disabled="sendingTest"
          @click="sendTestMessage"
        >
          {{ sendingTest ? '发送中...' : '发送测试消息' }}
        </button>
      </div>
      <span v-if="testResult" :class="['test-result', testResultOk ? 'ok' : 'err']">
        {{ testResult }}
      </span>
    </div> -->

    <!-- 环境配置（编译期常量，仅展示） -->
    <!-- <div class="env-config">
      <div class="config-row">
        <span class="config-label">消息入库</span>
        <span class="config-value">{{ runtimeConfig.persist_received_messages ? '是' : '否' }}</span>
        <span class="config-note">（编译期配置）</span>
      </div>
      <div class="config-row">
        <span class="config-label">消息匹配</span>
        <span class="config-value">{{ runtimeConfig.match_lottery_messages ? '是' : '否' }}</span>
        <span class="config-note">（编译期配置）</span>
      </div>
    </div> -->
  </div>

  <!-- 长模板使用独立对话框编辑，避免受消息标题栏宽度挤压。 -->
  <Teleport to="body">
    <div
      v-if="templateEditing"
      class="template-dialog-backdrop"
      data-test="lottery-template-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="lottery-template-dialog-title"
      @click.self="cancelTemplateEdit"
    >
      <section class="template-dialog-card">
        <header class="template-dialog-header">
          <div>
            <h2 id="lottery-template-dialog-title">编辑广播模板</h2>
            <p>保留占位符，开奖后会自动替换为实际内容。</p>
          </div>
          <button class="template-dialog-close" type="button" aria-label="关闭模板编辑器" @click="cancelTemplateEdit">×</button>
        </header>
        <textarea
          ref="templateTextarea"
          v-model="templateEditValue"
          class="template-textarea"
          data-test="lottery-template-textarea"
          aria-label="广播模板内容"
          spellcheck="false"
        ></textarea>
        <p class="template-placeholder-help">
          可用：${preDrawIssue}、${preDrawCode}、${sumNum}、${sumBigSmall}、${sumSingleDouble}、${patternDesc}、${lastTenDraws}
        </p>
        <footer class="template-dialog-actions">
          <button class="btn-ghost" type="button" @click="cancelTemplateEdit">取消</button>
          <button class="btn-primary" type="button" @click="saveTemplate">保存模板</button>
        </footer>
      </section>
    </div>
  </Teleport>
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

/* 广播面板 */
.broadcast-section {
  border-top: 1px solid var(--border-subtle);
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.broadcast-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.section-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.toggle-label {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-secondary);
  cursor: pointer;
  user-select: none;
}

.toggle-input {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
}

.toggle-slider {
  position: relative;
  width: 36px;
  height: 20px;
  flex: 0 0 auto;
  border: 1px solid var(--border-medium);
  border-radius: 999px;
  background: var(--bg-elevated-2);
  transition: border-color 160ms ease, background 160ms ease, box-shadow 160ms ease;
}

.toggle-slider::after {
  content: "";
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--text-tertiary);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  transition: transform 160ms ease, background 160ms ease;
}

.toggle-input:checked + .toggle-slider {
  border-color: var(--success);
  background: color-mix(in srgb, var(--success) 24%, var(--bg-elevated));
}

.toggle-input:checked + .toggle-slider::after {
  background: var(--success);
  transform: translateX(16px);
}

.toggle-input:focus-visible + .toggle-slider {
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 24%, transparent);
}

.toggle-label:hover .toggle-slider {
  border-color: var(--text-tertiary);
}

.template-textarea {
  width: 100%;
  font-family: "IBM Plex Mono", monospace;
  min-height: 420px;
  font-size: 13px;
  line-height: 1.65;
  padding: 14px 16px;
  border: 1px solid var(--border-medium);
  border-radius: 8px;
  background: var(--bg-surface);
  color: var(--text-primary);
  resize: vertical;
  outline: none;
  box-sizing: border-box;
}

.template-textarea:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(240, 180, 70, 0.12);
}

.template-dialog-backdrop {
  position: fixed;
  z-index: 1000;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(8, 10, 14, 0.72);
  backdrop-filter: blur(4px);
}

.template-dialog-card {
  width: min(720px, calc(100vw - 32px));
  max-height: calc(100vh - 48px);
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px;
  overflow: auto;
  border: 1px solid var(--border-medium);
  border-radius: 14px;
  background: var(--bg-elevated);
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.42);
}

.template-dialog-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.template-dialog-header h2 {
  margin: 0;
  color: var(--text-primary);
  font-size: 17px;
}

.template-dialog-header p,
.template-placeholder-help {
  margin: 4px 0 0;
  color: var(--text-tertiary);
  font-size: 11px;
}

.template-placeholder-help {
  overflow-wrap: anywhere;
  font-family: "IBM Plex Mono", monospace;
}

.template-dialog-close {
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font-size: 24px;
  line-height: 1;
  cursor: pointer;
}

.template-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

@media (max-width: 640px) {
  .template-dialog-backdrop {
    align-items: stretch;
    padding: 8px;
  }

  .template-dialog-card {
    width: 100%;
    max-height: none;
    padding: 16px;
    border-radius: 10px;
  }

  .template-textarea {
    min-height: 0;
    flex: 1;
  }
}

.btn-icon {
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--text-secondary);
  font-size: 12px;
  padding: 2px 4px;
  border-radius: 3px;
  transition: background 0.15s, color 0.15s;
}

.btn-icon:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}

.btn-icon:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.test-send-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-top: 4px;
  border-top: 1px solid var(--border-subtle);
}

.test-send-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.test-input {
  font-size: 11px;
  padding: 3px 8px;
  border: 1px solid var(--border-medium);
  border-radius: var(--radius);
  background: var(--bg-surface);
  color: var(--text-primary);
  outline: none;
  font-family: "IBM Plex Mono", monospace;
}

.test-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(240, 180, 70, 0.12);
}

.test-result {
  font-size: 10px;
  padding: 2px 0;
}

.test-result.ok {
  color: var(--success);
}

.test-result.err {
  color: var(--danger);
}

.env-config {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-top: 4px;
  border-top: 1px solid var(--border-subtle);
}

.config-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
}

.config-label {
  color: var(--text-tertiary);
  width: 60px;
  flex-shrink: 0;
}

.config-value {
  color: var(--text-secondary);
  font-weight: 500;
}

.config-note {
  color: var(--text-tertiary);
  font-size: 10px;
  font-style: italic;
}
</style>
