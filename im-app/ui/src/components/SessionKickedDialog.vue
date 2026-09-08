<!-- 会话被踢弹窗：当账号在其他设备登录、当前会话被强制断开时显示。 -->
<script setup lang="ts">
const emit = defineEmits<{ confirm: [] }>()

const handleConfirm = () => emit('confirm')
</script>

<template>
  <Teleport to="body">
    <div class="session-kicked-overlay" @click.self="handleConfirm">
      <div class="session-kicked-card">
        <div class="sk-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="12" cy="12" r="10" />
            <line x1="4.93" y1="4.93" x2="19.07" y2="19.07" />
          </svg>
        </div>
        <h2 class="sk-title">会话已失效</h2>
        <p class="sk-desc">
          您的账号已在其他设备登录，当前会话已被强制断开。<br />
          请重新登录？
        </p>
        <button class="sk-btn sk-btn--primary" @click="handleConfirm">重新登录</button>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
/* ── 遮罩层 ─────────────────────────────────────────────── */
.session-kicked-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
  animation: sk-fade-in 200ms ease-out both;
}

@keyframes sk-fade-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}

/* ── 卡片 ──────────────────────────────────────────────── */
.session-kicked-card {
  width: min(420px, calc(100% - 32px));
  background: var(--bg-surface);
  border: 1px solid var(--border-medium);
  border-radius: 16px;
  padding: 32px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  box-shadow: 0 24px 64px rgba(0, 0, 0, 0.45), 0 4px 16px rgba(0, 0, 0, 0.25);
  animation: sk-card-in 280ms cubic-bezier(0.16, 1, 0.3, 1) both;
}

[data-theme="light"] .session-kicked-card {
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.12), 0 2px 8px rgba(0, 0, 0, 0.07);
}

@keyframes sk-card-in {
  from {
    opacity: 0;
    transform: scale(0.92) translateY(16px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

/* ── 图标 ──────────────────────────────────────────────── */
.sk-icon {
  width: 48px;
  height: 48px;
  border-radius: 50%;
  background: rgba(248, 81, 73, 0.12);
  border: 1px solid rgba(248, 81, 73, 0.25);
  display: grid;
  place-items: center;
  flex-shrink: 0;
}

.sk-icon svg {
  width: 24px;
  height: 24px;
  color: var(--danger);
}

/* ── 文字 ──────────────────────────────────────────────── */
.sk-title {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
  color: var(--text-primary);
  letter-spacing: -0.01em;
  text-align: center;
  line-height: 1.3;
}

.sk-desc {
  margin: 0;
  font-size: 14px;
  color: var(--text-secondary);
  line-height: 1.6;
  text-align: center;
}

/* ── 按钮 ──────────────────────────────────────────────── */
.sk-btn {
  appearance: none;
  border: 1px solid transparent;
  border-radius: 8px;
  padding: 8px 24px;
  font-size: 14px;
  font-weight: 600;
  font-family: inherit;
  cursor: pointer;
  transition: background 150ms, box-shadow 150ms;
  line-height: 1.4;
  background: var(--danger);
  color: #fff;
  box-shadow: 0 2px 8px rgba(248, 81, 73, 0.3);
}

.sk-btn:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: 2px;
}

.sk-btn:hover {
  filter: brightness(1.1);
  box-shadow: 0 4px 14px rgba(248, 81, 73, 0.4);
}
</style>
