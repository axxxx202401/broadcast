import { onUnmounted, ref } from 'vue'

import type { Gt4Fields } from '../types/im'

export const DEFAULT_GT4_CAPTCHA_ID = 'd7b9e5c52c8d9d8b214bc7a4c6db1f4f'
export const GT4_SCRIPT_URLS = [
  '/vendor/gt4.js',
  'https://static.geetest.com/v4/gt4.js',
] as const

export interface Gt4RawValidation {
  lot_number: string
  captcha_output: string
  pass_token: string
  gen_time: string
}

export interface Gt4Instance {
  onReady(handler: () => void): Gt4Instance
  onSuccess(handler: () => void): Gt4Instance
  onFail(handler: (event?: unknown) => void): Gt4Instance
  onError(handler: (event?: unknown) => void): Gt4Instance
  onClose(handler: () => void): Gt4Instance
  showCaptcha(): void
  getValidate(): Gt4RawValidation | false
  reset(): void
  destroy(): void
}

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
    initGeetest4?: Gt4Init
  }
}

let scriptPromise: Promise<Gt4Init> | null = null

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

export interface UseGt4Options {
  captchaId?: string
  init?: Gt4Init
  loadScript?: () => Promise<Gt4Init>
}

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
  let accountSnapshot = ''

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
          const init = options.init ?? await (options.loadScript ?? loadGt4Script)()
          if (disposed || currentGeneration !== generation) {
            settleInitialization?.(false)
            return
          }
          init(
            {
              captchaId: options.captchaId
                || import.meta.env.VITE_GT4_CAPTCHA_ID
                || DEFAULT_GT4_CAPTCHA_ID,
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
                  const callback = pendingSuccess
                  pendingSuccess = null
                  void callback(accountSnapshot, {
                    lotNumber: raw.lot_number,
                    captchaOutput: raw.captcha_output,
                    passToken: raw.pass_token,
                    genTime: raw.gen_time,
                  })
                })
                .onFail(() => {
                  if (currentGeneration !== generation) return
                  error.value = 'GT4 验证失败，请重试'
                })
                .onError(() => {
                  if (currentGeneration !== generation) return
                  error.value = 'GT4 验证异常，请稍后重试'
                  settleInitialization?.(false)
                })
                .onClose(() => {
                  if (currentGeneration !== generation) return
                  pendingSuccess = null
                  error.value = 'GT4 验证已关闭'
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

  const show = (
    snapshot: string,
    onSuccess: (accountSnapshot: string, fields: Gt4Fields) => void | Promise<void>,
  ) => {
    if (!ready.value || !instance || disposed) return false
    error.value = ''
    accountSnapshot = snapshot
    pendingSuccess = onSuccess
    successConsumed = false
    instance.showCaptcha()
    return true
  }

  const reset = () => {
    pendingSuccess = null
    successConsumed = false
    accountSnapshot = ''
    error.value = ''
    instance?.reset()
  }

  const destroy = () => {
    settleInitialization?.(false)
    generation += 1
    ready.value = false
    loading.value = false
    pendingSuccess = null
    settleInitialization = null
    initialization = null
    instance?.destroy()
    instance = null
  }

  void initialize()
  onUnmounted(() => {
    disposed = true
    destroy()
  })

  return { loading, ready, error, initialize, show, reset, destroy }
}
