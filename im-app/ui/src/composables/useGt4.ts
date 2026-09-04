import { onUnmounted, ref } from 'vue'

import type { Gt4Fields } from '../types/im'

/** GT4 脚本候选地址，严格按本地资源、官方 CDN 的顺序尝试。加载会向全局文档插入脚本。 */
export const GT4_SCRIPT_URLS = [
  '/vendor/gt4.js',
  'https://static.geetest.com/v4/gt4.js',
] as const

/** GT4 SDK 返回的原始验证字段；字段名遵循 SDK 的 snake_case 约定。 */
export interface Gt4RawValidation {
  /** 本次验证批次号。 */
  lot_number: string
  /** SDK 生成的验证输出。 */
  captcha_output: string
  /** SDK 生成的通过令牌。 */
  pass_token: string
  /** SDK 生成验证结果的时间字段。 */
  gen_time: string
}

/** 当前组合式函数依赖的 GT4 实例最小接口。 */
export interface Gt4Instance {
  /** 注册 SDK 就绪回调，并返回实例以支持链式注册。 */
  onReady(handler: () => void): Gt4Instance
  /** 注册验证成功回调，并返回实例以支持链式注册。 */
  onSuccess(handler: () => void): Gt4Instance
  /** 注册验证失败回调，并返回实例以支持链式注册。 */
  onFail(handler: (event?: unknown) => void): Gt4Instance
  /** 注册 SDK 异常回调，并返回实例以支持链式注册。 */
  onError(handler: (event?: unknown) => void): Gt4Instance
  /** 注册用户关闭验证码回调，并返回实例以支持链式注册。 */
  onClose(handler: () => void): Gt4Instance
  /** 显示绑定模式验证码。 */
  showCaptcha(): void
  /** 读取验证结果；结果尚不可用时返回 false。 */
  getValidate(): Gt4RawValidation | false
  /** 重置当前验证流程，以便重试。 */
  reset(): void
  /** 销毁实例及其 SDK 侧资源。 */
  destroy(): void
}

/** GT4 全局初始化函数签名；初始化结果通过 callback 而非返回 Promise 交付。 */
export type Gt4Init = (
  options: {
    captchaId: string
    product: 'bind'
    language: 'zho'
    protocol: 'https://'
  },
  callback: (instance: Gt4Instance) => void,
) => void

declare global {
  interface Window {
    /** GT4 脚本加载后写入 window 的全局初始化函数。 */
    initGeetest4?: Gt4Init
  }
}

// 全模块共享加载 Promise，避免多个组件重复插入同一个 GT4 脚本。
let scriptPromise: Promise<Gt4Init> | null = null

/**
 * 提取 GT4 SDK 错误事件中的公开诊断字段。
 *
 * SDK 不保证事件结构，因此只读取常见的 code/msg 字段并限制长度；账号、验证结果等
 * 其他字段不会拼入界面错误，避免意外暴露验证材料。
 */
function gt4ErrorDetail(event: unknown): string {
  if (!event || typeof event !== 'object') return ''
  const record = event as Record<string, unknown>
  const values = [
    record.code ?? record.error_code,
    record.msg ?? record.error_message ?? record.desc,
  ]
    .filter((value): value is string | number =>
      typeof value === 'string' || typeof value === 'number')
    .map(value => String(value).slice(0, 160))
  return values.length > 0 ? `（${values.join('：')}）` : ''
}

/** 插入单个脚本，并把脚本事件转换为 Promise；失败时移除对应节点。 */
function appendGt4Script(src: string): Promise<Gt4Init> {
  return new Promise<Gt4Init>((resolve, reject) => {
    const script = document.createElement('script')
    script.src = src
    script.async = true
    script.onload = () => {
      if (window.initGeetest4) resolve(window.initGeetest4)
      else {
        script.remove()
        reject(new Error(`GT4 脚本未提供初始化函数：${src}`))
      }
    }
    script.onerror = () => {
      script.remove()
      reject(new Error(`GT4 脚本加载失败：${src}`))
    }
    document.head.append(script)
  })
}

/**
 * 获取 GT4 全局初始化函数。
 *
 * 已存在全局函数时不会插入脚本；否则先加载本地资源，再回退到 CDN。两者均失败时
 * Promise 会拒绝，并清除共享失败缓存，使后续调用可以重新尝试。成功脚本会保留，
 * 且 SDK 对 `window.initGeetest4` 等全局状态的修改不会在此处撤销。
 */
export function loadGt4Script(): Promise<Gt4Init> {
  if (window.initGeetest4) return Promise.resolve(window.initGeetest4)
  if (scriptPromise) return scriptPromise

  const promise = (async () => {
    let lastError: unknown
    for (const src of GT4_SCRIPT_URLS) {
      try {
        return await appendGt4Script(src)
      } catch (reason) {
        lastError = reason
      }
    }
    const detail = lastError instanceof Error ? `：${lastError.message}` : ''
    throw new Error(`GT4 脚本加载失败，本地资源与 CDN 均不可用${detail}`)
  })()

  scriptPromise = promise
  void promise.catch(() => {
    if (scriptPromise === promise) scriptPromise = null
  })
  return promise
}

/** `useGt4` 的可替换配置，主要用于覆盖 captchaId、初始化器及脚本加载器。 */
export interface UseGt4Options {
  /** 显式 captchaId，主要供测试或嵌入方覆盖构建环境变量。 */
  captchaId?: string
  /** 直接注入初始化函数；提供后不会加载外部脚本。 */
  init?: Gt4Init
  /** 自定义异步脚本加载器。 */
  loadScript?: () => Promise<Gt4Init>
}

/**
 * 管理 GT4 实例从初始化、展示、一次消费到销毁的完整生命周期。
 *
 * `initialize` 的 Promise 只表示 SDK 是否触发 ready；初始化器自身仍采用回调交付实例。
 * SDK 的 fail/close 主要更新可观察错误，error 会结束当前初始化；调用方传入的成功回调
 * 以 fire-and-forget 方式执行，其 Promise 拒绝不会由本组合式函数捕获。卸载时会销毁实例，
 * 但不会删除已加载脚本或恢复 SDK 写入的全局变量。
 *
 * @returns GT4 状态，以及 initialize/show/reset/destroy 四个生命周期操作。
 */
export function useGt4(options: UseGt4Options = {}) {
  const loading = ref(true)
  const ready = ref(false)
  const error = ref('')
  let instance: Gt4Instance | null = null
  let disposed = false
  let generation = 0
  let initialization: Promise<boolean> | null = null
  let settleInitialization: ((ready: boolean) => void) | null = null
  let successConsumed = false
  let pendingSuccess:
    | ((accountSnapshot: string, fields: Gt4Fields) => void | Promise<void>)
    | null = null
  /** 滑块失败或用户关闭时通知调用方结束忙碌态，不得据此写入界面错误。 */
  let pendingDismiss: (() => void) | null = null
  let accountSnapshot = ''

  /**
   * 初始化或复用当前实例；generation 保证 destroy 后迟到的加载结果和 SDK 回调失效。
   * @returns SDK ready 时为 true，失败、销毁或卸载时为 false。
   */
  const initialize = () => {
    if (disposed) return Promise.resolve(false)
    if (ready.value && instance) return Promise.resolve(true)
    if (initialization) return initialization
    loading.value = true
    error.value = ''
    const currentGeneration = ++generation
    initialization = new Promise<boolean>((resolve) => {
      settleInitialization = (isReady) => {
        if (currentGeneration !== generation) return
        loading.value = false
        initialization = null
        settleInitialization = null
        resolve(isReady)
      }
      void (async () => {
        try {
          const captchaId = (options.captchaId
            ?? import.meta.env.VITE_GT4_CAPTCHA_ID
            ?? '').trim()
          if (!captchaId) {
            throw new Error('GT4 配置缺少 VITE_GT4_CAPTCHA_ID')
          }
          const init = options.init ?? await (options.loadScript ?? loadGt4Script)()
          if (disposed || currentGeneration !== generation) {
            settleInitialization?.(false)
            return
          }
          init(
            {
              captchaId,
              product: 'bind',
              language: 'zho',
              protocol: 'https://',
            },
            (created) => {
              if (disposed || currentGeneration !== generation) {
                created.destroy()
                return
              }
              instance = created
              // 将 SDK 回调集中绑定到当前代，避免旧实例在重建后继续影响界面。
              created
                .onReady(() => {
                  if (currentGeneration !== generation) return
                  ready.value = true
                  error.value = ''
                  settleInitialization?.(true)
                })
                .onSuccess(() => {
                  if (currentGeneration !== generation || successConsumed || !pendingSuccess) return
                  const raw = created.getValidate()
                  if (!raw) {
                    error.value = 'GT4 未返回有效验证结果'
                    return
                  }
                  successConsumed = true
                  error.value = ''
                  pendingDismiss = null
                  const callback = pendingSuccess
                  pendingSuccess = null
                  // SDK 的 snake_case 在边界处转换为应用层 camelCase；同一次 show 仅消费一次。
                  void callback(accountSnapshot, {
                    lotNumber: raw.lot_number,
                    captchaOutput: raw.captcha_output,
                    passToken: raw.pass_token,
                    genTime: raw.gen_time,
                  })
                })
                .onFail(() => {
                  if (currentGeneration !== generation) return
                  // 滑块失败由 GT4 控件自身提示，界面不再重复写错误条。
                  error.value = ''
                  const dismiss = pendingDismiss
                  pendingDismiss = null
                  dismiss?.()
                })
                .onError((event) => {
                  if (currentGeneration !== generation) return
                  error.value = `GT4 验证异常，请稍后重试${gt4ErrorDetail(event)}`
                  settleInitialization?.(false)
                })
                .onClose(() => {
                  if (currentGeneration !== generation) return
                  pendingSuccess = null
                  error.value = ''
                  const dismiss = pendingDismiss
                  pendingDismiss = null
                  dismiss?.()
                })
            },
          )
        } catch (reason) {
          if (currentGeneration !== generation) return
          error.value = reason instanceof Error ? reason.message : 'GT4 初始化失败'
          settleInitialization?.(false)
        }
      })()
    })
    return initialization
  }

  /**
   * 展示已就绪的验证码，并冻结账号供成功回调使用。
   * `onDismiss` 仅在滑块失败或用户关闭时调用，供调用方结束忙碌态，不表示应展示错误。
   * @returns 成功触发展示为 true；未就绪、无实例或已卸载为 false。
   */
  const show = (
    snapshot: string,
    onSuccess: (accountSnapshot: string, fields: Gt4Fields) => void | Promise<void>,
    onDismiss?: () => void,
  ) => {
    if (!ready.value || !instance || disposed) return false
    error.value = ''
    accountSnapshot = snapshot
    pendingSuccess = onSuccess
    pendingDismiss = onDismiss ?? null
    successConsumed = false
    instance.showCaptcha()
    return true
  }

  /** 清除一次消费状态、账号快照和错误，并重置当前 SDK 实例以便重试。 */
  const reset = () => {
    pendingSuccess = null
    pendingDismiss = null
    successConsumed = false
    accountSnapshot = ''
    error.value = ''
    instance?.reset()
  }

  /** 销毁实例并结算悬空初始化；递增 generation 使当前代的迟到回调失效。 */
  const destroy = () => {
    settleInitialization?.(false)
    generation += 1
    ready.value = false
    loading.value = false
    pendingSuccess = null
    pendingDismiss = null
    settleInitialization = null
    initialization = null
    error.value = ''
    instance?.destroy()
    instance = null
  }

  void initialize()
  onUnmounted(() => {
    // 组件卸载后永久禁止重新初始化，并释放当前 GT4 实例。
    disposed = true
    destroy()
  })

  return {
    /** 脚本或实例仍在初始化。 */
    loading,
    /** SDK 已触发 ready 且实例可展示。 */
    ready,
    /** 最近一次 GT4 生命周期错误。 */
    error,
    initialize,
    show,
    reset,
    destroy,
  }
}
