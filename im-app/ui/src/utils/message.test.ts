import { describe, expect, it } from 'vitest'

import { decodeMessageContent, isCurrentMessageRequest, mergeMessages } from './message'
import type { MessageDto } from '../types/im'

const message = (msgId: string, sendTime: number): MessageDto => ({
  msg_id: msgId,
  group_id: '9223372036854775806',
  send_uid: '9223372036854775805',
  msg_type: 1,
  content_b64: btoa(`message-${msgId}`),
  send_time: sendTime,
  content_md5: `md5-${msgId}`,
  stored_at: null,
})

describe('消息正文解码', () => {
  it('将 Base64 中的 UTF-8 中文字节无损解码为文本', () => {
    expect(decodeMessageContent(btoa(unescape(encodeURIComponent('告警：连接中断'))))).toBe(
      '告警：连接中断',
    )
  })

  it('对非 UTF-8 字节返回可见的二进制回退提示', () => {
    expect(decodeMessageContent('/w==')).toBe('[二进制内容 · 1 B]')
  })
})

describe('历史与实时消息合并', () => {
  it('将乱序历史响应按发送时间升序排列', () => {
    expect(mergeMessages([], [message('2', 20), message('1', 10)]).map(({ msg_id }) => msg_id))
      .toEqual(['1', '2'])
  })

  it('以字符串消息 ID 去重且不损失大整数精度', () => {
    const id = '9223372036854775807'
    expect(mergeMessages([message(id, 20)], [message(id, 20)])).toHaveLength(1)
  })

  it('实时消息时间相同时保持到达顺序', () => {
    const current = [message('9007199254740993', 20), message('9007199254740992', 20)]
    const merged = mergeMessages(current, [message('9007199254740994', 20)])
    expect(merged.map(({ msg_id }) => msg_id)).toEqual([
      '9007199254740993',
      '9007199254740992',
      '9007199254740994',
    ])
  })

  it('排序去重后只保留最新 1000 条消息', () => {
    const incoming = Array.from({ length: 1005 }, (_, index) =>
      message(String(index), index),
    ).reverse()

    const merged = mergeMessages(
      [message('1004', 1004)],
      incoming,
    )

    expect(merged).toHaveLength(1000)
    expect(merged[0]?.msg_id).toBe('5')
    expect(merged.at(-1)?.msg_id).toBe('1004')
    expect(new Set(merged.map(({ msg_id }) => msg_id)).size).toBe(1000)
  })

  it('不会截断正常的 50 条历史响应', () => {
    const history = Array.from({ length: 50 }, (_, index) => message(String(index), index))
    expect(mergeMessages([], history)).toHaveLength(50)
  })
})

describe('消息请求竞态门禁', () => {
  it('仅接受当前所选群组的最新请求结果', () => {
    expect(isCurrentMessageRequest(3, 3, '7', '7')).toBe(true)
    expect(isCurrentMessageRequest(2, 3, '7', '7')).toBe(false)
    expect(isCurrentMessageRequest(3, 3, '7', '8')).toBe(false)
  })
})
