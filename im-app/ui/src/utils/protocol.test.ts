import { describe, expect, it } from 'vitest'

import { createGt4Payload, errorMessage, normalizeConnectionStatus } from './protocol'

describe('createGt4Payload', () => {
  it('passes through only the four confirmed GT4 fields', () => {
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

describe('normalizeConnectionStatus', () => {
  it('maps known backend event values and rejects unknown states safely', () => {
    expect(normalizeConnectionStatus('connected')).toBe('connected')
    expect(normalizeConnectionStatus('connecting')).toBe('connecting')
    expect(normalizeConnectionStatus('disconnected')).toBe('disconnected')
    expect(normalizeConnectionStatus('unexpected')).toBe('disconnected')
  })
})

describe('errorMessage', () => {
  it('preserves command errors and handles unknown rejected values', () => {
    expect(errorMessage(new Error('Not logged in'))).toBe('Not logged in')
    expect(errorMessage('network failed')).toBe('network failed')
    expect(errorMessage({ code: 500 })).toBe('操作失败，请查看后端日志')
  })
})
