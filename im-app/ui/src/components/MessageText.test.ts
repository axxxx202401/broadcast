// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import MessageText from './MessageText.vue'

describe('MessageText', () => {
  // 别名必须走文本节点替换；潜在 HTML 不得进入 DOM 解析，避免 XSS。
  it('使用文本节点显示别名且不解析 HTML', () => {
    const wrapper = mount(MessageText, { props: { text: '<img src=x onerror=alert(1)>[呲牙]' } })
    expect(wrapper.text()).toBe('<img src=x onerror=alert(1)>😁')
    expect(wrapper.find('img').exists()).toBe(false)
  })

  // 仅少量 Emoji 时加突出样式，供卡片正文放大字号。
  it('纯 Emoji 使用突出样式', () => {
    const wrapper = mount(MessageText, { props: { text: '[呲牙][憨笑]' } })
    expect(wrapper.classes()).toContain('message-text--emoji-only')
  })
})
