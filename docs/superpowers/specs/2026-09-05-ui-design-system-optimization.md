# IM 实时监控控制台 UI 设计系统优化

## 1. 目标

对 IM 实时监控控制台的前端 UI 进行系统性设计优化，统一视觉语言，提升暗色/亮色双主题的阅读舒适度和交互质感。

**优化范围：**
- 配色系统：重构 CSS 变量色板，统一双主题渐变层次
- 字体系统：规范字号层级与字体栈
- 按钮体系：优化所有操作按钮的视觉权重、反馈动效
- 交互细节：卡片 hover、抽屉过渡、通知横幅动画等

**不改变的内容：**
- 布局结构（侧栏/消息区/顶栏网格）
- 组件结构和交互逻辑
- 数据流和业务逻辑
- 无障碍（ARIA）语义

---

## 2. 当前问题

### 2.1 配色问题

| 问题 | 位置 |
|------|------|
| 暗色背景 `#080a0b` 过于接近纯黑，缺乏层次 | `base.css:6` |
| 部分颜色硬编码（`#59635f`、`#445155`），未走 CSS 变量 | `base.css:98`、`console.css:117` |
| 亮色画布 `#fafbfb` 与卡片 `#ffffff` 对比度仅 1.04:1 | `base.css:27`、`console.css:1843` |
| 强调色 amber 在亮色下偏暗（`#b8860b`），可读性不足 | `base.css:36` |
| 全局无 `--bg-*` 语义化色值，所有层级靠 `--ink-*` 表达，命名混乱 | `base.css:6-12` |

### 2.2 按钮问题

| 问题 | 位置 |
|------|------|
| 按钮高度 38px 偏小，触控友好度不足 | `base.css:131` |
| 只有 hover 反馈，无 active 按压感 | `base.css:140-144` |
| 图标按钮与主要按钮视觉权重接近 | `base.css:127-133` vs `console.css:927-935` |
| 过渡时间 140ms 偏短，体验偏"硬" | `base.css:136` |
| 危险按钮透明度 0.08 过于隐晦 | `base.css:155` |

### 2.3 字体问题

| 问题 | 位置 |
|------|------|
| 字体栈包含 emoji 字体在 main font 位置 | `base.css:5` |
| 字号层级扁平，无明确 scale | `base.css` / `console.css` 多处 |
| 消息内容 15px 与整体 11-12px 跨度大，视觉上跳 | `console.css:1379` |

### 2.4 交互问题

| 问题 | 位置 |
|------|------|
| 消息卡片 hover 只改边框，反馈不够 | `console.css:1334` |
| 群组列表行 hover 背景变化太小（0.025 → 0.04） | `console.css:1017-1019` |
| 抽屉过渡 cubic-bezier 未使用，只有 `ease` | `console.css:818` |
| 全局通知横幅无滑入动画 | `console.css:753-768` |
| `GroupSidebar.vue` 中有独立的 toggle slider 实现，与主 CSS `.monitor-switch` 重复 | `GroupSidebar.vue:177-223` |

---

## 3. 设计决策

### 3.1 配色系统

#### 暗色主题

采用 GitHub Dark Dimmed 级别的色调基准，保留工业感但降低纯黑压迫感。

| Token | 新值 | 用途 |
|-------|------|------|
| `--bg-canvas` | `#0d1117` | 画布背景 |
| `--bg-surface` | `#161b22` | 卡片/面板主体 |
| `--bg-elevated` | `#1c2128` | 顶栏 |
| `--bg-elevated-2` | `#21262d` | 侧栏 |
| `--border-subtle` | `#30363d` | 细边框/分隔线 |
| `--border-medium` | `#3d444d` | 输入框/按钮边框 |
| `--text-primary` | `#e6edf3` | 主文字 |
| `--text-secondary` | `#8b949e` | 次要文字 |
| `--text-tertiary` | `#6e7681` | 弱文字 |
| `--accent` | `#f0b446` | 琥珀强调色 |
| `--accent-soft` | `#d4922e` | 琥珀 hover/active |
| `--success` | `#3fb950` | 绿色成功 |
| `--info` | `#58a6ff` | 蓝色辅助 |
| `--danger` | `#f85149` | 红色危险 |
| `--focus-ring` | `#d4922e` | Focus 环 |

网格背景线：`rgba(255,255,255,0.025)`（当前 0.018，提升可见度）。

#### 亮色主题

提高灰阶递进的对比度，让层级分明。

| Token | 新值 | 用途 |
|-------|------|------|
| `--bg-canvas` | `#f6f8fa` | 画布（柔和灰） |
| `--bg-surface` | `#ffffff` | 卡片（纯白） |
| `--bg-elevated` | `#f0f3f6` | 顶栏 |
| `--bg-elevated-2` | `#e8ecf0` | 侧栏 |
| `--border-subtle` | `#d0d7de` | 细边框 |
| `--border-medium` | `#afb8c1` | 输入框边框 |
| `--text-primary` | `#1f2328` | 主文字 |
| `--text-secondary` | `#656d76` | 次要文字 |
| `--text-tertiary` | `#8c959f` | 弱文字 |
| `--accent` | `#b08800` | 琥珀强调色 |
| `--accent-soft` | `#9a7209` | 琥珀 hover/active |
| `--success` | `#1a7f37` | 绿色 |
| `--info` | `#0969da` | 蓝色 |
| `--danger` | `#cf222e` | 红色 |
| `--focus-ring` | `#0969da` | Focus 环（亮色用蓝色更清晰） |

**废弃旧变量**：`--ink-950` ~ `--ink-600`、`--text-100`/`300`/`500`、`--amber`/`--amber-soft` 等旧命名全部替换为新语义化 token，保持向后兼容的做法不可取——直接重写，避免两套变量并存。

### 3.2 字体系统

#### 字体栈

```css
font-family: "IBM Plex Sans", "PingFang SC", "Noto Sans SC",
             "Helvetica Neue", "Segoe UI", system-ui, sans-serif;
```

- 移除 `Apple Color Emoji` 等 emoji 字体（emoji 由系统默认处理）
- `PingFang SC` 提至 `Noto Sans SC` 之前（macOS/iOS 优先）
- 添加 `system-ui` 兜底

Monospace 字体栈保持不变：
```css
font-family: "IBM Plex Mono", "SFMono-Regular", Consolas, monospace;
```

#### 字号层级（Major Third 1.25x scale）

| Level | Size | Weight | Line-height | Use Case |
|-------|------|--------|-------------|----------|
| xs | 10px | 600 mono | 1.4 | eyebrow/标签 |
| sm | 11px | 500 | 1.5 | 辅助文字、meta info |
| base | 13px | 400 | 1.55 | 常规正文 |
| md | 14px | 400 | 1.6 | 消息内容（从 15px 下调） |
| lg | 17px | 600 | 1.3 | 标题 |
| xl | 21px | 700 | 1.2 | 统计数字 |
| xxl | 28px | 800 | 1.1 | emoji-only 消息 |

### 3.3 按钮体系

#### 主按钮（`.button.primary`）

- 高度：38px → 40px
- 添加 box-shadow：暗色 `0 1px 3px rgba(0,0,0,0.25)`，亮色 `0 1px 2px rgba(0,0,0,0.08)`
- hover：box-shadow 加深 + background 微亮
- active：`transform: translateY(1px)` + shadow 消除，模拟按压

#### 次要按钮（`.button.secondary`）

- hover 背景从 `var(--ink-700)` 改为更柔和的 `rgba(255,255,255,0.04)`（暗色）/ `rgba(0,0,0,0.03)`（亮色）
- 过渡时间：140ms → 180ms ease

#### 图标按钮（`.icon-button`）

- 尺寸：36px → 38px
- 添加 box-shadow，hover 时微浮起

#### 幽灵按钮（`.button.ghost`）

- hover 背景：`rgba(255,255,255,0.03)`（暗色）/ `rgba(0,0,0,0.03)`（亮色）

#### 危险按钮（`.button.danger`）

- 背景透明度：0.08 → 0.12
- border-color 透明度同步提高

#### 通用过渡

- 所有 transition-duration：140ms → 180ms ease
- active 状态过渡：80ms ease-out

### 3.4 交互动效

| 元素 | 动画 | 时长 | Easing |
|------|------|------|--------|
| 按钮 hover | background/border-color/box-shadow | 180ms | ease |
| 按钮 active | transform/box-shadow | 80ms | ease-out |
| 消息卡片 hover | background/border/box-shadow | 200ms | ease |
| 群组行 hover | background | 150ms | ease |
| 侧栏收起/展开 | width/grid-template-columns | 220ms | cubic-bezier(0.4, 0, 0.2, 1) |
| 抽屉滑入/出 | transform: translateX | 220ms | cubic-bezier(0.4, 0, 0.2, 1) |
| 监控开关 | background/transform | 200ms | ease |
| 主题切换 | 全局颜色过渡 | 300ms | ease |
| 通知横幅 | slide-in（从右侧） | 250ms | cubic-bezier(0.4, 0, 0.2, 1) |
| 新消息高亮 | background fade-in | 800ms | ease |
| 加载 grid | opacity pulse | 800ms | ease-in-out |

### 3.5 具体组件改动

#### 消息卡片（`.message-card`）
- hover：增加 `box-shadow: 0 2px 8px rgba(0,0,0,0.15)`（暗色）
- active 态：按压微缩（`transform: scale(0.995)`）

#### 群组列表行（`.group-row`）
- hover：背景从 `rgba(255,255,255,0.025)` 提升到 `rgba(255,255,255,0.045)`
- selected：保留 amber left border，增加 `box-shadow: inset 0 0 16px rgba(240,180,70,0.06)`

#### 全局通知横幅（`.global-error`）
- 新增 `@keyframes slide-in` 滑入动画
- 关闭按钮扩大点击区域至 28×28px

#### 监控开关（`.monitor-switch`）
- 统一 `GroupSidebar.vue` 中重复的 `.toggle-slider` 实现，移除该重复代码块
- 激活态背景使用 `--success` 变量而非硬编码

#### 搜索框（`.search-field input`）
- focus 外发光 ring：`box-shadow: 0 0 0 3px rgba(240,180,70,0.15)`（暗色）

#### 滚动条
- 暗色：thumb 透明度从 0.55 → 0.65，hover 从 0.88 → 0.95
- 亮色：同步调整

---

## 4. 文件变更范围

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `im-app/ui/src/styles/base.css` | 重写 | 色板变量、字体栈、按钮体系、工具类 |
| `im-app/ui/src/styles/console.css` | 重写 | 各组件颜色改用新变量、新增动效、交互细节 |
| `im-app/ui/src/components/GroupSidebar.vue` | 修改 | 移除重复 `.toggle-slider` 样式，改用主 CSS 变量 |
| `im-app/ui/index.html` | 修改 | 更新 `<meta name="theme-color">` 为暗色新值 |

---

## 6. 恢复启动页优化

当前 `recovering` 阶段只显示一行 `<p>` 文字，视觉空洞。改为与登录页风格统一的卡片式布局。

### 6.1 结构

```
[ logo 图标 ]
[ 状态标题 ]     ← "正在恢复会话…" / "正在切换账号…"
[ 原因副文案 ]   ← 仅在 retryableMessage 有值时显示（失败场景）
[ 操作按钮 ]     ← 重试 + 使用其他账号（仅失败时显示）
```

### 6.2 文案

| 场景 | 标题 | 副文案 |
|------|------|--------|
| 正常恢复中 | "正在恢复会话…" | （无） |
| 切换账号中 | "正在切换账号…" | （无） |
| 恢复失败 | "正在恢复会话…" | "网络连接失败，请重试" |
| 切换失败 | "正在切换账号…" | "网络连接失败，请重试" |
| 退出未确认 | "正在恢复会话…" | "本次无法确认已退出，请重试" |

### 6.3 视觉

- 居中卡片，宽度 400px，最大宽度 90vw
- 卡片背景 `--bg-surface`，边框 `--border-subtle`，圆角 8px
- 顶部 Logo：32×32px（复用 `/32x32.png`），margin-bottom 16px
- 加载指示器：3 个点的弹性跳动动画（css only，无需图片）
- 标题字号 17px，字重 600，color `--text-primary`
- 副文案字号 13px，color `--text-secondary`
- 入场动画：fade + slide-up，300ms ease-out

### 6.4 文件变更

- `im-app/ui/src/App.vue`：模板结构调整（加入 logo、加载指示器、副文案层级），更新文字，新增 scoped style
- `im-app/ui/index.html`：`theme-color` 从 `#0b0e0f` 更新为 `#0d1117`

---

## 7. 验收标准

- [ ] 暗色主题下所有层次背景有明确的视觉区分（至少 3 级递进）
- [ ] 亮色主题下所有层次背景有明确的视觉区分
- [ ] 暗色背景不是纯黑（避免 `#000000`）
- [ ] 亮色背景不是纯白作为画布（避免 `#ffffff` 作为最大面积色）
- [ ] 所有按钮 hover/active 状态有明确的视觉反馈
- [ ] 所有 CSS 颜色通过变量引用，无硬编码颜色值（已定义的除外）
- [ ] 主题切换后所有颜色正确响应，无遗漏
- [ ] 抽屉收起/展开动画流畅，无卡顿
- [ ] 消息卡片 hover 有反馈感
- [ ] 全局通知横幅有滑入动画
- [ ] 字体大小层级清晰，无突兀跳变
- [ ] 恢复启动页有 logo、标题、可选副文案和入场动画
- [ ] 正常恢复中不显示重试按钮，失败时才显示
- [ ] 切换账号时标题文案正确（"正在切换账号…"）
- [ ] 所有测试通过
- [ ] prefers-reduced-motion 媒体查询继续生效
