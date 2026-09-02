import type { MessageDto } from '../types/im'

/** 前端内存中最多保留的已排序、去重消息数。 */
export const MAX_MESSAGES = 1000

/**
 * 解码后端提供的标准 Base64 正文字节。
 * 字节是合法 UTF-8 时返回文本；Base64 有效但 UTF-8 非法时显示二进制字节数，
 * Base64 本身无法解码时返回固定失败提示，避免把二进制内容误当文本。
 */
export function decodeMessageContent(contentBase64: string): string {
  try {
    const binary = atob(contentBase64)
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0))
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    try {
      return `[二进制内容 · ${atob(contentBase64).length} B]`
    } catch {
      return '[无法解码的消息内容]'
    }
  }
}

/**
 * 合并历史与实时消息：以字符串 `msg_id` 去重，后到消息覆盖同 ID 旧值，再按 `send_time`
 * 升序稳定排列；超过 {@link MAX_MESSAGES} 时仅保留排序后最新的 1000 条。
 */
export function mergeMessages(current: MessageDto[], incoming: MessageDto[]): MessageDto[] {
  const byId = new Map(current.map((message) => [message.msg_id, message]))
  for (const message of incoming) byId.set(message.msg_id, message)
  const ordered = [...byId.values()].sort((left, right) => left.send_time - right.send_time)
  return ordered.slice(-MAX_MESSAGES)
}

/**
 * 判断异步消息查询结果是否仍可应用。
 * 仅请求序号仍为最新且请求群组仍是当前选择时返回 `true`，用于阻止切组或重载竞态的迟到响应。
 */
export function isCurrentMessageRequest(
  requestId: number,
  currentRequestId: number,
  requestedGroupId: string,
  selectedGroupId: string | null,
): boolean {
  return requestId === currentRequestId && requestedGroupId === selectedGroupId
}

/**
 * 格式化消息时间：小于 `10_000_000_000` 的值按 Unix 秒转换，其余按毫秒处理。
 * 这是量级启发式而非协议校验；阈值、负数、非有限值或超出 `Date` 范围的输入可能被误判，
 * `Intl.DateTimeFormat` 对无效日期可能抛出 `RangeError`。
 */
export function formatMessageTime(timestamp: number): string {
  const milliseconds = timestamp < 10_000_000_000 ? timestamp * 1000 : timestamp
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(new Date(milliseconds))
}
