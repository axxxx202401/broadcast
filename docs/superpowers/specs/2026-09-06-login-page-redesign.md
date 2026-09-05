# 登录页重新设计

## 1. 目标

对登录页面进行全面视觉与交互升级，解决以下核心问题：
- 当前单列窄卡片布局缺乏品牌表现力，用户进入时第一印象平淡
- 暗色主题偏严肃、亮色主题缺乏独立个性
- Tab 切换、输入框 focus、验证码按钮等交互细节反馈不足

**优化范围：**
- 布局重构：宽屏双栏分割（左侧品牌区 + 右侧表单区），窄屏单列回退
- 主题系统增强：暗色提升质感层次，亮色建立独立个性（保持整体色调一致）
- 交互增强：tab 滑入动画、floating label、密码可见性切换、提交 loading、验证码倒计时
- 保留所有现有功能：primary login / challenge 二次验证 / 已保存账号选择 / 国家区号

**不改变的内容：**
- 业务逻辑（useAuth composable、账号管理、Tauri 通信）
- 无障碍语义（ARIA 属性）
- 测试覆盖率要求（已有测试需继续通过）

---

## 2. 当前问题

### 2.1 布局问题

| 问题 | 位置 |
|------|------|
| `.login-shell` 固定 560px 宽，内容密度低，缺少品牌展示空间 | `console.css:2-9` |
| `.login-card` 单列堆叠，无左右分区，视觉扁平 | `console.css:12-24` |
| 品牌侧（`.login-intro` / `.login-console`）仅在宽屏下通过内联 HTML 渲染，窄屏才折叠 | LoginPanel.vue（无对应布局说明） |
| 左侧 brand 区与右侧表单区没有通过 CSS grid 显式关联，依赖 HTML 顺序隐式布局 | — |

### 2.2 主题问题

| 问题 | 位置 |
|------|------|
| 暗色主题 `#0d1117` 画布与 `#161b22` 卡片对比仅 1.33:1，层次不够分明 | `base.css:9-10` |
| 亮色主题 accent `#b08800`（暗黄）对比度不足，视觉上偏弱 | `base.css:54` |
| 亮色主题仅做了暗色的"浅灰反色"，缺少独立的视觉个性 | `base.css:35-60` |
| body 网格线在亮色下 `rgba(0,0,0,0.04)` 过于微弱，几乎不可见 | `base.css:89-92` |
| 登录卡片阴影暗色 `0 24px 70px rgba(0,0,0,0.38)` 过深，亮色却只有 `0.08`，亮度跨度大 | `console.css:22` vs `console.css:2001` |

### 2.3 交互问题

| 问题 | 位置 |
|------|------|
| 登录方式 tab 切换无动画，选中状态仅底部 2px 色条 | `console.css:248-252` |
| 输入框 focus 只有黄色光晕，无 label 联动反馈 | `console.css:120-124` |
| 验证码按钮绝对定位嵌入输入框，视觉割裂 | `console.css:134-145` |
| 密码模式无可见性切换按钮 | `LoginPanel.vue:211` |
| 提交按钮 disabled 状态与 hover 状态区分度低 | `base.css:195-199` |
| 无 loading 旋转指示器嵌入按钮内部 | — |

---

## 3. 设计方案

### 3.1 布局：双栏分割

**宽屏（≥900px）**

```
┌─────────────────────────────────────────────────────┐
│  LEFT (45%)          │  RIGHT (55%)                 │
│  ┌──────────────────┐ │  ┌──────────────────────┐   │
│  │                  │ │  │  [返回按钮]           │   │
│  │  渐变背景         │ │  ├──────────────────────┤   │
│  │  Logo + Slogan   │ │  │  已保存账号选择        │   │
│  │  [可选装饰元素]   │ │  ├──────────────────────┤   │
│  │                  │ │  │  Tab 栏              │   │
│  │                  │ │  ├──────────────────────┤   │
│  │                  │ │  │  表单区域            │   │
│  │                  │ │  │  (响应式高度)        │   │
│  │                  │ │  ├──────────────────────┤   │
│  │                  │ │  │  提交按钮            │   │
│  │                  │ │  └──────────────────────┘   │
│  └──────────────────┘ │                              │
└─────────────────────────────────────────────────────┘
```

- 左栏：固定比例 45%，背景用从 accent 色派生的微妙渐变，放置 logo（64×64px）+ slogan（"进入实时监控控制台"）
- 右栏：flex 1，承载所有表单逻辑，padding 加大至 48px
- 两栏之间用 1px border 分隔

**窄屏（<900px）**

- 左栏隐藏，logo + slogan 移至右栏顶部作为 header
- 表单区占满宽度，padding 保持一致

**CSS 结构变化：**

```css
/* .login-shell 改为双栏 grid */
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
}

/* 左栏品牌区 */
.login-brand {
  background: linear-gradient(160deg, var(--accent) 0%, var(--accent-soft) 100%);
  display: grid;
  place-content: center;
  padding: 64px 48px;
  position: relative;
  overflow: hidden;
}

/* 右栏表单区 */
.login-form-panel {
  display: flex;
  flex-direction: column;
  padding: 48px;
  background: var(--bg-surface);
  overflow-y: auto;
}
```

### 3.2 主题增强

#### 暗色主题

保留 GitHub Dark 基调，提升层次对比：

| Token | 当前值 | 新值 | 说明 |
|-------|--------|------|------|
| `--bg-canvas` | `#0d1117` | `#0a0e14` | 更深邃，增加纵向层次 |
| `--bg-surface` | `#161b22` | `#1a1f28` | 微升明度，与画布对比提升至 1.5:1 |
| `--bg-elevated` | `#1c2128` | `#212732` | 顶栏/面板 |
| `--bg-elevated-2` | `#21262d` | `#262d3a` | 次级面板 |
| `--border-subtle` | `#30363d` | `#2d3340` | 细边框 |
| `--border-medium` | `#3d444d` | `#383f4d` | 主边框 |
| `--text-primary` | `#e6edf3` | 保持 | — |
| `--text-secondary` | `#8b949e` | `#848d9a` | 微降一点灰度 |
| `--text-tertiary` | `#6e7681` | `#636c76` | — |
| `--accent` | `#f0b446` | 保持 | 品牌金色 |
| `--accent-soft` | `#d4922e` | 保持 | — |
| `--focus-ring` | `#d4922e` | 保持 | — |
| `--radius` | `6px` | `10px` | 整体圆角加大，更现代 |

- body 网格线从 `rgba(255,255,255,0.025)` 提升为 `rgba(255,255,255,0.035)`
- 登录卡片改为圆角 16px，无独立 border（两栏拼接成一个整体卡片）
- 左栏渐变叠加：在 accent 基础上叠加 `rgba(0,0,0,0.15)` 增加深度

#### 亮色主题

建立独立个性，不采用"浅灰反色"策略：

| Token | 当前值 | 新值 | 说明 |
|-------|--------|------|------|
| `--bg-canvas` | `#f6f8fa` | `#f0f2f5` | 柔和浅灰，带极淡蓝调 |
| `--bg-surface` | `#ffffff` | `#ffffff` | 保持纯白 |
| `--bg-elevated` | `#f0f3f6` | `#e8ebf0` | 顶栏/面板 |
| `--bg-elevated-2` | `#e8ecf0` | `#e0e4ea` | 次级面板 |
| `--border-subtle` | `#d0d7de` | `#cdd3da` | — |
| `--border-medium` | `#afb8c1` | `#b8c0ca` | — |
| `--text-primary` | `#1f2328` | 保持 | — |
| `--text-secondary` | `#656d76` | 保持 | — |
| `--text-tertiary` | `#8c959f` | `#858e99` | — |
| `--accent` | `#b08800` | `#c49a2f` | 提高明度，与暗色 accent 同一色系但更亮 |
| `--accent-soft` | `#9a7209` | `#a8852a` | — |
| `--focus-ring` | `#0969da` | `#c49a2f` | 与 accent 统一 |
| `--radius` | `6px` | `10px` | — |

- body 网格线：`rgba(0,0,0,0.06)`（适度提升可见度）
- 左栏渐变：`linear-gradient(160deg, #c49a2f 0%, #a8852a 100%)`，与暗色同色系
- 亮色下登录卡片加轻微内阴影 `inset 0 1px 0 rgba(255,255,255,0.8)` 增加材质感

### 3.3 交互增强

#### Tab 切换动画

将现有的 4 个 tab 从 grid 改为相对定位 + sliding indicator：

```css
.login-method-tabs {
  position: relative;
  display: flex;
  gap: 0;
  height: 44px;
  background: var(--bg-elevated);
  border-radius: var(--radius);
  padding: 3px;
}

.login-method-tab {
  /* 保持现有样式，去掉 border-radius，改为 flex:1 */
  flex: 1;
  border-radius: calc(var(--radius) - 3px);
  /* 去掉 active 状态的背景替换，改为由 JS 驱动 indicator */
}

/* sliding indicator：绝对定位，跟随 active tab */
.login-tab-indicator {
  position: absolute;
  top: 3px;
  height: calc(100% - 6px);
  background: var(--bg-surface);
  border-radius: calc(var(--radius) - 3px);
  box-shadow: 0 1px 4px rgba(0,0,0,0.12);
  transition: left 220ms cubic-bezier(0.4, 0, 0.2, 1), width 220ms cubic-bezier(0.4, 0, 0.2, 1);
}
```

实现方式：在 `LoginPanel.vue` 中用 `ref` + `nextTick` + `getBoundingClientRect` 计算 active tab 位置，CSS transition 驱动滑动。

#### 输入框 Floating Label

当 input 获得 focus 或有值时，label 上浮至 input 顶部边界并与左边框对齐：

```css
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
  font-size: 13px;
  pointer-events: none;
  transition: all 200ms cubic-bezier(0.4, 0, 0.2, 1);
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
```

注意：现有模板中使用 `<label><span>字段名</span><input /></label>` 结构，需要改为 `.field-group` wrapper 包裹 `<input>` 和 `<label>` 为兄弟元素才能用 CSS 同级选择器 `~`。这是一个 HTML 结构变更。

#### 密码可见性切换

在密码输入框右侧添加眼睛图标按钮，点击切换 `type="password"` ↔ `type="text"`。

```vue
<div class="field-control">
  <input ... :type="showPassword ? 'text' : 'password'" />
  <button type="button" class="eye-toggle" @click="showPassword = !showPassword">
    <!-- SVG icon -->
  </button>
</div>
```

#### 提交按钮 Loading

当 `auth.busy.value` 为 truthy 时，按钮内部显示旋转指示器，文字替换为"登录中…"：

```vue
<button :disabled="!canSubmitPrimary" class="button primary login-submit">
  <span v-if="auth.busy.value" class="spinning">●</span>
  {{ auth.busy.value ? '登录中…' : '登录' }}
</button>
```

#### 验证码倒计时

现有实现已在 `resendSeconds` 中处理，只需优化按钮视觉：倒计时期间按钮显示灰色 + 圆角 pill 样式，结束后恢复正常。当前已在按钮文案上体现（`${resendSeconds}s 后可重发`），保持不变。

#### 整体进入动画

保留现有 `console-enter`（opacity + translateY），优化 easing：从 `ease-out` 改为 `cubic-bezier(0.4, 0, 0.2, 1)`（material standard）。同时为左栏 brand 区和右栏表单区设置 staggered 进入（右栏延迟 80ms）。

---

## 4. 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `im-app/ui/src/styles/base.css` | 修改 | CSS 变量更新：暗色提升对比、亮色独立个性、radius 6→10 |
| `im-app/ui/src/styles/console.css` | 重写 | 登录相关样式全部重构：双栏布局、floating label、tab 动画、brand 区 |
| `im-app/ui/src/components/LoginPanel.vue` | 修改 | HTML 结构调整：双栏 wrapper、floating label 结构、密码可见性 toggle、tab indicator |
| `im-app/ui/src/composables/useTheme.ts` | 可选修改 | 如需支持 `prefers-color-scheme` 自动检测则扩展；否则保持不变 |

---

## 5. 保留不变的确认项

- `useAuth` composable 的所有状态和方法不变
- `AccountSummary`、`PrimaryLoginType` 等类型定义不变
- ARIA 属性（`role="tablist"`, `aria-selected`, `aria-label` 等）全部保留
- 响应式断点保持 `900px`
- 已有单元测试（`LoginPanel.test.ts`、`useAuth.test.ts`）的断言不变
- 多账号恢复流程（`restore`、`switchAccount`、`addAccount`）逻辑不变
- 验证码发送逻辑（`sendCode`、`sendChallengeCode`、`resendSeconds`）不变
- Challenge 二次验证流程（`selectedChallenge`、`challengePending`、`submitChallenge`）不变

---

## 6. 测试兼容性约束

以下 CSS 类名被 `LoginPanel.test.ts` 直接引用，**必须保留**，不得重命名或删除：

| 类名 | 测试引用位置 | 用途 |
|------|-------------|------|
| `.login-method-tab` | 多处 | tab 元素，`.is-active` 修饰选中态 |
| `.login-submit` | `wrapper.get('.login-submit')` | 提交按钮 |
| `.account-row` + `.is-phone` | `wrapper.find('.account-row.is-phone')` | 手机号行容器 |
| `.account-cell` | `wrapper.get('.account-cell input')` | 账号输入框所在 label |
| `.secret-field` + `.is-code` | `wrapper.get('.secret-field.is-code')` | 密码/验证码字段容器 |
| `.field-control` | `wrapper.find('.field-control')` | 验证码模式下 input+按钮行 |
| `.password-sentinel` + `.is-visible` | `wrapper.find('.password-sentinel.is-visible')` | 已保存密码标记 |
| `.challenge-step` | `wrapper.find('.challenge-step')` | 二次验证区域 |
| `.login-back` | `data-test="login-back"` + `wrapper.get('[data-test="login-back"]')` | 返回按钮 |
| `.login-logo` | 无测试引用 | logo 图片 |
| `.purpose` | 无测试引用 | slogan 段落 |
| `.login-primary-panel` | `wrapper.get('.login-primary-panel')` | 主登录面板（用于测量高度一致性） |
| `.login-form` | 无直接引用 | 表单元素 |
| `.login-form-fields` | 无直接引用 | 字段容器 |
| `.code-input-wrap` | 无直接引用 | 验证码输入+发送按钮容器 |
| `.country-code-cell` | 无直接引用 | 国家区号 label |
| `.pending-list` | 无直接引用 | 其他验证方式列表 |

**HTML 结构变更原则：**
- 新增双栏 wrapper（`.login-brand`、`.login-form-panel`）可以插入在 `.login-card` 内部的第一层
- `.login-card` 本身保留，仅修改其 CSS 为双栏 grid
- floating label 实现需要调整内部 label/input 关系，但不能改变外层 class 名称
- `.login-primary-panel` 保持作为主登录表单区的 wrapper，测试用它测量高度
- 所有 `data-test` 属性全部保留不动

---

## 7. 测试策略

- 所有已有测试必须继续通过（不改行为逻辑）
- 新增交互（floating label、eye toggle、tab indicator）无需额外测试，因它们纯 CSS/视觉层面
- 双栏布局的响应式断点行为可通过视觉回归确认（已有 `LoginPanel.test.ts` 覆盖 DOM 结构）

---

## 8. 风险评估

| 风险 | 影响 | 缓解 |
|------|------|------|
| Floating label 改变 DOM 结构可能影响现有测试的 CSS 选择器 | 中 | 检查 `LoginPanel.test.ts` 所有 query selector，确保兼容新结构 |
| Tab indicator 用 JS 计算位置，SSR/测试环境下 `getBoundingClientRect` 可能失败 | 低 | 在测试中 mock `getBoundingClientRect`，或 fallback 到 index-based 定位 |
| 双栏布局在非 Tauri WebView（纯浏览器测试）中的表现 | 低 | 断点 900px 以下强制单列，测试环境通常窄于该值 |
| 亮色主题 accent 色变化影响其他组件（按钮、badge 等） | 高 | 仅在 base.css 中修改 token 值，确保全局一致；逐个组件回归检查 |

---

## 9. 实施顺序建议

1. **base.css**：更新 CSS 变量（先暗色后亮色，每个改完立即 build 验证）
2. **console.css**：重写登录相关样式（双栏布局 → floating label → tab 动画）
3. **LoginPanel.vue**：调整 HTML 结构适配新 CSS（floating label wrapper、eye toggle、tab indicator）
4. **回归测试**：运行 `npm test`，修复 break 的测试
5. **视觉验收**：手动切换暗色/亮色，检查宽屏/窄屏表现
