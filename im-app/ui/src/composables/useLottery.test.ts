// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DrawItem } from '../services/tauri'
import { useLottery } from './useLottery'

const mocks = vi.hoisted(() => ({
  fetchLotteryHistory: vi.fn(),
  getLotteryConfig: vi.fn(async () => ({
    api_url: 'https://example.test/lottery',
    current_issues: [100],
  })),
  setLotteryConfig: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
}))

vi.mock('../services/tauri', () => ({
  api: {
    fetchLotteryHistory: mocks.fetchLotteryHistory,
    getLotteryConfig: mocks.getLotteryConfig,
    setLotteryConfig: mocks.setLotteryConfig,
  },
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.listen,
}))

describe('useLottery 后端单一轮询', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.clearAllMocks()
    mocks.listen.mockResolvedValue(mocks.unlisten)
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('不创建前端轮询，并使用后端事件更新开奖历史', async () => {
    let lottery!: ReturnType<typeof useLottery>
    const wrapper = mount(defineComponent({
      setup() {
        lottery = useLottery(ref(true))
        return () => h('div')
      },
    }))
    await flushPromises()

    expect(mocks.fetchLotteryHistory).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
    expect(mocks.listen).toHaveBeenCalledWith('lottery_history_updated', expect.any(Function))

    const draw: DrawItem = {
      preDrawIssue: 101,
      preDrawTime: '2026-09-14 13:00:00',
      preDrawCode: '1,3,5',
      sumNum: 9,
      sumBigSmall: -1,
      sumSingleDouble: 1,
    }
    const listener = mocks.listen.mock.calls[0]?.[1] as
      | ((event: { payload: DrawItem[] }) => void)
      | undefined
    listener?.({ payload: [draw] })

    expect(lottery.drawHistory.value).toEqual([draw])
    expect(lottery.config.value.current_issues).toEqual([101])

    wrapper.unmount()
    await flushPromises()
    expect(mocks.unlisten).toHaveBeenCalledOnce()
  })
})
