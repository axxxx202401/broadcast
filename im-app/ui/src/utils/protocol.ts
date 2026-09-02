import type { ConnectionStatus, Gt4Fields } from '../types/im'

export function createGt4Payload(input: Record<string, unknown>): Gt4Fields {
  return {
    lotNumber: String(input.lotNumber ?? '').trim(),
    captchaOutput: String(input.captchaOutput ?? '').trim(),
    passToken: String(input.passToken ?? '').trim(),
    genTime: String(input.genTime ?? '').trim(),
  }
}

export function hasCompleteGt4Payload(fields: Gt4Fields): boolean {
  return Object.values(fields).every(Boolean)
}

export function normalizeConnectionStatus(value: unknown): ConnectionStatus {
  if (value === 'connected' || value === 'connecting') return value
  return 'disconnected'
}

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
