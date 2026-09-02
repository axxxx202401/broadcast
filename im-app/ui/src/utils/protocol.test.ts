import { describe, expect, it } from 'vitest'

import { createGt4Payload, errorMessage, normalizeConnectionStatus } from './protocol'

describe('GT4 参数归一化', () => {
  it('仅向后端转发协议确认的四个 GT4 字段', () => {
    expect(
      createGt4Payload({
        lotNumber: 'lot',
        captchaOutput: 'output',
        passToken: 'pass',
        genTime: 'time',
        ignored: 'not-forwarded',
      }),
    ).toEqual({
      lotNumber: 'lot',
      captchaOutput: 'output',
      passToken: 'pass',
      genTime: 'time',
    })
  })
})

describe('连接状态归一化', () => {
  it('保留已知后端状态并将未知值降级为断开', () => {
    expect(normalizeConnectionStatus('connected')).toBe('connected')
    expect(normalizeConnectionStatus('connecting')).toBe('connecting')
    expect(normalizeConnectionStatus('disconnected')).toBe('disconnected')
    expect(normalizeConnectionStatus('unexpected')).toBe('disconnected')
  })
})

describe('IPC 错误展示', () => {
  it('保留可识别错误消息并为未知拒绝值提供统一提示', () => {
    expect(errorMessage(new Error('Not logged in'))).toBe('Not logged in')
    expect(errorMessage('network failed')).toBe('network failed')
    expect(errorMessage({ code: 500 })).toBe('操作失败，请查看后端日志')
  })
})
