import { describe, expect, it } from 'vitest'

import { isEmojiOnly, tokenizeMessageText } from './emoji'

describe('tokenizeMessageText', () => {
  /** 映射表命中转为 Unicode；未收录别名不得丢弃，须作为文本 token 保留原文。 */
  it('转换已知别名并保留未知别名', () => {
    expect(tokenizeMessageText('[呲牙][憨笑][未知]')).toEqual([
      { kind: 'emoji', source: '[呲牙]', value: '😁' },
      { kind: 'emoji', source: '[憨笑]', value: '😄' },
      { kind: 'text', value: '[未知]' },
    ])
  })

  /** 标签按普通文本保留，拼接后不得丢失或改写潜在 HTML，已知别名仍替换为 Emoji。 */
  it('保留混合文本和潜在 HTML', () => {
    expect(
      tokenizeMessageText('告警<script>[呲牙]').map(({ value }) => value).join(''),
    ).toBe('告警<script>😁')
  })

  /** 仅空白与少量 Emoji 时突出展示；夹杂汉字则按普通正文处理。 */
  it('识别少量纯 Emoji 内容', () => {
    expect(isEmojiOnly(tokenizeMessageText('[呲牙] 😄'))).toBe(true)
    expect(isEmojiOnly(tokenizeMessageText('收到 😄'))).toBe(false)
  })
})
