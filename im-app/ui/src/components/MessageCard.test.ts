// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import type { MessageDto } from '../types/im'
import MessageCard from './MessageCard.vue'

vi.mock('../services/tauri', () => ({
  api: { downloadMessageAttachment: vi.fn() },
}))

/** 设计示例：群 13537、用户 100267、正文「重要告警」。 */
function textMessage(): MessageDto {
  return {
    msg_id: '1',
    group_id: '13537',
    group_name: '运营群',
    send_uid: '100267',
    msg_type: 0,
    content_b64: '',
    decoded_content: { kind: 'text', text: '重要告警' },
    decode_error: null,
    send_time: 1,
    content_md5: '',
    stored_at: null,
  }
}

describe('MessageCard', () => {
  // 卡片视觉顺序固定为来源 → 发送人/时间 → 正文；已知文本不出现类型标签。
  it('消息卡先显示弱化元信息再显示正文', () => {
    const wrapper = mount(MessageCard, { props: { message: textMessage(), showGroup: true } })
    expect(wrapper.get('.message-source').text()).toContain('#13537')
    expect(wrapper.get('.message-sender').text()).toBe('用户 100267')
    expect(wrapper.get('.message-content').text()).toContain('重要告警')
    expect(wrapper.text()).not.toContain('文本')
  })
})
