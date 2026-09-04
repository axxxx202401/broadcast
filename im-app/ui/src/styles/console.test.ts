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
})
