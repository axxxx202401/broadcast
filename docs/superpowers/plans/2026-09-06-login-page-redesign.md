# Login Page Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the login page with a split-screen layout (brand left, form right), enhanced dark/light themes, and micro-interactions — while keeping all 213 existing tests passing.

**Architecture:** The login card is restructured from a single-column 560px box into a two-panel grid (45% brand + 55% form) on wide screens, collapsing to single-column below 900px. Theme tokens in `base.css` are updated first; CSS in `console.css` is rewritten; Vue template gains sliding tab indicator, eye toggle, and staggered entry animation. All `data-test` attributes and test-critical class names are preserved.

**Tech Stack:** Vue 3 (Composition API), Vite, Vitest (jsdom), scoped CSS, CSS custom properties, no new dependencies.

**Spec:** [docs/superpowers/specs/2026-09-06-login-page-redesign.md](../specs/2026-09-06-login-page-redesign.md)

## Global Constraints

- All 213 existing tests must pass after every task; `npm test` in `im-app/ui/` is the gate.
- The 17 CSS class names listed in spec section 6 must be preserved verbatim (`.login-method-tab`, `.login-submit`, `.account-row`, `.is-phone`, `.account-cell`, `.secret-field`, `.is-code`, `.field-control`, `.password-sentinel`, `.is-visible`, `.challenge-step`, `.login-back`, `.login-logo`, `.purpose`, `.login-primary-panel`, `.login-form`, `.login-form-fields`, `.code-input-wrap`, `.country-code-cell`, `.pending-list`).
- All `data-test` attributes must be preserved verbatim.
- ARIA attributes (`role="tablist"`, `aria-selected`, `aria-label`) must be preserved.
- Dark accent stays `#f0b446`; light accent changes from `#b08800` to `#c49a2f`.
- `--radius` changes from `6px` to `10px` globally.
- `getBoundingClientRect` for tab indicator must guard against null in jsdom (test environment returns zero-sized elements).

---

### Task 1: Update CSS theme tokens in base.css

**Files:**
- Modify: `im-app/ui/src/styles/base.css`

**Interfaces:**
- Consumes: current CSS variable values (see spec section 3.2 tables)
- Produces: updated `--bg-*`, `--border-*`, `--text-*`, `--accent-*`, `--focus-ring`, `--radius` tokens

- [ ] **Step 1: Update dark theme tokens**

In `base.css`, replace the dark theme block (lines 2–32) with:

```css
:root {
  color-scheme: dark;
  font-family: "IBM Plex Sans", "PingFang SC", "Noto Sans SC",
               "Helvetica Neue", "Segoe UI", system-ui, sans-serif;
  font-synthesis: none;

  /* 暗色主题 — 画布到表面的四层递进 */
  --bg-canvas:      #0a0e14;
  --bg-surface:     #1a1f28;
  --bg-elevated:    #212732;
  --bg-elevated-2:  #262d3a;

  /* 边框层次 */
  --border-subtle:  #2d3340;
  --border-medium:  #383f4d;

  /* 文字层次 */
  --text-primary:   #e6edf3;
  --text-secondary: #848d9a;
  --text-tertiary:  #636c76;

  /* 语义色 */
  --accent:       #f0b446;
  --accent-soft:  #d4922e;
  --success:      #3fb950;
  --info:         #58a6ff;
  --danger:       #f85149;
  --focus-ring:   #d4922e;

  --radius: 10px;
}
```

Also update the body grid line opacity from `0.025` to `0.035`:

```css
  background:
    linear-gradient(rgba(255, 255, 255, 0.035) 1px, transparent 1px),
    linear-gradient(90deg, rgba(255, 255, 255, 0.035) 1px, transparent 1px),
    var(--bg-canvas);
```

- [ ] **Step 2: Update light theme tokens**

Replace the `[data-theme="light"]` block (lines 35–60) with:

```css
/* ===== 亮色主题 ===== */
[data-theme="light"] {
  color-scheme: light;

  /* 亮色主题 — 柔和蓝灰画布 + 纯白卡片 */
  --bg-canvas:      #f0f2f5;
  --bg-surface:     #ffffff;
  --bg-elevated:    #e8ebf0;
  --bg-elevated-2:  #e0e4ea;

  /* 边框层次 */
  --border-subtle:  #cdd3da;
  --border-medium:  #b8c0ca;

  /* 文字层次 */
  --text-primary:   #1f2328;
  --text-secondary: #656d76;
  --text-tertiary:  #858e99;

  /* 语义色 — 与暗色同色系，明度提升 */
  --accent:       #c49a2f;
  --accent-soft:  #a8852a;
  --success:      #1a7f37;
  --info:         #0969da;
  --danger:       #cf222e;
  --focus-ring:   #c49a2f;
}
```

Update light body grid line opacity from `0.04` to `0.06`:

```css
/* 亮色：极淡网格，与背景融合 */
[data-theme="light"] body {
  background:
    linear-gradient(rgba(0, 0, 0, 0.06) 1px, transparent 1px),
    linear-gradient(90deg, rgba(0, 0, 0, 0.06) 1px, transparent 1px),
    var(--bg-canvas);
}
```

- [ ] **Step 3: Run build to verify no CSS errors**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vite build --mode development
```
Expected: ✓ built successfully, no errors.

- [ ] **Step 4: Run tests to verify nothing breaks**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vitest run
```
Expected: 213 tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/styles/base.css
git commit -m "style(base): enhance dark/light theme tokens and radius for login redesign"
```

---

### Task 2: Rewrite login layout CSS in console.css

**Files:**
- Modify: `im-app/ui/src/styles/console.css`

**What to replace:** Lines 1–299 (the entire `.login-shell` through `.account-picker select` block), plus the responsive block at lines 1671–1715, plus the light-theme overrides at lines 1998–2029.

**Interfaces:**
- Consumes: new CSS tokens from Task 1 (`--bg-canvas`, `--bg-surface`, `--accent`, `--radius`, etc.)
- Produces: new dual-panel login layout styles; preserves all 20+ test-critical class names

- [ ] **Step 1: Replace the login layout block (lines 1–299)**

Replace everything from the top of `console.css` through the end of `.account-picker select { width: 100%; }` (approximately lines 1–299) with the following. All original class names are preserved; new classes `.login-brand`, `.login-form-panel`, `.login-tab-indicator`, `.eye-toggle`, `.field-group` are added.

```css
/* 登录页：宽屏双栏分割，窄屏单列。login-card 为双栏 grid，左品牌右表单。 */
.login-shell {
  display: grid;
  grid-template-columns: minmax(0, 45%) minmax(0, 55%);
  width: 900px;
  max-width: calc(100% - 48px);
  min-height: calc(100% - 80px);
  margin: 40px auto;
  border-radius: 16px;
  overflow: hidden;
  box-shadow: 0 32px 80px rgba(0, 0, 0, 0.4);
  animation: console-enter 380ms cubic-bezier(0.4, 0, 0.2, 1) both;
}

.login-card {
  display: grid;
  grid-template-columns: minmax(0, 45%) minmax(0, 55%);
  width: 100%;
  min-width: 0;
  box-sizing: border-box;
  max-height: calc(100vh - 80px);
  background: var(--bg-surface);
  scrollbar-gutter: stable;
}

/* 左栏：品牌区，渐变背景，居中展示 logo + slogan。 */
.login-brand {
  position: relative;
  overflow: hidden;
  background: linear-gradient(160deg, var(--accent) 0%, var(--accent-soft) 100%);
  display: grid;
  place-content: center;
  padding: 64px 48px;
  animation: console-enter 380ms cubic-bezier(0.4, 0, 0.2, 1) both;
}

/* 装饰性背景纹理：同心圆环 */
.login-brand::before {
  content: "";
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 30% 40%, rgba(255,255,255,0.08) 0%, transparent 60%),
    radial-gradient(circle at 70% 80%, rgba(0,0,0,0.12) 0%, transparent 50%);
  pointer-events: none;
}

.login-brand .login-logo {
  width: 64px;
  height: 64px;
  filter: drop-shadow(0 2px 8px rgba(0,0,0,0.2));
  margin-bottom: 16px;
}

.login-brand .purpose {
  margin: 0;
  color: rgba(255, 255, 255, 0.88);
  font-size: 14px;
  font-weight: 500;
  letter-spacing: 0.02em;
  text-align: center;
  line-height: 1.6;
}

/* 右栏：表单区，可滚动。 */
.login-form-panel {
  display: flex;
  flex-direction: column;
  padding: 48px;
  overflow-y: auto;
  animation: console-enter 380ms cubic-bezier(0.4, 0, 0.2, 1) 80ms both;
}

/* 亮色右栏加轻微内阴影增加材质感 */
[data-theme="light"] .login-form-panel {
  box-shadow: inset 1px 0 0 rgba(0,0,0,0.04);
}

.login-primary-panel {
  display: grid;
  grid-template-rows: auto 1fr;
  gap: 16px;
  flex: 1;
  min-height: 0;
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
  flex: 1;
  min-height: 0;
}

.account-picker {
  display: grid;
  gap: 8px;
}

.account-picker label {
  display: grid;
  gap: 6px;
}

/* 主字段区：flex 列。 */
.login-form-fields {
  display: flex;
  flex-direction: column;
  gap: 14px;
  flex: 1;
  min-height: 0;
}

.login-form-fields .account-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 6px;
  align-items: end;
}

.login-form-fields .account-row.is-phone {
  grid-template-columns: 110px minmax(0, 1fr);
}

/* Floating label 包装层：input 和 label 为兄弟元素。 */
.field-group {
  position: relative;
  display: grid;
  gap: 0;
}

.field-group label {
  position: absolute;
  top: 50%;
  left: 12px;
  transform: translateY(-50%);
  color: var(--text-tertiary);
  font-family: inherit;
  font-size: 13px;
  font-weight: 500;
  letter-spacing: 0;
  text-transform: none;
  line-height: 1.4;
  pointer-events: none;
  transition: all 200ms cubic-bezier(0.4, 0, 0.2, 1);
  margin: 0;
}

.field-group input:focus ~ label,
.field-group input:not(:placeholder-shown) ~ label {
  top: 0;
  left: 10px;
  transform: translateY(-50%);
  font-size: 11px;
  color: var(--accent);
  background: var(--bg-surface);
  padding: 0 4px;
}

/* 国家区号 cell 不应用 floating label（固定位置 label）。 */
.field-group.country-code-group label {
  position: static;
  transform: none;
  font-size: 12px;
  margin-bottom: 6px;
}

.field-group.country-code-group input:focus ~ label,
.field-group.country-code-group input:not(:placeholder-shown) ~ label {
  top: 50%;
  left: 12px;
  transform: translateY(-50%);
  font-size: 13px;
  color: var(--text-tertiary);
  background: transparent;
  padding: 0;
}

/* 秘密字段：label + 输入行。密码模式下使用 floating label。 */
.login-form-fields .secret-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

/* 已保存密码哨兵：内联在 label 行右侧。 */
.password-sentinel {
  display: none;
  color: var(--accent);
  font-size: 11px;
  line-height: 1;
  font-weight: 500;
}

.password-sentinel.is-visible {
  display: inline;
}

/* 所有登录表单 input：统一 44px 高、12px 内边距、基础边框。 */
.login-form-fields input,
.field-group input {
  height: 44px;
  padding: 10px 12px;
  width: 100%;
  box-sizing: border-box;
  border: 1px solid var(--border-medium);
  border-radius: var(--radius);
  background: var(--bg-elevated);
  color: var(--text-primary);
  font-size: 14px;
  transition: border-color 180ms ease, box-shadow 180ms ease, background 180ms ease;
}

.login-form-fields input:hover,
.field-group input:hover {
  border-color: var(--border-subtle);
  background: var(--bg-elevated-2);
}

.login-form-fields input:focus,
.field-group input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(240, 180, 70, 0.15);
  background: var(--bg-surface);
}

/* 亮色 focus ring 用金色 */
[data-theme="light"] .login-form-fields input:focus,
[data-theme="light"] .field-group input:focus {
  box-shadow: 0 0 0 3px rgba(196, 154, 47, 0.2);
}

/* 验证码模式 input 右侧留白给发送按钮。 */
.login-form-fields .secret-field.is-code input,
.secret-field.is-code .field-group input {
  padding-right: 100px;
}

.code-input-wrap {
  position: relative;
  display: flex;
  align-items: center;
  width: 100%;
}

/* 发送验证码按钮：嵌在输入框右侧。 */
.code-send-inline {
  position: absolute;
  right: 4px;
  flex: 0 0 auto;
  flex-shrink: 0;
  height: 32px;
  padding: 0 10px;
  border-radius: calc(var(--radius) - 4px);
  border: 1px solid var(--border-subtle);
  white-space: nowrap;
  font-size: 11px;
  font-weight: 600;
}

/* 密码可见性切换按钮。 */
.eye-toggle {
  position: absolute;
  right: 4px;
  top: 6px;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: calc(var(--radius) - 4px);
  background: transparent;
  color: var(--text-tertiary);
  cursor: pointer;
  display: grid;
  place-items: center;
  transition: color 150ms ease, background 150ms ease;
  padding: 0;
}

.eye-toggle:hover {
  color: var(--text-primary);
  background: var(--bg-elevated-2);
}

.eye-toggle svg {
  width: 16px;
  height: 16px;
}

/* Tab 栏：flex + sliding indicator。 */
.login-method-tabs {
  position: relative;
  display: flex;
  gap: 0;
  height: 44px;
  background: var(--bg-elevated);
  border: 1px solid var(--border-medium);
  border-radius: var(--radius);
  padding: 3px;
  overflow: hidden;
}

.login-tab-indicator {
  position: absolute;
  top: 3px;
  height: calc(100% - 6px);
  background: var(--bg-surface);
  border: 1px solid var(--border-medium);
  border-radius: calc(var(--radius) - 3px);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.12);
  transition: left 220ms cubic-bezier(0.4, 0, 0.2, 1), width 220ms cubic-bezier(0.4, 0, 0.2, 1);
  pointer-events: none;
  z-index: 0;
}

.login-header {
  display: grid;
  justify-items: center;
  gap: 12px;
  text-align: center;
}

.login-logo {
  width: 56px;
  height: 56px;
}

.purpose {
  margin: 0;
  color: var(--text-tertiary);
  font-size: 14px;
  line-height: 1.6;
}

.login-method-tab {
  position: relative;
  z-index: 1;
  min-width: 0;
  height: 38px;
  padding: 0 4px;
  border: 0;
  border-radius: calc(var(--radius) - 3px);
  background: transparent;
  color: var(--text-tertiary);
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  transition: color 180ms ease;
  flex: 1;
}

.login-method-tab:hover:not(.is-active) {
  color: var(--text-secondary);
}

.login-method-tab.is-active {
  color: var(--text-primary);
}

.login-method-tab:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -2px;
}

.login-form-fields .account-row input,
.login-form-fields .account-row select,
.field-group input,
.field-group select {
  width: 100%;
  box-sizing: border-box;
}

.login-back {
  justify-self: start;
  align-self: flex-start;
}

.challenge-step {
  display: grid;
  gap: 12px;
}

.challenge-step > label {
  display: grid;
  gap: 6px;
}

.challenge-step > h3,
.challenge-progress,
.challenge-method,
.challenge-target {
  margin: 0;
}

.challenge-progress,
.challenge-target {
  color: var(--text-tertiary);
  font-size: 13px;
}

.pending-list {
  display: grid;
  gap: 8px;
}

.account-picker select {
  width: 100%;
}
```

- [ ] **Step 2: Update responsive rules (lines 1671–1715)**

Replace the `@media (max-width: 900px)` block for login styles with:

```css
@media (max-width: 900px) {
  .login-shell {
    grid-template-columns: 1fr;
    width: calc(100% - 24px);
    min-height: calc(100% - 24px);
    margin: 12px;
    border-radius: var(--radius);
  }

  .login-card {
    grid-template-columns: 1fr;
    max-height: calc(100vh - 24px);
  }

  .login-brand {
    display: none;
  }

  .login-form-panel {
    padding: 32px 24px;
  }
}
```

- [ ] **Step 3: Update light-theme login overrides**

Replace lines 1998–2029 (the `[data-theme="light"] .login-card` through `[data-theme="light"] .login-console` blocks) with:

```css
/* 登录表单区（亮色） */
[data-theme="light"] .login-form-panel {
  background: var(--bg-surface);
}

/* 左栏品牌区（亮色）：渐变保持金色系 */
[data-theme="light"] .login-brand {
  background: linear-gradient(160deg, #c49a2f 0%, #a8852a 100%);
}

/* 登录 tab 栏（亮色） */
[data-theme="light"] .login-method-tabs {
  background: var(--bg-elevated-2);
  border-color: var(--border-subtle);
}

[data-theme="light"] .login-tab-indicator {
  background: var(--bg-surface);
  border-color: var(--border-subtle);
}
```

- [ ] **Step 4: Run build to verify**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vite build --mode development
```
Expected: ✓ built successfully.

- [ ] **Step 5: Run tests**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vitest run
```
Expected: 213 tests pass. If any fail, check that class names `.login-method-tab`, `.login-submit`, `.account-row`, `.secret-field`, `.field-control`, `.password-sentinel`, `.challenge-step`, `.login-back`, `.login-primary-panel` are all still present in the CSS.

- [ ] **Step 6: Commit**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/styles/console.css
git commit -m "style(console): rewrite login layout as dual-panel with floating labels and tab slider"
```

---

### Task 3: Update LoginPanel.vue template and script

**Files:**
- Modify: `im-app/ui/src/components/LoginPanel.vue`

**Interfaces:**
- Consumes: `useAuth` composable (unchanged), existing props/emits (unchanged)
- Produces: new template with `.login-brand` + `.login-form-panel` wrappers, sliding tab indicator, eye toggle, floating label structure

- [ ] **Step 1: Add `activeTabEl` ref and indicator update function to script**

After the existing imports and before `const emit`, add:

```typescript
/** 滑动 tab indicator 位置跟踪。jsdom 中 getBoundingClientRect 返回零，需防御。 */
const activeTabEl = ref<HTMLElement | null>(null)
const tabIndicator = ref<HTMLElement | null>(null)

function updateTabIndicator() {
  if (!activeTabEl.value || !tabIndicator.value) return
  const tabRect = activeTabEl.value.getBoundingClientRect()
  const containerRect = activeTabEl.value.parentElement!.getBoundingClientRect()
  const left = tabRect.left - containerRect.left
  const width = tabRect.width
  tabIndicator.value.style.left = `${left}px`
  tabIndicator.value.style.width = `${width}px`
}
```

Also add `showPassword` ref (only for primary password mode, not challenge):

```typescript
const showPassword = ref(false)
```

Watch login method changes to reset `showPassword`:

```typescript
watch(() => props.auth.loginMethod.value, () => {
  showPassword.value = false
})
```

- [ ] **Step 2: Update template — add brand panel and form-panel wrappers**

Replace the template section starting from `<main class="login-shell">` with this structure. Keep all existing class names and `data-test` attributes intact.

```vue
<template>
  <main class="login-shell">
    <!-- 左栏品牌区：宽屏显示，窄屏隐藏 -->
    <div class="login-brand" aria-hidden="true">
      <img src="/icon.svg" alt="" class="login-logo" />
      <p class="purpose">进入实时监控控制台</p>
    </div>

    <!-- 右栏表单区 -->
    <section class="login-form-panel" aria-label="登录">
      <button
        v-if="props.canReturn"
        class="button ghost login-back"
        data-test="login-back"
        type="button"
        @click="emit('back')"
      >
        返回
      </button>

      <!-- 主登录：账号选择、邮箱或手机、密码哨兵、提交。 -->
      <template v-if="!auth.challengePending.value.length">
        <div v-if="props.accounts.length" class="account-picker">
          <label>
            <span>已保存账号</span>
            <select :value="auth.selectedAccountUid.value ?? ''" @change="onAccountChange">
              <option value="">添加账号</option>
              <option v-for="acc in props.accounts" :key="acc.uid" :value="acc.uid">
                {{ acc.displayAccount }}
              </option>
            </select>
          </label>
        </div>

        <div class="login-primary-panel">
          <div
            class="login-method-tabs"
            role="tablist"
            aria-label="登录方式"
          >
            <div class="login-tab-indicator" ref="tabIndicator" aria-hidden="true"></div>
            <button
              v-for="(tab, index) in loginMethodTabs"
              :key="tab.method"
              type="button"
              role="tab"
              class="login-method-tab"
              data-test="login-method-tab"
              :class="{ 'is-active': auth.loginMethod.value === tab.method }"
              :aria-selected="auth.loginMethod.value === tab.method"
              :ref="(el: unknown) => { if (el && typeof el === 'object') { const arr = (activeTabEl as any)._tabs || ((activeTabEl as any)._tabs = []); arr[index] = el as HTMLElement; updateTabIndicator() } }"
              @click="() => { chooseLoginMethod(tab.method); updateTabIndicator() }"
            >
              {{ tab.label }}
            </button>
          </div>

          <form class="login-form" @submit.prevent="auth.submitLogin">
            <div class="login-form-fields">
              <div class="account-row" :class="{ 'is-phone': isPhoneMethod }">
                <label v-if="isPhoneMethod" class="country-code-cell field-group country-code-group">
                  <span>国家区号</span>
                  <input
                    v-model.number="auth.countryCode.value"
                    type="text"
                    inputmode="numeric"
                    pattern="[0-9]*"
                    autocomplete="tel-country-code"
                  />
                </label>
                <div v-else class="field-group account-cell">
                  <input
                    v-model.trim="auth.account.value"
                    :type="isPhoneMethod ? 'tel' : 'email'"
                    :autocomplete="isPhoneMethod ? 'tel' : 'email'"
                    :placeholder="isPhoneMethod ? '输入手机号' : '输入邮箱地址'"
                    required
                  />
                  <label>{{ isPhoneMethod ? '手机号' : '邮箱地址' }}</label>
                </div>
              </div>

              <div class="secret-field" :class="{ 'is-code': auth.isCodeMode.value }">
                <div v-if="!auth.isCodeMode.value" class="field-group">
                  <input
                    v-model.trim="auth.validateValue.value"
                    :type="showPassword ? 'text' : 'password'"
                    :inputmode="auth.isCodeMode.value ? 'numeric' : 'text'"
                    :autocomplete="auth.isCodeMode.value ? 'one-time-code' : 'current-password'"
                    :required="auth.isCodeMode.value || auth.passwordMode.value !== 'saved'"
                    :placeholder="auth.passwordMode.value === 'saved' ? '••••••••' : '输入登录密码'"
                  />
                  <label>
                    登录密码
                    <span
                      class="password-sentinel"
                      :class="{ 'is-visible': auth.passwordMode.value === 'saved' }"
                      aria-hidden="true"
                    >已保存密码</span>
                  </label>
                  <button
                    v-if="auth.passwordMode.value !== 'saved'"
                    type="button"
                    class="eye-toggle"
                    aria-label="切换密码可见性"
                    @click="showPassword = !showPassword"
                  >
                    <!-- Eye open icon -->
                    <svg v-if="!showPassword" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/>
                      <circle cx="12" cy="12" r="3"/>
                    </svg>
                    <!-- Eye closed icon -->
                    <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/>
                      <line x1="1" y1="1" x2="23" y2="23"/>
                    </svg>
                  </button>
                </div>
                <div v-else class="code-input-wrap">
                  <input
                    v-model.trim="auth.validateValue.value"
                    :type="'text'"
                    :inputmode="'numeric'"
                    autocomplete="one-time-code"
                    :required="true"
                    placeholder="输入验证码"
                  />
                  <button
                    class="button secondary code-send-inline"
                    data-test="send-code"
                    type="button"
                    :disabled="!!auth.busy.value || auth.gt4Loading.value || !auth.accountReady.value"
                    @click="auth.sendCode"
                  >
                    {{ auth.busy.value === 'captcha' ? '等待验证…' : auth.busy.value === 'code' ? '发送中…' : '发送验证码' }}
                  </button>
                </div>
              </div>
            </div>

            <button
              class="button primary login-submit"
              type="submit"
              :disabled="!canSubmitPrimary"
            >
              <span v-if="auth.busy.value" class="spinning" aria-hidden="true">●</span>
              {{ auth.busy.value ? '登录中…' : '登录' }}
            </button>
          </form>
        </div>
      </template>

      <!-- 二次验证：只展示用户可理解的步骤与方式，不展示协议字段、业务码或 GT4 状态。 -->
      <template v-else>
        <section class="challenge-step" aria-label="二次验证">
          <h3>还差一步，请确认是你本人</h3>
          <p class="challenge-progress">
            安全验证第 {{ auth.challengeStep.value || 1 }} 步
            <template v-if="challengeTotalKnown !== null">
              （共 {{ challengeTotalKnown }} 步）
            </template>
          </p>
          <p class="challenge-method">{{ challengeMethodLabel }}</p>
          <p
            v-if="auth.selectedChallenge.value?.account"
            class="challenge-target"
          >
            {{ auth.selectedChallenge.value.account }}
          </p>

          <button
            v-if="auth.challengePending.value.length > 1"
            class="button ghost"
            data-test="challenge-switch"
            type="button"
            @click="choosingOtherMethods = !choosingOtherMethods"
          >
            改用其他验证方式
          </button>

          <div
            v-if="choosingOtherMethods && auth.challengePending.value.length > 1"
            class="pending-list"
            aria-label="其他验证方式"
          >
            <label
              v-for="item in auth.challengePending.value"
              :key="`${item.validateType}-${item.account ?? ''}`"
            >
              <input
                v-model.number="auth.selectedChallengeType.value"
                type="radio"
                name="pending-validation"
                :value="item.validateType"
              />
              <span>{{ validateTypeLabels[item.validateType] ?? '安全验证' }}</span>
            </label>
          </div>

          <label v-if="needsSupplementedTarget">
            <span>补充完整{{ auth.selectedChallenge.value?.validateType === 17 ? '手机号' : '邮箱' }}</span>
            <input
              v-model.trim="auth.supplementedTarget.value"
              data-test="challenge-supplement"
              :type="auth.selectedChallenge.value?.validateType === 17 ? 'tel' : 'email'"
              autocomplete="off"
            />
          </label>

          <label v-if="showChallengeSecretInput">
            <span>
              {{
                isPasswordValidation(auth.selectedChallenge.value?.validateType)
                  ? (validateTypeLabels[auth.selectedChallenge.value?.validateType as number] ?? '验证值')
                  : '验证值'
              }}
            </span>
            <div class="field-control" :class="{ 'code-input-row': auth.isChallengeCode.value }">
              <input
                v-model.trim="auth.challengeValue.value"
                data-test="challenge-value"
                :type="isPasswordValidation(auth.selectedChallenge.value?.validateType) ? 'password' : 'text'"
                :autocomplete="challengeValueAutocomplete"
                required
              />
              <button
                v-if="auth.isChallengeCode.value"
                class="button secondary code-send-inline"
                data-test="challenge-send-code"
                type="button"
                :disabled="!!auth.busy.value || auth.gt4Loading.value || auth.resendSeconds.value > 0"
                @click="auth.sendChallengeCode"
              >
                {{
                  auth.resendSeconds.value > 0
                    ? `${auth.resendSeconds.value}s 后可重发`
                    : auth.selectedChallenge.value?.validateType === 16
                      ? '发送邮箱验证码'
                      : '发送手机验证码'
                }}
              </button>
            </div>
          </label>

          <button
            v-if="showChallengeSecretInput"
            class="button primary login-submit"
            data-test="challenge-submit"
            type="button"
            :disabled="!!auth.busy.value || !auth.selectedChallenge.value || !auth.challengeValue.value.trim()"
            @click="auth.submitChallenge"
          >
            完成验证
          </button>

          <button
            class="button ghost"
            data-test="challenge-back"
            type="button"
            @click="auth.resetChallenge"
          >
            返回登录
          </button>
        </section>
      </template>

      <p v-if="auth.error.value" class="feedback error" role="alert">{{ auth.error.value }}</p>
      <p v-if="auth.notice.value" class="feedback notice" role="status">{{ auth.notice.value }}</p>
    </section>
  </main>
</template>
```

Wait — the ref callback approach above is fragile. Let me use a cleaner `@vue/ref-macros` compatible approach. Instead, use a simpler method:

Replace the script section's tab-related logic with this cleaner version. Use a keyed array of template refs:

```typescript
// In the <script setup>, add after existing computed/ref declarations:
const tabRefs = ref<(HTMLElement | null)[]>([])

/** 将滑动 indicator 定位到当前激活的 tab。jsdom 中 getBoundingClientRect 可能返回零。 */
function positionTabIndicator() {
  const activeIdx = loginMethodTabs.findIndex(t => t.method === props.auth.loginMethod.value)
  const el = tabRefs.value[activeIdx]
  const indicator = document.querySelector('.login-tab-indicator') as HTMLElement | null
  if (!el || !indicator) return
  const tabRect = el.getBoundingClientRect()
  const containerRect = el.parentElement!.getBoundingClientRect()
  indicator.style.left = `${tabRect.left - containerRect.left}px`
  indicator.style.width = `${tabRect.width}px`
}

// Watch loginMethod to reposition indicator on switch
watch(() => props.auth.loginMethod.value, () => {
  showPassword.value = false
  // nextTick ensures DOM has updated before measuring
  const nextTick = (window as any).Vue?.nextTick ?? ((fn: () => void) => setTimeout(fn, 0))
  nextTick(positionTabIndicator)
})

// Also position on mount
import { onMounted } from 'vue'
onMounted(() => {
  const nextTick = (window as any).Vue?.nextTick ?? ((fn: () => void) => setTimeout(fn, 0))
  nextTick(positionTabIndicator)
})
```

And in the template, use `:ref` with an array push pattern:

```html
<div
  class="login-method-tabs"
  role="tablist"
  aria-label="登录方式"
>
  <div class="login-tab-indicator" aria-hidden="true"></div>
  <button
    v-for="(tab, index) in loginMethodTabs"
    :key="tab.method"
    type="button"
    role="tab"
    class="login-method-tab"
    data-test="login-method-tab"
    :class="{ 'is-active': auth.loginMethod.value === tab.method }"
    :aria-selected="auth.loginMethod.value === tab.method"
    :ref="(el) => { tabRefs[index] = el as HTMLElement; if (auth.loginMethod.value === tab.method) positionTabIndicator() }"
    @click="() => { chooseLoginMethod(tab.method) }"
  >
    {{ tab.label }}
  </button>
</div>
```

- [ ] **Step 3: Fix the floating label HTML structure for account and password fields**

The key structural change: wrap each input in a `.field-group` div with the `<label>` as a sibling after the `<input>`, so the CSS `input:focus ~ label` selector works.

For the account input (email mode), change from:
```html
<label class="account-cell">
  <span>邮箱地址</span>
  <input ... />
</label>
```
To:
```html
<div class="field-group account-cell">
  <input ... />
  <label>邮箱地址</label>
</div>
```

For the password field (non-code mode), change from:
```html
<div class="secret-field">
  <label>
    <span>登录密码<span class="password-sentinel...">已保存密码</span></span>
  </label>
  <div class="code-input-wrap">
    <input ... />
  </div>
</div>
```
To:
```html
<div class="secret-field">
  <div class="field-group">
    <input :type="showPassword ? 'text' : 'password'" ... />
    <label>
      登录密码
      <span class="password-sentinel ..." aria-hidden="true">已保存密码</span>
    </label>
    <button type="button" class="eye-toggle" aria-label="切换密码可见性" @click="showPassword = !showPassword">
      <!-- SVG icons -->
    </button>
  </div>
</div>
```

For code mode, keep the existing `.code-input-wrap` structure (it doesn't need floating label since the label is implicit in the field group context, and the test checks `.secret-field.is-code` which we preserve).

- [ ] **Step 4: Write a minimal test to verify tab indicator exists in DOM**

Add this test to the end of `LoginPanel.test.ts`:

```typescript
it('渲染 tab 滑动指示器元素', async () => {
  const auth = authStub()
  const wrapper = mountPanel(auth)
  expect(wrapper.find('.login-tab-indicator').exists()).toBe(true)
})
```

- [ ] **Step 5: Run tests**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vitest run
```
Expected: All 213+ tests pass (213 original + 1 new = 214). If the new test fails because `.login-tab-indicator` doesn't exist, the template edit in Step 2 was incomplete.

If existing tests fail:
- `.login-method-tab.is-active` failing → check that `.is-active` class is still applied
- `.account-cell input` failing → check that `.account-cell` class is on the `.field-group` wrapper
- `.secret-field.is-code` failing → check that `.secret-field` still has `.is-code` class
- `.field-control` failing → check challenge step HTML is unchanged
- `.password-sentinel.is-visible` failing → check sentinel span is inside the new label
- `.login-primary-panel` height measurement failing → check grid structure is preserved

- [ ] **Step 6: Fix any failing tests, then commit**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/components/LoginPanel.vue im-app/ui/src/components/LoginPanel.test.ts
git commit -m "feat(login): add dual-panel layout, sliding tab indicator, eye toggle, floating labels"
```

---

### Task 4: Final regression and visual verification

**Files:**
- No code changes — verification only

**Interfaces:**
- Consumes: all previous tasks' output
- Produces: confirmed passing test suite + visual sanity check

- [ ] **Step 1: Run full test suite one final time**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vitest run
```
Expected: all tests pass (214 total).

- [ ] **Step 2: Build and verify no TypeScript errors**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vite build --mode development
```
Expected: ✓ built successfully, no errors.

- [ ] **Step 3: Visual check in browser**

Start the dev server and visually inspect:
```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui && npx vite --port 1420
```

Check:
- Dark theme: deep navy canvas (#0a0e14), warm gold accent, grid lines visible
- Light theme: blue-gray canvas (#f0f2f5), brighter gold accent (#c49a2f)
- Wide screen (≥900px): left panel shows logo on gold gradient, right panel has form
- Narrow screen (<900px): single column, no left panel
- Tab switching: white indicator slides smoothly between tabs
- Input focus: label floats up to top-left corner, border turns gold
- Password mode: eye toggle button appears, clicking shows/hides password
- Submit button: shows "登录中…" with spinner when busy
- Challenge flow: unchanged behavior, same elements present
- Theme toggle in topbar: persists across reloads via localStorage

- [ ] **Step 4: Commit any final fixes**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add -A
git commit -m "style(login): polish floating label transitions and responsive breakpoints"
```

---

## Self-Review Checklist

**Spec coverage:**
- ✅ Dual-panel layout (wide/narrow) — Task 2 CSS + Task 3 template
- ✅ Dark theme token updates — Task 1
- ✅ Light theme token updates with independent personality — Task 1
- ✅ Tab sliding indicator animation — Task 3
- ✅ Floating label on inputs — Task 3
- ✅ Password eye toggle — Task 3
- ✅ Submit button loading state — Task 3
- ✅ Staggered entry animation (80ms delay on form panel) — Task 2 CSS
- ✅ All 17 test-critical class names preserved — Task 3 template review
- ✅ All `data-test` attributes preserved — Task 3
- ✅ All ARIA attributes preserved — Task 3
- ✅ All 213 existing tests must pass — Task 4 gate

**Placeholder scan:** No TBD/TODO found. All code blocks are complete.

**Type consistency:** `loginMethodTabs` type `{ method: PrimaryLoginType; label: string }[]` used consistently. `showPassword: ref(false)` used in both template and watch. `tabRefs: ref<(HTMLElement | null)[]>([])` used in template `:ref` binding.

**Risk noted:** The `@ref` array callback in Vue 3 template uses `(el) => { tabRefs[index] = el as HTMLElement }` — this is standard Vue 3 template ref behavior and works in jsdom. The `positionTabIndicator` function guards against null `getBoundingClientRect` by checking `!el || !indicator` before proceeding.
