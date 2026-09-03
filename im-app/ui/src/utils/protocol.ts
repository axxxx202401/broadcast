import type { ConnectionStatus, Gt4Fields } from '../types/im'

/**
 * 从第三方回调中只提取后端 `Gt4Dto` 确认的四个 camelCase 字段。
 * 各值先转为字符串并去除首尾空白；缺失值归一化为空字符串，其他回调字段不会转发。
 */
export function createGt4Payload(input: Record<string, unknown>): Gt4Fields {
  return {
    lotNumber: String(input.lotNumber ?? '').trim(),
    captchaOutput: String(input.captchaOutput ?? '').trim(),
    passToken: String(input.passToken ?? '').trim(),
    genTime: String(input.genTime ?? '').trim(),
  }
}

/** 仅当四个已归一化的 GT4 字段都为非空字符串时，才判定挑战材料完整。 */
export function hasCompleteGt4Payload(fields: Gt4Fields): boolean {
  return Object.values(fields).every(Boolean)
}

/**
 * 把后端事件值限制为前端三态。
 * `connected`、`connecting` 原样保留；显式断开及任何未知、缺失或类型异常值均降级为 `disconnected`。
 */
export function normalizeConnectionStatus(value: unknown): ConnectionStatus {
  if (value === 'connected' || value === 'connecting') return value
  return 'disconnected'
}

/**
 * 将未知拒绝值整理为可展示文本。
 * `Error` 和非空字符串直接取消息；结构化业务错误按“标题 · 业务码 · 消息 · 参数 · data JSON”
 * 跳过缺失项后拼接，`other` 错误取 `message`。无法识别的值返回统一后端日志提示，但
 * `kind: 'business'` 对象缺少所有可拼接字段时会直接得到空字符串，不会进入该提示。
 */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string' && error.trim()) return error
  if (error && typeof error === 'object') {
    const commandError = error as Record<string, unknown>
    if (commandError.kind === 'business') {
      return [
        commandError.title,
        commandError.code,
        commandError.msg,
        Array.isArray(commandError.params) ? commandError.params.join(', ') : undefined,
        commandError.data === undefined ? undefined : JSON.stringify(commandError.data),
      ].filter((part) => part !== undefined && part !== '').join(' · ')
    }
    if (commandError.kind === 'other' && typeof commandError.message === 'string') {
      return commandError.message
    }
  }
  return '操作失败，请查看后端日志'
}

/**
 * 给登录/二次验证界面用的错误文案：只保留标题与消息，去掉业务码和诊断 JSON。
 * 完整诊断仍写入 `console.debug`，便于对照 `errorMessage`。
 * 普通字符串（如 challenge notice）同样剥离 `311xxxx` 业务码。
 */
export function userFacingError(error: unknown): string {
  // 普通提示字符串不刷 debug；结构化错误才留下完整诊断。
  if (typeof error !== 'string' && typeof console !== 'undefined' && typeof console.debug === 'function') {
    console.debug('[auth-error]', errorMessage(error), error)
  }
  let text = ''
  if (error && typeof error === 'object' && !(error instanceof Error)) {
    const commandError = error as Record<string, unknown>
    if (commandError.kind === 'business') {
      text = [commandError.title, commandError.msg]
        .filter((part) => typeof part === 'string' && part.trim())
        .join(' · ')
    } else if (commandError.kind === 'other' && typeof commandError.message === 'string') {
      text = commandError.message
    }
  } else if (error instanceof Error) {
    text = error.message
  } else if (typeof error === 'string') {
    text = error
  }
  text = text
    .replace(/\b311\d+\b/g, '')
    .replace(/\s*·\s*/g, ' · ')
    .replace(/^(?: · )+|(?: · )+$/g, '')
    .trim()
  return text || '验证失败，请重试'
}
