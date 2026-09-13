// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'

import LotteryPanel from './LotteryPanel.vue'

const { setLotteryTemplate } = vi.hoisted(() => ({
  setLotteryTemplate: vi.fn(),
}))

vi.mock('../services/tauri', () => ({
  api: {
    getLotteryTemplate: vi.fn(async () => ({ template: '第${preDrawIssue}期', enabled: true })),
    getAppRuntimeConfig: vi.fn(async () => ({
      persist_received_messages: true,
      match_lottery_messages: true,
    })),
    setLotteryTemplate,
  },
}))

describe('LotteryPanel 模板编辑器', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    document.body.innerHTML = ''
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  function mountPanel() {
    return mount(LotteryPanel, {
      attachTo: document.body,
      props: {
        lottery: {
          config: ref({ api_url: '', current_issues: [] }),
          drawHistory: ref([]),
          loading: ref(false),
          error: ref(''),
          loadConfig: vi.fn(async () => {}),
          saveConfig: vi.fn(async () => {}),
          fetchHistory: vi.fn(async () => {}),
        },
      },
    })
  }

  it('在独立宽屏对话框中编辑模板并支持 Escape 取消', async () => {
    const wrapper = mountPanel()
    await flushPromises()

    await wrapper.get('[data-test="edit-lottery-template"]').trigger('click')
    const dialog = document.body.querySelector<HTMLElement>('[data-test="lottery-template-dialog"]')
    const textarea = document.body.querySelector<HTMLTextAreaElement>(
      '[data-test="lottery-template-textarea"]',
    )

    expect(dialog?.getAttribute('role')).toBe('dialog')
    expect(dialog?.getAttribute('aria-modal')).toBe('true')
    expect(textarea?.value).toBe('第${preDrawIssue}期')

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()

    expect(document.body.querySelector('[data-test="lottery-template-dialog"]')).toBeNull()
  })

  it('自动广播使用可访问的滑动开关', async () => {
    const wrapper = mountPanel()
    await flushPromises()

    const toggle = wrapper.get('[data-test="lottery-broadcast-toggle"]')
    expect(toggle.attributes('role')).toBe('switch')
    expect(toggle.classes()).toContain('toggle-input')

    await toggle.setValue(false)
    expect(setLotteryTemplate).toHaveBeenCalledWith('第${preDrawIssue}期', false)
  })
})
