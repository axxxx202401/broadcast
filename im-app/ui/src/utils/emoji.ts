/**
 * 消息正文的方括号表情分词与纯 Emoji 判定。
 *
 * 只转换集中映射表中的别名，未知 `[...]` 并入相邻文本 token；潜在 HTML 一律按普通文本保留。
 * 本模块不引入 `v-html`，也不依赖第三方远程 Emoji 资源。
 */

/**
 * 线性分词结果：已知别名成为独立 emoji token，其余连续文本（含未知别名与 HTML）合并为 text token。
 * `source` 仅存在于 emoji token，记录转换前的方括号原文，供后续按原文对照或调试。
 */
export type MessageTextToken =
  | { kind: 'text'; value: string }
  | { kind: 'emoji'; source: string; value: string }

/**
 * QQ / 微信风格方括号别名到 Unicode Emoji 的集中映射。
 * 仅收录能与常见客户端默认表情对齐的别名；未列出的别名不得臆造，须保留原文。
 */
export const BRACKET_EMOJI = {
  '[呲牙]': '😁',
  '[憨笑]': '😄',
  '[微笑]': '🙂',
  '[流泪]': '😢',
  '[大哭]': '😭',
  '[发怒]': '😡',
  '[爱心]': '❤️',
  '[强]': '👍',
} as const

/** 匹配长度 1–16、不含换行与嵌套括号的方括号片段；`g`+`u` 供 `matchAll` 线性扫描。 */
const ALIAS_PATTERN = /\[[^[\]\r\n]{1,16}\]/gu

/**
 * 纯 Emoji 可见文本的字符集合：空白、扩展象形、Emoji 展示属性、变体选择符 U+FE0F、ZWJ U+200D。
 * 用于先排除汉字等正文，再单独限制可见 Emoji 数量。
 */
const EMOJI_ONLY_PATTERN =
  /^(?:\s|\p{Extended_Pictographic}|\p{Emoji_Presentation}|\uFE0F|\u200D)+$/u

/**
 * 将消息原文拆成文本 / Emoji token。
 *
 * 按 `ALIAS_PATTERN` 命中位置保留前后文本；映射表命中才输出 emoji token，
 * 未知别名与前后普通文本合并，避免把 `[未知]` 拆成孤立碎片。空字符串返回空数组。
 */
export function tokenizeMessageText(input: string): MessageTextToken[] {
  const tokens: MessageTextToken[] = []
  let cursor = 0

  for (const match of input.matchAll(ALIAS_PATTERN)) {
    const alias = match[0]
    const start = match.index ?? 0
    if (start > cursor) {
      pushText(tokens, input.slice(cursor, start))
    }
    const mapped = lookupBracketEmoji(alias)
    if (mapped !== undefined) {
      tokens.push({ kind: 'emoji', source: alias, value: mapped })
    } else {
      pushText(tokens, alias)
    }
    cursor = start + alias.length
  }

  if (cursor < input.length) {
    pushText(tokens, input.slice(cursor))
  }
  return tokens
}

/**
 * 判断分词结果是否为“少量纯 Emoji”消息。
 *
 * 先拼接各 token 的可见 `value`（别名已替换为 Unicode），再要求整段只含 Emoji / 变体选择符 / ZWJ / 空白，
 * 且非空白字素簇数量在 1–6。用于突出字号，汉字或超过 6 个可见 Emoji 均返回 false。
 */
export function isEmojiOnly(tokens: MessageTextToken[]): boolean {
  const visible = tokens.map((token) => token.value).join('')
  if (!visible || !EMOJI_ONLY_PATTERN.test(visible)) return false
  const emojiCount = countVisibleEmoji(visible)
  return emojiCount >= 1 && emojiCount <= 6
}

function lookupBracketEmoji(alias: string): string | undefined {
  if (!Object.hasOwn(BRACKET_EMOJI, alias)) return undefined
  return BRACKET_EMOJI[alias as keyof typeof BRACKET_EMOJI]
}

/** 将文本并入末尾 text token，保证未知别名与相邻正文合成一段。 */
function pushText(tokens: MessageTextToken[], value: string): void {
  if (!value) return
  const last = tokens.at(-1)
  if (last?.kind === 'text') {
    last.value += value
    return
  }
  tokens.push({ kind: 'text', value })
}

/**
 * 按字素簇统计可见 Emoji：跳过纯空白，ZWJ 序列计为 1。
 * 调用方须已确认整段通过 `EMOJI_ONLY_PATTERN`，因此每个非空白簇都是一个可见 Emoji。
 */
function countVisibleEmoji(visible: string): number {
  const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' })
  let count = 0
  for (const { segment } of segmenter.segment(visible)) {
    if (/^\s+$/u.test(segment)) continue
    count += 1
  }
  return count
}
