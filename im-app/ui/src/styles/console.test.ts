// @ts-expect-error 当前 tsconfig 只纳入 Vite/DOM types，不含 Node 内置模块。
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

/**
 * 滚动布局静态验收：禁止整页滚动，仅群列表与消息视口作为业务滚动区，
 * 并同时提供标准 scrollbar-color 与 WebKit 伪元素样式。
 */
const base = readFileSync(new URL('./base.css', import.meta.url), 'utf8')
const consoleCss = readFileSync(new URL('./console.css', import.meta.url), 'utf8')

describe('scroll layout', () => {
  it('禁止整页滚动并限定业务滚动区', () => {
    expect(base).toMatch(/body[\s\S]*overflow:\s*hidden/)
    expect(consoleCss).toMatch(/\.group-list[\s\S]*overflow-y:\s*auto/)
    expect(consoleCss).toMatch(/\.message-viewport[\s\S]*overflow-y:\s*auto/)
  })

  it('同时定义标准和 WebKit 滚动条', () => {
    expect(consoleCss).toContain('scrollbar-color')
    expect(consoleCss).toContain('::-webkit-scrollbar')
  })

  it('登录表单输入框与手机区号网格使用全宽布局', () => {
    expect(consoleCss).toMatch(/\.login-primary-panel[\s\S]*grid-template-rows:\s*auto\s+1fr/)
    expect(consoleCss).toMatch(/\.login-form-fields \.account-row\.is-phone[\s\S]*grid-template-columns:\s*110px minmax\(0,\s*1fr\)/)
    expect(consoleCss).toMatch(/\.login-form-fields \.secret-field\.is-code[\s\S]*padding-right:\s*100px/)
    expect(consoleCss).toMatch(/\.login-form-fields input[\s\S]*height:\s*44px/)
    expect(consoleCss).toMatch(/\.login-shell[\s\S]*width:\s*900px/)
  })
})
