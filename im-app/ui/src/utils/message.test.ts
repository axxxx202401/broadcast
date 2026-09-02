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

describe('decodeMessageContent', () => {
  it('decodes UTF-8 base64 payloads without corrupting Chinese text', () => {
    expect(decodeMessageContent(btoa(unescape(encodeURIComponent('告警：连接中断'))))).toBe(
      '告警：连接中断',
    )
  })

  it('returns a visible binary fallback for invalid UTF-8', () => {
    expect(decodeMessageContent('/w==')).toBe('[二进制内容 · 1 B]')
  })
})

describe('mergeMessages', () => {
  it('sorts an unsorted historical response by send time ascending', () => {
    expect(mergeMessages([], [message('2', 20), message('1', 10)]).map(({ msg_id }) => msg_id))
      .toEqual(['1', '2'])
  })

  it('deduplicates precision-sensitive string ids', () => {
    const id = '9223372036854775807'
    expect(mergeMessages([message(id, 20)], [message(id, 20)])).toHaveLength(1)
  })

  it('preserves arrival order when realtime messages share a timestamp', () => {
    const current = [message('9007199254740993', 20), message('9007199254740992', 20)]
    const merged = mergeMessages(current, [message('9007199254740994', 20)])
    expect(merged.map(({ msg_id }) => msg_id)).toEqual([
      '9007199254740993',
      '9007199254740992',
      '9007199254740994',
    ])
  })

  it('keeps only the most recent 1000 messages after sorting and deduplication', () => {
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

  it('does not trim a normal 50-message history response', () => {
    const history = Array.from({ length: 50 }, (_, index) => message(String(index), index))
    expect(mergeMessages([], history)).toHaveLength(50)
  })
})

describe('isCurrentMessageRequest', () => {
  it('accepts only the latest request for the still-selected group', () => {
    expect(isCurrentMessageRequest(3, 3, '7', '7')).toBe(true)
    expect(isCurrentMessageRequest(2, 3, '7', '7')).toBe(false)
    expect(isCurrentMessageRequest(3, 3, '7', '8')).toBe(false)
  })
})
