import type { MessageDto } from '../types/im'

/** 前端内存中最多保留的已排序、去重消息数。 */
export const MAX_MESSAGES = 1000

/** 超过内存上限时选择保留时间较新或较早的一侧窗口。 */
export type MessageTrimStrategy = 'keep-latest' | 'keep-earliest'

/** 一次索引合并的可观察结果；`trimmed` 是因容量上限实际删除的唯一消息数。 */
export interface MessageMergeResult {
  changed: boolean
  trimmed: number
}

/**
 * 持久维护消息 ID 索引与有序视图。
 *
 * 不变量：`byId` 与 `ordered` 始终包含相同的消息集合；数组按 `send_time` 升序排列，
 * 时间相同时按十进制 `msg_id` 的 i64 数值升序排列，与后端 keyset 顺序完全互逆。
 * 常规有序消息为 O(1) 尾部追加，乱序消息以 O(log n) 二分查找位置、再以 O(n)
 * 移动数组元素；任何路径都不执行全量排序。
 */
export class MessageIndex {
  private readonly byId = new Map<string, MessageDto>()
  private readonly ordered: MessageDto[] = []

  constructor(initial: MessageDto[] = []) {
    this.merge(initial)
  }

  /** 当前索引内的消息数量；始终与内部有序数组长度一致。 */
  get size(): number {
    return this.byId.size
  }

  /** 按十进制字符串 ID 读取当前对象，不将大整数 ID 转换为 `number`。 */
  get(msgId: string): MessageDto | undefined {
    return this.byId.get(msgId)
  }

  /** 清空 ID 索引及有序数组，用于切换消息查询范围。 */
  clear(): void {
    this.byId.clear()
    this.ordered.length = 0
  }

  /**
   * 原位合并一批消息，批内重复 ID 由最后一项覆盖。
   * `keep-latest` 用于实时和首屏，超限时删除头部旧消息；`keep-earliest` 用于向上翻页，
   * 超限时删除尾部新消息，使新取得的历史窗口不会被立即丢弃。两种策略都同步维护 Map。
   */
  merge(
    incoming: MessageDto[],
    trimStrategy: MessageTrimStrategy = 'keep-latest',
  ): boolean {
    return this.mergeWithResult(incoming, trimStrategy).changed
  }

  /**
   * 原位合并并返回实际裁剪数量。
   *
   * 批内先按 ID 折叠为最后一个对象，确保同一新 ID 不会因重复出现而被重复计作容量裁剪；
   * `merge` 保留原有布尔返回契约，新调用方可用此结果判断数据库消息是否已离开可见窗口。
   */
  mergeWithResult(
    incoming: MessageDto[],
    trimStrategy: MessageTrimStrategy = 'keep-latest',
  ): MessageMergeResult {
    if (incoming.length === 0) return { changed: false, trimmed: 0 }

    const uniqueIncoming = new Map<string, MessageDto>()
    for (const message of incoming) uniqueIncoming.set(message.msg_id, message)

    let trimmed = 0
    for (const message of uniqueIncoming.values()) {
      const previous = this.byId.get(message.msg_id)
      if (previous) {
        const previousIndex = this.ordered.indexOf(previous)
        this.byId.set(message.msg_id, message)
        if (previous.send_time === message.send_time) {
          this.ordered[previousIndex] = message
          continue
        }
        this.ordered.splice(previousIndex, 1)
      } else {
        this.byId.set(message.msg_id, message)
      }

      const insertionIndex = this.findInsertionIndex(message)
      this.ordered.splice(insertionIndex, 0, message)
      trimmed += this.trim(trimStrategy)
    }
    return { changed: true, trimmed }
  }

  /** 在一次合并完成后复制有序视图，供 Vue 以单次赋值发布响应式更新。 */
  snapshot(): MessageDto[] {
    return [...this.ordered]
  }

  private findInsertionIndex(message: MessageDto): number {
    const last = this.ordered.at(-1)
    if (!last || this.compare(last, message) <= 0) return this.ordered.length

    let low = 0
    let high = this.ordered.length
    while (low < high) {
      const middle = low + Math.floor((high - low) / 2)
      if (this.compare(this.ordered[middle]!, message) <= 0) low = middle + 1
      else high = middle
    }
    return low
  }

  private compare(left: MessageDto, right: MessageDto): number {
    if (left.send_time !== right.send_time) return left.send_time - right.send_time
    return compareDecimalI64(left.msg_id, right.msg_id)
  }

  private trim(strategy: MessageTrimStrategy): number {
    let trimmed = 0
    while (this.ordered.length > MAX_MESSAGES) {
      const removed = strategy === 'keep-latest'
        ? this.ordered.shift()!
        : this.ordered.pop()!
      this.byId.delete(removed.msg_id)
      trimmed += 1
    }
    return trimmed
  }
}

/**
 * 比较两个规范十进制 i64 字符串，不转换为 JavaScript `number`。
 *
 * 后端保证消息 ID 是可解析的 i64 十进制表示；这里按符号、有效数字长度和字典序比较，
 * 因而能精确处理超过 `2^53` 的值。负数的绝对值越大，实际数值越小。
 */
function compareDecimalI64(left: string, right: string): number {
  if (left === right) return 0
  const leftNegative = left.startsWith('-')
  const rightNegative = right.startsWith('-')
  if (leftNegative !== rightNegative) return leftNegative ? -1 : 1

  const leftDigits = leftNegative ? left.slice(1) : left
  const rightDigits = rightNegative ? right.slice(1) : right
  const magnitude = leftDigits.length === rightDigits.length
    ? leftDigits < rightDigits ? -1 : 1
    : leftDigits.length - rightDigits.length
  return leftNegative ? -magnitude : magnitude
}

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
 * 合并历史与实时消息：以字符串 `msg_id` 去重，后到消息覆盖同 ID 旧值，再按
 * `(send_time, msg_id)` 升序排列；超过 {@link MAX_MESSAGES} 时保留最新 1000 条。
 */
export function mergeMessages(current: MessageDto[], incoming: MessageDto[]): MessageDto[] {
  const index = new MessageIndex(current)
  index.merge(incoming)
  return index.snapshot()
}

/**
 * 判断异步消息查询结果是否仍可应用。
 * 仅请求序号仍为最新且请求群组仍是当前选择时返回 `true`，用于阻止切组或重载竞态的迟到响应。
 */
export function isCurrentMessageRequest(
  requestId: number,
  currentRequestId: number,
  requestedGroupId: string | null,
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
