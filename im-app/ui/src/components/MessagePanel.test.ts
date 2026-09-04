// @vitest-environment jsdom

import { mount, type VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { GroupDto, MessageDto } from '../types/im'
import { MessageIndex } from '../utils/message'
import MessagePanel from './MessagePanel.vue'
import MonitoredGroupSummary from './MonitoredGroupSummary.vue'

vi.mock('../services/tauri', () => ({
  api: { downloadMessageAttachment: vi.fn() },
}))

const VIEWPORT_HEIGHT = 320
const ROW_HEIGHT = 56
const resizeObservers: ResizeObserverMock[] = []
const rectCalls = new Map<Element, number>()
/** 按 `msg_id` 覆盖虚拟行高度；未登记的行回退到 `ROW_HEIGHT`。 */
const rowHeightByMsgId = new Map<string, number>()
/** 当前面板消息快照，供测量 mock 把 `data-index` 解析成稳定的 `msg_id`。 */
let mountedMessages: MessageDto[] = []
/** 发起历史前插时根据已测行高算出的目标 `scrollTop`，供锚点断言读取。 */
let capturedAnchorOffset = 0
/** `attachTo: document.body` 的面板，供 `afterEach` 卸掉，让行节点 `isConnected` 以便重测。 */
let attachedPanel: VueWrapper | null = null
const originalOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')

class ResizeObserverMock {
  private readonly callback: ResizeObserverCallback
  readonly observed = new Set<Element>()

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
    resizeObservers.push(this)
  }

  observe(element: Element) {
    this.observed.add(element)
    // 视口尺寸必须立即可用；虚拟行则模拟浏览器 ResizeObserver 的异步批处理，
    // 由测试显式触发媒体尺寸变化，避免同步递归测量制造非浏览器行为。
    if (element.classList.contains('message-viewport')) this.notify(element)
  }

  unobserve() {}
  disconnect() {}

  notify(element: Element) {
    this.callback([{
      target: element,
      contentRect: element.getBoundingClientRect(),
      borderBoxSize: [{
        inlineSize: element.getBoundingClientRect().width,
        blockSize: element.getBoundingClientRect().height,
      }],
    } as unknown as ResizeObserverEntry], this as unknown as ResizeObserver)
  }
}

function makeMessage(index: number): MessageDto {
  return {
    msg_id: String(index + 1),
    group_id: '20',
    group_name: '运维群',
    send_uid: '3',
    msg_type: 0,
    content_b64: '',
    decoded_content: { kind: 'text', text: `消息 ${index + 1}` },
    decode_error: null,
    send_time: index + 1,
    content_md5: '',
    stored_at: null,
    matched: 0,
  }
}

/**
 * 读取元素对应虚拟行的目标高度。
 * 视口固定为 `VIEWPORT_HEIGHT`；行优先按 `msg_id` 查表，避免前插后 `data-index` 错位。
 * 只用 `[data-index]`，不用 `.message-log > li`：jsdom 在节点尚未挂到 `ol` 时对子组合选择器会匹配失败。
 */
function resolveRowHeight(element: HTMLElement): number {
  if (element.classList.contains('message-viewport')) return VIEWPORT_HEIGHT
  const indexed = element.hasAttribute('data-index')
    ? element
    : element.closest('[data-index]') as HTMLElement | null
  if (!indexed) return ROW_HEIGHT
  const index = Number(indexed.getAttribute('data-index'))
  const msgId = Number.isFinite(index) ? mountedMessages[index]?.msg_id : undefined
  if (msgId && rowHeightByMsgId.has(msgId)) return rowHeightByMsgId.get(msgId)!
  return ROW_HEIGHT
}

async function settleVirtualizer() {
  await nextTick()
  await nextTick()
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

/** 挂载可按行指定高度的消息面板，并同步测量 mock 使用的消息快照。 */
function mountMeasuredPanel(options: { rowHeights: number[] }) {
  const messages = options.rowHeights.map((_, index) => makeMessage(index))
  options.rowHeights.forEach((height, index) => {
    rowHeightByMsgId.set(messages[index].msg_id, height)
  })
  mountedMessages = messages
  attachedPanel?.unmount()
  attachedPanel = mount(MessagePanel, {
    // 挂到 document，TanStack 的 ResizeObserver 回调才认为行 `isConnected` 并接受重测。
    attachTo: document.body,
    props: {
      group: null,
      loading: false,
      loadingOlder: false,
      olderRequestToken: 3,
      hasOlder: true,
      messages,
    },
  })
  return attachedPanel
}

function viewport(wrapper: VueWrapper): HTMLElement {
  return wrapper.get('.message-viewport').element as HTMLElement
}

/** 更早一页的稳定测试数据；`msg_id` 不与 `mountMeasuredPanel` 的初始 1..n 冲突。 */
function olderMessages(): MessageDto[] {
  return Array.from({ length: 4 }, (_, index) => makeMessage(index + 50))
}

function anchorOffsetBeforeLoad(): number {
  return capturedAnchorOffset
}

/** 当前虚拟列表总高度，取自占位 `ol` 的行内 style。 */
function currentVirtualSize(wrapper: VueWrapper): number {
  return Number.parseFloat((wrapper.get('.message-log').element as HTMLElement).style.height)
}

/**
 * 把指定行的测量高度改成新媒体尺寸，并通知已观察该节点的 ResizeObserver。
 * 程序化 `scrollTo` 会让虚拟列表处于 `isScrolling`，`measureElement` 会跳过写入，
 * 因此同时调用 `resizeItem` 把新媒体高度落入测量缓存。
 */
function resizeObservedRow(wrapper: VueWrapper, height: number) {
  const row = wrapper.get('.message-log > li').element as HTMLElement
  const index = Number(row.getAttribute('data-index'))
  const msgId = mountedMessages[index]?.msg_id
  if (msgId) rowHeightByMsgId.set(msgId, height)

  const observer = resizeObservers.find(candidate => candidate.observed.has(row))
  observer?.notify(row)

  const virtualizer = (wrapper.vm as {
    virtualizer?: { resizeItem: (itemIndex: number, size: number) => void }
  }).virtualizer
  if (Number.isFinite(index)) virtualizer?.resizeItem(index, height)
}

/**
 * 滚到顶部阈值内触发历史请求，再前插更早消息。
 * 目标锚点 = 前插行实测高度之和 + 请求前的 `scrollTop`，与实现用 `msg_id` 恢复的约定一致。
 */
async function scrollNearTopAndPrepend(wrapper: VueWrapper, older: MessageDto[]) {
  await settleVirtualizer()
  const element = viewport(wrapper)
  Object.defineProperties(element, {
    clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
    scrollHeight: { configurable: true, value: 2100 },
  })
  element.scrollTop = 40
  await wrapper.get('.message-viewport').trigger('scroll')
  await wrapper.setProps({ loadingOlder: true })

  const prependedHeight = older.reduce(
    (sum, message) => sum + (rowHeightByMsgId.get(message.msg_id) ?? ROW_HEIGHT),
    0,
  )
  capturedAnchorOffset = prependedHeight + element.scrollTop

  const current = (wrapper.props() as { messages: MessageDto[] }).messages
  mountedMessages = [...older, ...current]
  await wrapper.setProps({
    messages: mountedMessages,
    loadingOlder: false,
  })
  await settleVirtualizer()
}

describe('MessagePanel', () => {
  beforeEach(() => {
    resizeObservers.length = 0
    rectCalls.clear()
    rowHeightByMsgId.clear()
    mountedMessages = []
    capturedAnchorOffset = 0
    vi.stubGlobal('ResizeObserver', ResizeObserverMock)
    // `offsetHeight` 供 TanStack 首次 `measureElement`（无 ResizeObserver entry）读取真实行高。
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get(this: HTMLElement) {
        return resolveRowHeight(this)
      },
    })
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      rectCalls.set(this, (rectCalls.get(this) ?? 0) + 1)
      const height = resolveRowHeight(this)
      return {
        width: 900,
        height,
        top: 0,
        right: 900,
        bottom: height,
        left: 0,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      }
    })
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', {
      configurable: true,
      writable: true,
      value: vi.fn(function (this: HTMLElement, options: ScrollToOptions) {
        if (typeof options.top === 'number') {
          this.scrollTop = options.top
          this.dispatchEvent(new Event('scroll'))
        }
      }),
    })
  })

  afterEach(() => {
    attachedPanel?.unmount()
    attachedPanel = null
    if (originalOffsetHeight) {
      Object.defineProperty(HTMLElement.prototype, 'offsetHeight', originalOffsetHeight)
    }
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('10000条逻辑输入裁剪为1000条后只挂载可视窗口附近的虚拟行', async () => {
    const index = new MessageIndex()
    index.merge(Array.from({ length: 10_000 }, (_, messageIndex) => makeMessage(messageIndex)))
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        messages: index.snapshot(),
      },
    })

    await settleVirtualizer()

    const rows = wrapper.findAll('.message-log > li')
    expect(rows.length).toBeGreaterThan(0)
    expect(rows.length).toBeLessThan(100)
    expect(wrapper.text()).toContain('1000 条消息')
  })

  it('虚拟行带索引、位移和动态尺寸测量接入', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        messages: Array.from({ length: 30 }, (_, index) => makeMessage(index)),
      },
    })

    await settleVirtualizer()

    const row = wrapper.find('.message-log > li')
    expect(row.attributes('data-index')).toMatch(/^\d+$/)
    expect(row.attributes('style')).toContain('transform: translateY(')

    const rowElement = row.element as HTMLElement
    const measurementsBeforeResize = rectCalls.get(rowElement) ?? 0
    const observer = resizeObservers.find(candidate => candidate.observed.has(rowElement))
    observer?.notify(rowElement)

    expect(observer).toBeDefined()
    expect(rectCalls.get(rowElement)).toBeGreaterThan(measurementsBeforeResize)
  })

  it('新消息到达前距底部不超过阈值时无动画滚到底部', async () => {
    const messages = Array.from({ length: 20 }, (_, index) => makeMessage(index))
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport').element as HTMLElement
    Object.defineProperties(viewport, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 1200 },
    })
    viewport.scrollTop = 820
    const scrollSpy = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollSpy.mockClear()

    await wrapper.setProps({ messages: [...messages, makeMessage(20)] })
    await settleVirtualizer()

    expect(scrollSpy).toHaveBeenCalledWith(expect.objectContaining({ behavior: 'auto' }))
  })

  it('用户远离底部阅读旧消息时不抢滚动', async () => {
    const messages = Array.from({ length: 20 }, (_, index) => makeMessage(index))
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport').element as HTMLElement
    Object.defineProperties(viewport, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 1200 },
    })
    viewport.scrollTop = 200
    const scrollSpy = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollSpy.mockClear()

    await wrapper.setProps({ messages: [...messages, makeMessage(20)] })
    await settleVirtualizer()

    expect(scrollSpy).not.toHaveBeenCalled()
  })

  it('满1000条实时尾部更新且首尾同时变化时近底用户继续自动跟随', async () => {
    const messages = Array.from({ length: 1000 }, (_, index) => makeMessage(index))
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport').element as HTMLElement
    Object.defineProperties(viewport, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 70_000 },
    })
    viewport.scrollTop = 69_650
    const scrollSpy = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollSpy.mockImplementation(function (
      this: HTMLElement,
      optionsOrX?: ScrollToOptions | number,
      y?: number,
    ) {
      if (typeof optionsOrX === 'object' && typeof optionsOrX.top === 'number') {
        this.scrollTop = optionsOrX.top
      } else if (typeof optionsOrX === 'number' && typeof y === 'number') {
        this.scrollTop = y
      }
    })
    scrollSpy.mockClear()

    await wrapper.setProps({ messages: [...messages.slice(1), makeMessage(1000)] })
    await settleVirtualizer()

    expect(scrollSpy).toHaveBeenCalledWith(expect.objectContaining({ behavior: 'auto' }))
  })

  it('满1000条实时尾部更新时远离底部仍不抢滚动', async () => {
    const messages = Array.from({ length: 1000 }, (_, index) => makeMessage(index))
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport').element as HTMLElement
    Object.defineProperties(viewport, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 70_000 },
    })
    viewport.scrollTop = 20_000
    const scrollSpy = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollSpy.mockImplementation(function (
      this: HTMLElement,
      optionsOrX?: ScrollToOptions | number,
      y?: number,
    ) {
      if (typeof optionsOrX === 'object' && typeof optionsOrX.top === 'number') {
        this.scrollTop = optionsOrX.top
      } else if (typeof optionsOrX === 'number' && typeof y === 'number') {
        this.scrollTop = y
      }
    })
    scrollSpy.mockClear()

    await wrapper.setProps({ messages: [...messages.slice(1), makeMessage(1000)] })
    await settleVirtualizer()

    expect(scrollSpy).not.toHaveBeenCalledWith(expect.objectContaining({ behavior: 'auto' }))
  })

  it('接近顶部时只触发一次更早页请求并受加载状态门禁', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        loadingOlder: false,
        olderRequestToken: 6,
        hasOlder: true,
        messages: Array.from({ length: 20 }, (_, index) => makeMessage(index)),
      },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport')
    ;(viewport.element as HTMLElement).scrollTop = 40
    await viewport.trigger('scroll')
    await viewport.trigger('scroll')

    expect(wrapper.emitted('load-older')).toHaveLength(1)
    await wrapper.setProps({ loadingOlder: true })
    await viewport.trigger('scroll')
    expect(wrapper.emitted('load-older')).toHaveLength(1)
  })

  it('前插历史消息后维持锚点且不执行自动滚底', async () => {
    const messages = Array.from({ length: 20 }, (_, index) => makeMessage(index + 20))
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        loadingOlder: false,
        olderRequestToken: 6,
        hasOlder: true,
        messages,
      },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport')
    const element = viewport.element as HTMLElement
    Object.defineProperties(element, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 2100 },
    })
    element.scrollTop = 40
    await viewport.trigger('scroll')
    await wrapper.setProps({ loadingOlder: true })
    const scrollSpy = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollSpy.mockClear()

    await wrapper.setProps({
      messages: [
        ...Array.from({ length: 10 }, (_, index) => makeMessage(index + 10)),
        ...messages,
      ],
      loadingOlder: false,
    })
    await settleVirtualizer()

    expect(scrollSpy).toHaveBeenCalledWith(expect.objectContaining({ behavior: 'auto' }))
    const scrollOptions = scrollSpy.mock.calls as unknown as Array<[ScrollToOptions]>
    expect(
      scrollOptions.some(([options]) =>
        typeof options.top === 'number' && options.top > 40,
      ),
    ).toBe(true)
  })

  it('前插与尾裁后总数仍为1000时按消息ID恢复锚点且不滚底', async () => {
    const messages = Array.from({ length: 1000 }, (_, index) => makeMessage(index + 200))
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        loadingOlder: false,
        olderRequestToken: 7,
        hasOlder: true,
        messages,
      },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport')
    const element = viewport.element as HTMLElement
    Object.defineProperties(element, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 70_000 },
    })
    element.scrollTop = 40
    await viewport.trigger('scroll')
    await wrapper.setProps({ loadingOlder: true })
    const scrollSpy = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollSpy.mockClear()

    await wrapper.setProps({
      messages: [
        ...Array.from({ length: 200 }, (_, index) => makeMessage(index)),
        ...messages.slice(0, 800),
      ],
      loadingOlder: false,
    })
    await settleVirtualizer()

    const scrollOptions = scrollSpy.mock.calls as unknown as Array<[ScrollToOptions]>
    const automaticOffsets = scrollOptions
      .map(([options]) => options)
      .filter((options) => options.behavior === 'auto')
      .map(({ top }) => top)
      .filter((top): top is number => typeof top === 'number')
    expect(automaticOffsets.length).toBeGreaterThan(0)
    expect(Math.max(...automaticOffsets)).toBeGreaterThan(40)
    expect(Math.max(...automaticOffsets)).toBeLessThan(30_000)
    expect(wrapper.emitted('older-settled')).toEqual([[7]])
  })

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

  it('历史失败或无新增时也在loadingOlder结束后发出当前轮握手', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        loadingOlder: false,
        olderRequestToken: 9,
        hasOlder: true,
        messages: [makeMessage(0)],
      },
    })
    await settleVirtualizer()

    await wrapper.setProps({ loadingOlder: true })
    await wrapper.setProps({ loadingOlder: false })
    await settleVirtualizer()

    expect(wrapper.emitted('older-settled')).toEqual([[9]])
  })

  it('没有更早消息时显示到达最早提示且不再触发请求', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        loadingOlder: false,
        hasOlder: false,
        messages: [makeMessage(0)],
      },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport')
    ;(viewport.element as HTMLElement).scrollTop = 0
    await viewport.trigger('scroll')

    expect(wrapper.text()).toContain('已到最早消息')
    expect(wrapper.emitted('load-older')).toBeUndefined()
  })

  it('加载态与空态不创建虚拟列表节点', async () => {
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: true, messages: [makeMessage(0)] },
    })
    await settleVirtualizer()
    expect(wrapper.find('.message-log').exists()).toBe(false)

    await wrapper.setProps({ loading: false, messages: [] })
    await settleVirtualizer()
    expect(wrapper.find('.message-log').exists()).toBe(false)
    expect(wrapper.text()).toContain('暂无已存储消息')
    expect(wrapper.text()).toContain('选择需要监控的群后，新消息会显示在这里')
    expect(wrapper.text()).not.toContain('正文和附件由 Rust 解密')
  })

  // 标题区只在全部群消息挂载汇总，并把调用方传入的监控 ID 原样交给子组件。
  it('全部群消息标题下展示监控群汇总', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        messages: [],
        monitoredGroupIds: ['101', '202'],
      },
    })

    expect(wrapper.text()).toContain('全部群消息')
    expect(wrapper.getComponent(MonitoredGroupSummary).props('groupIds')).toEqual(['101', '202'])
    expect(wrapper.text()).toContain('#101')
    expect(wrapper.text()).toContain('#202')
  })

  it('全部群消息未传监控群时显示尚未选择', async () => {
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages: [] },
    })

    expect(wrapper.text()).toContain('尚未选择监控群')
  })

  it('单群消息不展示监控群汇总', async () => {
    const group: GroupDto = {
      group_id: '101',
      name: '运维群',
      pic: '',
      host_id: null,
      member_count: 1,
      created_at: 0,
      monitored: 1,
      updated_at: 0,
    }
    const wrapper = mount(MessagePanel, {
      props: {
        group,
        loading: false,
        messages: [],
        monitoredGroupIds: ['101', '202'],
      },
    })

    expect(wrapper.text()).toContain('运维群')
    expect(wrapper.findComponent(MonitoredGroupSummary).exists()).toBe(false)
    expect(wrapper.text()).not.toContain('#202')
  })

  it('全部消息模式显示每条消息所属群组', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        messages: [{ ...makeMessage(0), decoded_content: { kind: 'text', text: '告警恢复' } }],
      },
    })
    await settleVirtualizer()

    expect(wrapper.text()).toContain('全部群消息')
    expect(wrapper.text()).toContain('运维群')
    expect(wrapper.text()).toContain('#20')
    expect(wrapper.text()).toContain('告警恢复')
  })

  // 首屏或 loading 结束后的第一批消息都视为初次载入，不得闪高亮。
  it('初次载入消息不高亮', async () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        messages: Array.from({ length: 8 }, (_, index) => makeMessage(index)),
      },
    })
    await settleVirtualizer()

    expect(wrapper.find('.message-card--new').exists()).toBe(false)
  })

  // 仅新尾 msg_id 进入高亮集合；已在窗口内的旧消息即使同批重渲染也不得带 --new。
  it('尾部追加一条实时消息时只给新 msg_id 增加高亮', async () => {
    const messages = Array.from({ length: 8 }, (_, index) => makeMessage(index))
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages },
    })
    await settleVirtualizer()

    await wrapper.setProps({ messages: [...messages, makeMessage(8)] })
    await settleVirtualizer()

    const highlighted = wrapper.findAll('.message-card--new')
    expect(highlighted).toHaveLength(1)
    expect(highlighted[0].text()).toContain('消息 9')
    expect(
      wrapper.findAll('.message-card').some((card) => (
        card.text().includes('消息 8') && !card.classes().includes('message-card--new')
      )),
    ).toBe(true)
  })

  // 历史前插由锚点 watcher 独占；即使可视行换成更早消息，也不得当作实时新尾。
  it('历史前插不得高亮', async () => {
    const messages = Array.from({ length: 20 }, (_, index) => makeMessage(index + 20))
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        loadingOlder: false,
        olderRequestToken: 6,
        hasOlder: true,
        messages,
      },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport')
    const element = viewport.element as HTMLElement
    Object.defineProperties(element, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 2100 },
    })
    element.scrollTop = 40
    await viewport.trigger('scroll')
    await wrapper.setProps({ loadingOlder: true })
    await wrapper.setProps({
      messages: [
        ...Array.from({ length: 10 }, (_, index) => makeMessage(index + 10)),
        ...messages,
      ],
      loadingOlder: false,
    })
    await settleVirtualizer()

    expect(wrapper.find('.message-card--new').exists()).toBe(false)
  })

  // 虚拟窗口回收后重新挂载已出现过的行，只复用原卡片，不得重新打上新消息高亮。
  it('虚拟行重新挂载不得高亮', async () => {
    const messages = Array.from({ length: 40 }, (_, index) => makeMessage(index))
    const wrapper = mount(MessagePanel, {
      props: { group: null, loading: false, messages },
    })
    await settleVirtualizer()

    const viewport = wrapper.get('.message-viewport')
    const element = viewport.element as HTMLElement
    Object.defineProperties(element, {
      clientHeight: { configurable: true, value: VIEWPORT_HEIGHT },
      scrollHeight: { configurable: true, value: 2800 },
    })
    element.scrollTop = 0
    await viewport.trigger('scroll')
    await settleVirtualizer()
    expect(wrapper.find('.message-card--new').exists()).toBe(false)

    element.scrollTop = 2000
    await viewport.trigger('scroll')
    await settleVirtualizer()
    expect(wrapper.find('.message-card--new').exists()).toBe(false)
  })
})

