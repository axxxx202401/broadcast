import type { MessageDto } from '../types/im'

export const MAX_MESSAGES = 1000

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

export function mergeMessages(current: MessageDto[], incoming: MessageDto[]): MessageDto[] {
  const byId = new Map(current.map((message) => [message.msg_id, message]))
  for (const message of incoming) byId.set(message.msg_id, message)
  const ordered = [...byId.values()].sort((left, right) => left.send_time - right.send_time)
  return ordered.slice(-MAX_MESSAGES)
}

export function isCurrentMessageRequest(
  requestId: number,
  currentRequestId: number,
  requestedGroupId: string,
  selectedGroupId: string | null,
): boolean {
  return requestId === currentRequestId && requestedGroupId === selectedGroupId
}

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
