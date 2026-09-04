// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useResponsiveSidebar } from './useResponsiveSidebar'

/**
 * 用可控 `MediaQueryList` 替身模拟 `(max-width: 900px)`。
 * `matches` 为真表示窄屏；监听器由 composable 注册后可在卸载测试中断言移除。
 */
function mockMatchMedia(matches: boolean) {
  const media = {
    matches,
    media: '(max-width: 900px)',
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }
  vi.stubGlobal('matchMedia', vi.fn((query: string) => {
    expect(query).toBe('(max-width: 900px)')
    return media
  }))
  return media
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('useResponsiveSidebar', () => {
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

  it('宽屏默认展开群列表', () => {
    mockMatchMedia(false)
    const layout = useResponsiveSidebar()
    expect(layout.isNarrow.value).toBe(false)
    expect(layout.sidebarOpen.value).toBe(true)
    layout.selectGroup()
    expect(layout.sidebarOpen.value).toBe(true)
  })

  it('卸载时移除 matchMedia 的 change 监听', () => {
    const media = mockMatchMedia(false)
    const wrapper = mount(defineComponent({
      setup() {
        useResponsiveSidebar()
        return () => h('div')
      },
    }))

    expect(media.addEventListener).toHaveBeenCalledWith('change', expect.any(Function))
    const listener = media.addEventListener.mock.calls.find(([type]) => type === 'change')?.[1]
    expect(listener).toEqual(expect.any(Function))

    wrapper.unmount()

    expect(media.removeEventListener).toHaveBeenCalledWith('change', listener)
  })
})
