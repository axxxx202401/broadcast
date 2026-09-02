import type { UserConfig } from 'vite'
import { describe, expect, it } from 'vitest'

import config, { DEV_CSP_HEADER } from '../vite.config'
import tauriConfig from '../../tauri.conf.json'

function serializeCsp(directives: Record<string, string>): string {
  return Object.entries(directives)
    .map(([directive, value]) => `${directive} ${value}`)
    .join('; ')
}

describe('Vite 开发环境 CSP', () => {
  it('发送与 Tauri devCsp 等价的响应头', () => {
    const headers = (config as UserConfig).server?.headers as Record<string, string>
    const expected = serializeCsp(tauriConfig.app.security.devCsp)

    expect(DEV_CSP_HEADER).toBe(expected)
    expect(headers['Content-Security-Policy']).toBe(expected)
  })

  it('允许 GT4 资源并保留限制文档能力的指令', () => {
    expect(DEV_CSP_HEADER).toContain('ws://127.0.0.1:1420')
    expect(DEV_CSP_HEADER).toContain("script-src 'self' https://static.geetest.com")
    expect(DEV_CSP_HEADER).toContain('https://gcaptcha4.geetest.com')
    expect(DEV_CSP_HEADER).toContain('https://monitor.geetest.com')
    expect(DEV_CSP_HEADER).toContain('frame-src https://static.geetest.com')
    expect(DEV_CSP_HEADER).toContain(
      "style-src 'self' 'unsafe-inline' https://static.geetest.com",
    )
    expect(DEV_CSP_HEADER).not.toContain('script-src *')
    expect(DEV_CSP_HEADER).toContain("object-src 'none'")
    expect(DEV_CSP_HEADER).toContain("base-uri 'self'")
    expect(DEV_CSP_HEADER).toContain("frame-ancestors 'none'")
  })
})
