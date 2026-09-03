# Message Content Experience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将群消息界面改为内容优先的消息卡片，增加监控 groupId 汇总、Emoji、安全普通文案、窄屏侧栏和美化滚动条。

**Architecture:** `useMonitor` 提供不受搜索影响的监控群 ID；小组件分别负责群 ID 汇总、Emoji 文本和单条消息卡片。`MessagePanel` 保留现有虚拟滚动及历史锚点职责，根布局只负责响应式侧栏，避免把展示逻辑继续堆入一个组件。

**Tech Stack:** Vue 3、TypeScript 5、CSS Grid/Flexbox、TanStack Vue Virtual 3、Vitest、Vue Test Utils

**Depends on:** 可独立于账号后端计划执行；与 `/Volumes/TRANSCEND/works/objects/rust/broadcast/docs/superpowers/plans/2026-09-03-account-auth-experience.md` 同时修改 `App.vue` 和 `console.css` 时必须按提交顺序合并冲突。

---

## 文件结构

- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/emoji.ts`：方括号别名分词和纯 Emoji 判断。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/emoji.test.ts`：Emoji 单元测试。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageText.vue`：只用文本节点渲染消息。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageText.test.ts`：安全渲染和纯 Emoji 样式测试。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MonitoredGroupSummary.vue`：监控 groupId 折叠汇总。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MonitoredGroupSummary.test.ts`：0、1–5、6+ 群交互测试。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageCard.vue`：消息元信息和正文卡片。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageCard.test.ts`：内容层级和普通文案测试。
- 新建 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useResponsiveSidebar.ts`：窄屏侧栏状态。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.vue`：接入汇总和卡片，校准虚拟行高。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageBody.vue`：接入 Emoji 文本并移除技术类型标签。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/GroupSidebar.vue`、`StatusBadge.vue`、`App.vue`：普通用户文案和响应式侧栏。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useMonitor.ts`：导出监控 groupId。
- 修改 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/base.css` 和 `console.css`：卡片、布局和滚动条。

### Task 1：统一普通用户文案

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/GroupSidebar.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/StatusBadge.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageBody.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`
- Test: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.test.ts`
- Test: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.test.ts`

- [ ] **Step 1：写禁用技术文案失败测试**

```typescript
it('主界面不显示开发术语', () => {
  const wrapper = mountAuthenticatedApp()
  const visible = wrapper.text()
  for (const forbidden of [
    'ALL CHANNELS',
    'ALL MONITORED CHANNELS',
    'LIVE MESSAGE STREAM',
    'CHANNEL /',
    'UID /',
    '链路在线',
    '断开链路',
    '正文和附件由 Rust 解密',
  ]) {
    expect(visible).not.toContain(forbidden)
  }
})
```

在 `MessageBody.test.ts` 增加断言：文本消息没有“文本”标签，媒体按钮没有“解密”字样。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/App.test.ts src/components/MessagePanel.test.ts src/components/MessageBody.test.ts
```

预期：当前模板包含被禁文案，测试失败。

- [ ] **Step 3：替换或删除文案**

使用以下固定对应关系：

```typescript
const connectionLabels = {
  disconnected: '已断开',
  connecting: '连接中',
  connected: '已连接',
} as const
```

- `ALL CHANNELS` → `全部群聊`
- `ALL MONITORED CHANNELS` → `全部监控群聊`
- `LIVE MESSAGE STREAM` → `群消息`
- `CHANNEL / 123` → `群 ID：123`
- `UID 100267` → `用户 100267`
- `只读采集` → `正在接收`
- `N 条已载入` → `N 条消息`
- `断开链路` → `断开连接`
- 空态 → `选择需要监控的群后，新消息会显示在这里`

删除消息页脚中的 Rust 解密说明。媒体按钮改为“打开图片”“打开音频”“打开视频”“打开文件”和“正在打开…”。

- [ ] **Step 4：运行测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/App.test.ts src/components/MessagePanel.test.ts src/components/MessageBody.test.ts
```

预期：禁词和用户文案测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/App.vue im-app/ui/src/App.test.ts im-app/ui/src/components/GroupSidebar.vue im-app/ui/src/components/StatusBadge.vue im-app/ui/src/components/MessagePanel.vue im-app/ui/src/components/MessagePanel.test.ts im-app/ui/src/components/MessageBody.vue im-app/ui/src/components/MessageBody.test.ts
git commit -m "refactor: simplify user-facing monitor copy"
```

### Task 2：增加监控 groupId 汇总

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useMonitor.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useMonitor.test.ts`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MonitoredGroupSummary.vue`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MonitoredGroupSummary.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`

- [ ] **Step 1：写数据源和折叠失败测试**

```typescript
it('监控群 ID 不受侧栏搜索影响', () => {
  const monitor = setupMonitor([
    group('101', '运维群', 1),
    group('202', '研发群', 1),
    group('303', '其他群', 0),
  ])
  monitor.search.value = '运维'
  expect(monitor.monitoredGroupIds.value).toEqual(['101', '202'])
})

it('超过五个群时默认折叠并可展开', async () => {
  const wrapper = mount(MonitoredGroupSummary, {
    props: { groupIds: ['1', '2', '3', '4', '5', '6', '7'] },
  })
  expect(wrapper.text()).toContain('另有 2 个')
  expect(wrapper.text()).not.toContain('#7')
  await wrapper.get('button').trigger('click')
  expect(wrapper.text()).toContain('#7')
  expect(wrapper.text()).toContain('收起')
})
```

另测空数组显示“尚未选择监控群”，1–5 个不出现按钮。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useMonitor.test.ts src/components/MonitoredGroupSummary.test.ts
```

预期：computed 和组件尚不存在。

- [ ] **Step 3：实现 computed 和组件**

```typescript
const monitoredGroupIds = computed(() =>
  groups.value
    .filter(({ monitored }) => monitored !== 0)
    .map(({ group_id }) => group_id),
)
```

组件契约：

```typescript
const props = defineProps<{ groupIds: string[] }>()
const expanded = ref(false)
const visibleIds = computed(() => expanded.value ? props.groupIds : props.groupIds.slice(0, 5))
const hiddenCount = computed(() => Math.max(0, props.groupIds.length - 5))
```

只在 `MessagePanel` 的 `group === null` 时渲染汇总。`App.vue` 必须传 `monitor.monitoredGroupIds.value`，不能传受搜索影响的 `filteredGroups`。

- [ ] **Step 4：运行测试与类型检查**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useMonitor.test.ts src/components/MonitoredGroupSummary.test.ts src/components/MessagePanel.test.ts
npm run typecheck
```

预期：0、1–5、6+ 群以及搜索隔离测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/composables/useMonitor.ts im-app/ui/src/composables/useMonitor.test.ts im-app/ui/src/components/MonitoredGroupSummary.vue im-app/ui/src/components/MonitoredGroupSummary.test.ts im-app/ui/src/components/MessagePanel.vue im-app/ui/src/components/MessagePanel.test.ts im-app/ui/src/App.vue
git commit -m "feat: summarize monitored group ids"
```

### Task 3：实现安全 Emoji 分词

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/emoji.ts`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/utils/emoji.test.ts`

- [ ] **Step 1：写 Emoji 失败测试**

```typescript
describe('tokenizeMessageText', () => {
  it('转换已知别名并保留未知别名', () => {
    expect(tokenizeMessageText('[呲牙][憨笑][未知]')).toEqual([
      { kind: 'emoji', source: '[呲牙]', value: '😁' },
      { kind: 'emoji', source: '[憨笑]', value: '😄' },
      { kind: 'text', value: '[未知]' },
    ])
  })

  it('保留混合文本和潜在 HTML', () => {
    expect(
      tokenizeMessageText('告警<script>[呲牙]').map(({ value }) => value).join(''),
    ).toBe('告警<script>😁')
  })

  it('识别少量纯 Emoji 内容', () => {
    expect(isEmojiOnly(tokenizeMessageText('[呲牙] 😄'))).toBe(true)
    expect(isEmojiOnly(tokenizeMessageText('收到 😄'))).toBe(false)
  })
})
```

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/utils/emoji.test.ts
```

预期：Emoji 工具不存在。

- [ ] **Step 3：实现映射和线性分词**

```typescript
export type MessageTextToken =
  | { kind: 'text'; value: string }
  | { kind: 'emoji'; source: string; value: string }

export const BRACKET_EMOJI = {
  '[呲牙]': '😁',
  '[憨笑]': '😄',
} as const

const ALIAS_PATTERN = /\[[^[\]\r\n]{1,16}\]/gu
const EMOJI_ONLY_PATTERN = /^(?:\s|\p{Extended_Pictographic}|\p{Emoji_Presentation}|\uFE0F|\u200D)+$/u
```

`tokenizeMessageText` 按正则命中位置保留前后文本；只转换映射表命中项，未知项合并到文本 token。`isEmojiOnly` 先拼接转换后的可见文本，再要求字符全部符合 Emoji、变体选择符、ZWJ 或空白且可见 Emoji 不超过 6 个。不要引入 `v-html` 或第三方远程 Emoji 资源。

- [ ] **Step 4：运行测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/utils/emoji.test.ts
npm run typecheck
```

预期：别名、未知项、HTML 文本和纯 Emoji 测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/utils/emoji.ts im-app/ui/src/utils/emoji.test.ts
git commit -m "feat: tokenize message emoji aliases"
```

### Task 4：渲染 Emoji 和内容优先消息卡片

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageText.vue`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageText.test.ts`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageCard.vue`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageCard.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageBody.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessageBody.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/base.css`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.css`

- [ ] **Step 1：写安全渲染和层级失败测试**

```typescript
it('使用文本节点显示别名且不解析 HTML', () => {
  const wrapper = mount(MessageText, { props: { text: '<img src=x onerror=alert(1)>[呲牙]' } })
  expect(wrapper.text()).toBe('<img src=x onerror=alert(1)>😁')
  expect(wrapper.find('img').exists()).toBe(false)
})

it('纯 Emoji 使用突出样式', () => {
  const wrapper = mount(MessageText, { props: { text: '[呲牙][憨笑]' } })
  expect(wrapper.classes()).toContain('message-text--emoji-only')
})

it('消息卡先显示弱化元信息再显示正文', () => {
  const wrapper = mount(MessageCard, { props: { message: textMessage(), showGroup: true } })
  expect(wrapper.get('.message-source').text()).toContain('#13537')
  expect(wrapper.get('.message-sender').text()).toBe('用户 100267')
  expect(wrapper.get('.message-content').text()).toContain('重要告警')
  expect(wrapper.text()).not.toContain('文本')
})
```

在 `MessagePanel.test.ts` 增加：初次载入不高亮；尾部追加一条实时消息时只给新 `msg_id` 增加 `message-card--new`；历史前插和虚拟行重新挂载不得高亮。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/components/MessageText.test.ts src/components/MessageCard.test.ts
```

预期：两个组件不存在。

- [ ] **Step 3：实现 MessageText**

```vue
<script setup lang="ts">
import { computed } from 'vue'
import { isEmojiOnly, tokenizeMessageText } from '../utils/emoji'
const props = defineProps<{ text: string }>()
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
```

- [ ] **Step 4：实现 MessageCard 并接入 MessageBody**

`MessageCard` props 为 `{ message: MessageDto; showGroup: boolean }`。模板顺序固定为：

```vue
<article class="message-card">
  <div v-if="showGroup" class="message-source">
    {{ message.group_name || `群 ${message.group_id}` }} <small>#{{ message.group_id }}</small>
  </div>
  <div class="message-meta">
    <span class="message-sender">用户 {{ message.send_uid }}</span>
    <time :datetime="isoTime">{{ formatMessageTime(message.send_time) }}</time>
  </div>
  <div class="message-content">
    <MessageBody :message="message" />
  </div>
</article>
```

`MessageBody` 的 text 分支改用 `MessageText`；默认不渲染 `.message-kind`，只在未知类型或错误时显示普通用户可理解的提示。

- [ ] **Step 5：替换虚拟列表行模板和样式**

`MessagePanel` 的 `<li>` 内只渲染 `MessageCard`。删除四列 grid；卡片正文至少 15px，元信息 10–11px，颜色对比弱于正文。纯 Emoji 约 28px。卡片 margin、padding 和 border 必须放在 `<li>` 内部 article，避免虚拟元素外边距折叠。

`base.css` 的正文 Emoji 字体回退为 `"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif`。`MessagePanel` 比较上一次尾部 `msg_id`，仅对非初次、非历史前插的新尾消息记录 1.2 秒高亮集合；定时结束后删除 ID，组件卸载时清理 timer。高亮只改变背景色，并在 `prefers-reduced-motion` 下禁用过渡。

- [ ] **Step 6：运行组件测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/components/MessageText.test.ts src/components/MessageCard.test.ts src/components/MessageBody.test.ts src/components/MessagePanel.test.ts
npm run typecheck
```

预期：内容层级、Emoji、安全文本、媒体和现有虚拟列表基本测试通过。

- [ ] **Step 7：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/components/MessageText.vue im-app/ui/src/components/MessageText.test.ts im-app/ui/src/components/MessageCard.vue im-app/ui/src/components/MessageCard.test.ts im-app/ui/src/components/MessageBody.vue im-app/ui/src/components/MessageBody.test.ts im-app/ui/src/components/MessagePanel.vue im-app/ui/src/components/MessagePanel.test.ts im-app/ui/src/styles/console.css
git commit -m "feat: prioritize message content"
```

### Task 5：限制滚动区域并美化滚动条

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/base.css`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.css`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.test.ts`

- [ ] **Step 1：写静态样式失败测试**

在 `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.test.ts` 新建：

```typescript
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const base = readFileSync(new URL('./base.css', import.meta.url), 'utf8')
const consoleCss = readFileSync(new URL('./console.css', import.meta.url), 'utf8')

describe('scroll layout', () => {
  it('禁止整页滚动并限定业务滚动区', () => {
    expect(base).toMatch(/body[\s\S]*overflow:\s*hidden/)
    expect(consoleCss).toMatch(/\.group-list[\s\S]*overflow-y:\s*auto/)
    expect(consoleCss).toMatch(/\.message-viewport[\s\S]*overflow-y:\s*auto/)
  })

  it('同时定义标准和 WebKit 滚动条', () => {
    expect(consoleCss).toContain('scrollbar-color')
    expect(consoleCss).toContain('::-webkit-scrollbar')
  })
})
```

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/styles/console.test.ts
```

预期：body 当前允许滚动且无自定义 scrollbar。

- [ ] **Step 3：实现滚动约束**

`html`、`body`、`#app` 设 `width/height: 100%` 和 `overflow: hidden`。`.operations-shell`、`.workspace`、`.message-panel` 的 grid/flex 子项补 `min-width: 0; min-height: 0`。只有 `.group-list`、`.message-viewport` 和窗口高度不足时的 `.login-console` 使用 `overflow-y: auto`。

对三个允许滚动的选择器统一设置：

```css
scrollbar-width: thin;
scrollbar-color: rgba(132, 149, 149, 0.55) rgba(10, 15, 16, 0.35);
```

同时实现 8px WebKit track/thumb，thumb 使用圆角和透明边框；hover 时提高对比度。不要隐藏滚动条。

- [ ] **Step 4：运行测试和构建**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/styles/console.test.ts
npm run build
```

预期：样式测试和生产构建通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/styles/base.css im-app/ui/src/styles/console.css im-app/ui/src/styles/console.test.ts
git commit -m "style: refine application scrolling"
```

### Task 6：增加窄屏群列表抽屉

**Files:**
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useResponsiveSidebar.ts`
- Create: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/composables/useResponsiveSidebar.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/App.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.css`

- [ ] **Step 1：写响应式状态失败测试**

```typescript
it('窄屏默认关闭群列表并允许切换', () => {
  mockMatchMedia(true)
  const layout = useResponsiveSidebar()
  expect(layout.isNarrow.value).toBe(true)
  expect(layout.sidebarOpen.value).toBe(false)
  layout.toggleSidebar()
  expect(layout.sidebarOpen.value).toBe(true)
  layout.selectGroup()
  expect(layout.sidebarOpen.value).toBe(false)
})
```

另测宽屏默认展开，以及 composable 卸载时移除 `change` listener。

- [ ] **Step 2：运行测试并确认失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useResponsiveSidebar.test.ts
```

预期：composable 不存在。

- [ ] **Step 3：实现并接线**

使用 `window.matchMedia('(max-width: 900px)')`，监听 `change`。窄屏：

- 顶部显示“群列表”按钮；
- 侧栏作为带遮罩的抽屉；
- 选择群或“全部群消息”后自动关闭；
- Escape 和点击遮罩关闭；
- 消息区始终占满剩余空间；
- 时间、群来源和正文保留，其他装饰性信息可隐藏。

抽屉按钮提供 `aria-expanded` 和 `aria-controls`，抽屉关闭时不可获得焦点。

- [ ] **Step 4：运行测试和类型检查**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/composables/useResponsiveSidebar.test.ts src/App.test.ts
npm run typecheck
```

预期：宽窄屏、关闭动作和无障碍属性测试通过。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/composables/useResponsiveSidebar.ts im-app/ui/src/composables/useResponsiveSidebar.test.ts im-app/ui/src/App.vue im-app/ui/src/App.test.ts im-app/ui/src/styles/console.css
git commit -m "feat: add responsive group drawer"
```

### Task 7：校准虚拟列表和历史锚点

**Files:**
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.vue`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/components/MessagePanel.test.ts`
- Modify: `/Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui/src/styles/console.css`

- [ ] **Step 1：增加动态高度回归测试**

测试必须覆盖：

```typescript
it('卡片高度变化后仍保持历史前插锚点', async () => {
  const wrapper = mountMeasuredPanel({ rowHeights: [72, 140, 88] })
  await scrollNearTopAndPrepend(wrapper, olderMessages())
  expect(viewport(wrapper).scrollTop).toBeCloseTo(anchorOffsetBeforeLoad(), 0)
})

it('媒体加载撑高消息时重新测量虚拟行', async () => {
  const wrapper = mountMeasuredPanel({ rowHeights: [72] })
  resizeObservedRow(wrapper, 220)
  await nextTick()
  expect(currentVirtualSize(wrapper)).toBe(220)
})
```

保留现有“初始滚底”“用户离开底部不抢滚”“1000 条 trim 后识别新尾消息”和“旧分页结果不污染新群”测试。

- [ ] **Step 2：运行测试并确认至少一个失败**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/components/MessagePanel.test.ts
```

预期：旧固定行高 mock 或卡片高度场景失败。

- [ ] **Step 3：校准估算与测量**

将 `estimateSize` 改为按内容提供保守估算：

```typescript
function estimateMessageHeight(message: MessageDto | undefined): number {
  if (!message?.decoded_content) return 96
  switch (message.decoded_content.kind) {
    case 'image':
    case 'video': return 220
    case 'audio':
    case 'file': return 112
    case 'text': return Math.min(220, 76 + Math.floor(message.decoded_content.text.length / 48) * 22)
  }
}
```

继续把 `measureElement` 绑定在绝对定位 `<li>`。确保卡片 margin 不在 `<li>` 外形成未测量空间；ResizeObserver 在媒体加载和窗口宽度变化后触发重测。锚点仍优先通过 `msg_id` 找 index，再用真实 `anchorStart + scrollOffset` 恢复。

- [ ] **Step 4：运行消息与全前端测试**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast/im-app/ui
npx vitest run src/components/MessagePanel.test.ts
npm test
npm run typecheck
npm run build
```

预期：所有虚拟滚动测试在默认 5 秒超时内稳定通过，不通过增加全局超时掩盖问题。

- [ ] **Step 5：提交**

```bash
cd /Volumes/TRANSCEND/works/objects/rust/broadcast
git add im-app/ui/src/components/MessagePanel.vue im-app/ui/src/components/MessagePanel.test.ts im-app/ui/src/styles/console.css
git commit -m "test: stabilize message card virtualization"
```

## 完成标准

- “全部群消息”显示最多 5 个监控 groupId，并可展开和收起；
- 消息正文是卡片视觉主体，时间、群来源和发送人弱化；
- 普通文本不显示“文本”，页面不显示 CHANNEL、UID、Rust 等开发文案；
- 原生 Emoji、`[呲牙]`、`[憨笑]`、未知别名和混合文本安全显示；
- 纯 Emoji 使用突出字号且不造成横向滚动；
- 整页不滚动，仅业务列表滚动，滚动条适配深色界面；
- 900px 以下群列表变为可访问的抽屉；
- 虚拟列表的滚底、动态高度和历史锚点测试稳定通过；
- `npm test`、`npm run typecheck` 和 `npm run build` 全部通过。
