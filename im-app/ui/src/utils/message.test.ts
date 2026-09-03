import { describe, expect, it, vi } from 'vitest'

import {
  decodeMessageContent,
  isCurrentMessageRequest,
  mergeMessages,
  MessageIndex,
} from './message'
import type { MessageDto } from '../types/im'

const message = (msgId: string, sendTime: number): MessageDto => ({
  msg_id: msgId,
  group_id: '9223372036854775806',
  send_uid: '9223372036854775805',
  msg_type: 1,
  group_name: '测试群',
  content_b64: btoa(`message-${msgId}`),
  decoded_content: null,
  decode_error: null,
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

  it('相同发送时间按超出安全整数范围的消息 ID 数值升序排列', () => {
    const current = [message('9007199254740993', 20), message('9007199254740992', 20)]
    const merged = mergeMessages(current, [message('9007199254740994', 20)])
    expect(merged.map(({ msg_id }) => msg_id)).toEqual([
      '9007199254740992',
      '9007199254740993',
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

describe('持久消息索引', () => {
  it('有序新消息走尾部追加且不调用全量排序', () => {
    const sort = vi.spyOn(Array.prototype, 'sort')
    const index = new MessageIndex([message('1', 10)])

    index.merge([message('2', 20), message('3', 30)])

    expect(index.snapshot().map(({ msg_id }) => msg_id)).toEqual(['1', '2', '3'])
    expect(sort).not.toHaveBeenCalled()
    sort.mockRestore()
  })

  it('乱序新消息按二分定位插入且不调用全量排序', () => {
    const sort = vi.spyOn(Array.prototype, 'sort')
    const index = new MessageIndex([message('1', 10), message('3', 30)])

    index.merge([message('2', 20)])

    expect(index.snapshot().map(({ msg_id }) => msg_id)).toEqual(['1', '2', '3'])
    expect(sort).not.toHaveBeenCalled()
    sort.mockRestore()
  })

  it('重复 ID 后到覆盖旧对象并在排序键变化时移动', () => {
    const index = new MessageIndex([message('1', 10), message('2', 20)])
    const replacement = { ...message('1', 30), content_md5: 'replacement' }

    index.merge([replacement])

    expect(index.snapshot().map(({ msg_id }) => msg_id)).toEqual(['2', '1'])
    expect(index.get('1')).toBe(replacement)
  })

  it('批内重复 ID 仅保留最后到达的对象', () => {
    const index = new MessageIndex()
    const replacement = { ...message('9223372036854775807', 20), content_md5: 'replacement' }

    index.merge([
      message('9223372036854775807', 10),
      replacement,
    ])

    expect(index.size).toBe(1)
    expect(index.get('9223372036854775807')).toBe(replacement)
    expect(index.snapshot()).toEqual([replacement])
  })

  it('超过上限时同步裁剪有序数组和 ID 索引', () => {
    const index = new MessageIndex()
    index.merge(Array.from({ length: 1005 }, (_, value) => message(String(value), value)))

    expect(index.size).toBe(1000)
    expect(index.snapshot()).toHaveLength(1000)
    expect(index.get('0')).toBeUndefined()
    expect(index.get('4')).toBeUndefined()
    expect(index.get('5')?.msg_id).toBe('5')
  })

  it('合并10000条突发与重复更新后保持复合排序、去重及Map一致', () => {
    const index = new MessageIndex()
    const burst = Array.from({ length: 10_000 }, (_, value) =>
      message(String(value), Math.floor(value / 10)),
    ).reverse()
    const duplicateUpdates = Array.from({ length: 500 }, (_, offset) => {
      const value = 9_500 + offset
      return {
        ...message(String(value), Math.floor(value / 10)),
        content_md5: `updated-${value}`,
      }
    }).reverse()
    const started = performance.now()

    const result = index.mergeWithResult([...burst, ...duplicateUpdates])
    const snapshot = index.snapshot()

    expect(result).toEqual({ changed: true, trimmed: 9000 })
    expect(snapshot).toHaveLength(1000)
    expect(index.size).toBe(1000)
    expect(snapshot.map(({ msg_id }) => msg_id)).toEqual(
      Array.from({ length: 1000 }, (_, offset) => String(9_000 + offset)),
    )
    expect(new Set(snapshot.map(({ msg_id }) => msg_id)).size).toBe(1000)
    expect(snapshot.every((entry) => index.get(entry.msg_id) === entry)).toBe(true)
    expect(index.get('8999')).toBeUndefined()
    expect(index.get('9500')?.content_md5).toBe('updated-9500')
    console.info(
      `10k MessageIndex load: elapsed=${(performance.now() - started).toFixed(2)}ms, retained=${snapshot.length}`,
    )
  })

  it('向上加载达到上限时保留更早窗口并同步裁剪尾部索引', () => {
    const index = new MessageIndex(
      Array.from({ length: 1000 }, (_, value) =>
        message(String(value + 200), value + 200),
      ),
    )

    index.merge(
      Array.from({ length: 200 }, (_, value) => message(String(value), value)),
      'keep-earliest',
    )

    const snapshot = index.snapshot()
    expect(snapshot).toHaveLength(1000)
    expect(index.size).toBe(1000)
    expect(snapshot[0]?.msg_id).toBe('0')
    expect(snapshot.at(-1)?.msg_id).toBe('999')
    expect(index.get('0')?.msg_id).toBe('0')
    expect(index.get('999')?.msg_id).toBe('999')
    expect(index.get('1000')).toBeUndefined()
    expect(index.get('1199')).toBeUndefined()
  })

  it('相同发送时间跨页合并后严格按有符号十进制 i64 消息 ID 升序排列', () => {
    const index = new MessageIndex([
      message('9223372036854775807', 100),
      message('9007199254740993', 100),
    ])

    index.merge([
      message('-9223372036854775808', 100),
      message('9007199254740992', 100),
    ], 'keep-earliest')

    expect(index.snapshot().map(({ msg_id }) => msg_id)).toEqual([
      '-9223372036854775808',
      '9007199254740992',
      '9007199254740993',
      '9223372036854775807',
    ])
  })
})

describe('消息请求竞态门禁', () => {
  it('仅接受当前所选群组的最新请求结果', () => {
    expect(isCurrentMessageRequest(3, 3, '7', '7')).toBe(true)
    expect(isCurrentMessageRequest(2, 3, '7', '7')).toBe(false)
    expect(isCurrentMessageRequest(3, 3, '7', '8')).toBe(false)
  })
})
