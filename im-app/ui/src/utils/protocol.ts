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
 * 跳过缺失项后拼接，`other` 错误取 `message`，无法识别时返回统一后端日志提示。
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
