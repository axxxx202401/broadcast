// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  DEFAULT_GT4_CAPTCHA_ID,
  loadGt4Script,
  useGt4,
  type Gt4Instance,
} from './useGt4'

/** 构造可手动触发 SDK 回调的实例，用于覆盖异步生命周期边界。 */
function fakeGt4() {
  const handlers: Record<string, () => void> = {}
  const instance: Gt4Instance = {
    onReady: vi.fn((handler) => {
      handlers.ready = handler
      return instance
    }),
    onSuccess: vi.fn((handler) => {
      handlers.success = handler
      return instance
    }),
    onFail: vi.fn((handler) => {
      handlers.fail = handler
      return instance
    }),
    onError: vi.fn((handler) => {
      handlers.error = handler
      return instance
    }),
    onClose: vi.fn((handler) => {
      handlers.close = handler
      return instance
    }),
    showCaptcha: vi.fn(),
    getValidate: vi.fn(() => ({
      lot_number: 'lot',
      captcha_output: 'output',
      pass_token: 'pass',
      gen_time: 'time',
    })),
    reset: vi.fn(),
    destroy: vi.fn(),
  }
  return { instance, handlers }
}

describe('useGt4', () => {
  beforeEach(() => vi.clearAllMocks())

  it('使用服务端配置的 Android 640+ captchaId', () => {
    expect(DEFAULT_GT4_CAPTCHA_ID).toBe('d7b9e5c52c8d9d8b214bc7a4c6db1f4f')
  })

  it('本地脚本失败后回退 CDN，并在全部失败后清缓存以允许重试', async () => {
    delete window.initGeetest4
    const before = document.querySelectorAll('script[src*="gt4.js"]').length
    const first = loadGt4Script()
    let script = document.querySelectorAll<HTMLScriptElement>(
      'script[src*="gt4.js"]',
    ).item(before)
    expect(script.getAttribute('src')).toBe('/vendor/gt4.js')
    script.onerror?.(new Event('error'))
    await Promise.resolve()

    script = document.querySelectorAll<HTMLScriptElement>('script[src*="gt4.js"]').item(before)
    expect(script.src).toBe('https://static.geetest.com/v4/gt4.js')
    script.onerror?.(new Event('error'))
    await expect(first).rejects.toThrow('加载失败')

    const second = loadGt4Script()
    script = document.querySelectorAll<HTMLScriptElement>('script[src*="gt4.js"]').item(before)
    expect(script.getAttribute('src')).toBe('/vendor/gt4.js')
    script.onerror?.(new Event('error'))
    await Promise.resolve()

    script = document.querySelectorAll<HTMLScriptElement>('script[src*="gt4.js"]').item(before)
    expect(second).not.toBe(first)
    const init = vi.fn()
    window.initGeetest4 = init
    script.onload?.(new Event('load'))
    await expect(second).resolves.toBe(init)
  })

  it('仅加载初始化一次，且 ready 前拒绝展示验证码', async () => {
    const fake = fakeGt4()
    const loadScript = vi.fn().mockResolvedValue(
      vi.fn((_options, callback) => callback(fake.instance)),
    )
    let gt4!: ReturnType<typeof useGt4>
    const wrapper = mount(defineComponent({
      setup() {
        gt4 = useGt4({ captchaId: 'public-id', loadScript })
        return () => h('div')
      },
    }))

    expect(gt4.show('13800138000', vi.fn())).toBe(false)
    await flushPromises()
    expect(loadScript).toHaveBeenCalledTimes(1)
    expect(gt4.loading.value).toBe(true)
    expect(gt4.ready.value).toBe(false)

    fake.handlers.ready?.()
    expect(gt4.loading.value).toBe(false)
    expect(gt4.show('13800138000', vi.fn())).toBe(true)
    expect(fake.instance.showCaptcha).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('转换 snake_case 验证字段、冻结账号，并只消费一次成功回调', async () => {
    const fake = fakeGt4()
    const success = vi.fn()
    let gt4!: ReturnType<typeof useGt4>
    const wrapper = mount(defineComponent({
      setup() {
        gt4 = useGt4({
          captchaId: 'public-id',
          init: (_options, callback) => callback(fake.instance),
        })
        return () => h('div')
      },
    }))
    await flushPromises()
    fake.handlers.ready?.()

    gt4.show('old@example.com', success)
    fake.handlers.success?.()
    fake.handlers.success?.()

    expect(success).toHaveBeenCalledTimes(1)
    expect(success).toHaveBeenCalledWith('old@example.com', {
      lotNumber: 'lot',
      captchaOutput: 'output',
      passToken: 'pass',
      genTime: 'time',
    })
    wrapper.unmount()
    expect(fake.instance.destroy).toHaveBeenCalledTimes(1)
  })

  it('覆盖 fail、error、close、reset 与 destroy 生命周期', async () => {
    const fake = fakeGt4()
    let gt4!: ReturnType<typeof useGt4>
    const wrapper = mount(defineComponent({
      setup() {
        gt4 = useGt4({
          captchaId: 'public-id',
          init: (_options, callback) => callback(fake.instance),
        })
        return () => h('div')
      },
    }))
    await flushPromises()
    fake.handlers.ready?.()
    gt4.show('account', vi.fn())

    fake.handlers.fail?.()
    expect(gt4.error.value).toContain('失败')
    fake.handlers.error?.()
    expect(gt4.error.value).toContain('异常')
    fake.handlers.close?.()
    expect(gt4.error.value).toContain('关闭')

    gt4.reset()
    expect(fake.instance.reset).toHaveBeenCalledTimes(1)
    expect(gt4.error.value).toBe('')
    wrapper.unmount()
    expect(fake.instance.destroy).toHaveBeenCalledTimes(1)
  })

  it('销毁旧实例后可以初始化全新实例', async () => {
    const first = fakeGt4()
    const second = fakeGt4()
    const init = vi.fn()
      .mockImplementationOnce((_options, callback) => callback(first.instance))
      .mockImplementationOnce((_options, callback) => callback(second.instance))
    let gt4!: ReturnType<typeof useGt4>
    const wrapper = mount(defineComponent({
      setup() {
        gt4 = useGt4({ captchaId: 'public-id', init })
        return () => h('div')
      },
    }))
    await flushPromises()
    first.handlers.ready?.()
    gt4.destroy()

    const initializing = gt4.initialize()
    second.handlers.ready?.()
    await initializing

    expect(init).toHaveBeenCalledTimes(2)
    expect(first.instance.destroy).toHaveBeenCalledTimes(1)
    expect(gt4.ready.value).toBe(true)
    wrapper.unmount()
    expect(second.instance.destroy).toHaveBeenCalledTimes(1)
  })

  it('销毁时结算仍在进行的初始化', async () => {
    let gt4!: ReturnType<typeof useGt4>
    const wrapper = mount(defineComponent({
      setup() {
        gt4 = useGt4({ captchaId: 'public-id', init: vi.fn() })
        return () => h('div')
      },
    }))
    const initializing = gt4.initialize()

    gt4.destroy()

    expect(await Promise.race([
      initializing,
      new Promise((resolve) => setTimeout(() => resolve('timed-out'), 10)),
    ])).toBe(false)
    wrapper.unmount()
  })
})
