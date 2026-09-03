// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import MessagePanel from './MessagePanel.vue'

vi.mock('../services/tauri', () => ({
  api: { downloadMessageAttachment: vi.fn() },
}))

describe('MessagePanel', () => {
  it('全部消息模式显示每条消息所属群组', () => {
    const wrapper = mount(MessagePanel, {
      props: {
        group: null,
        loading: false,
        messages: [{
          msg_id: '1',
          group_id: '20',
          group_name: '运维群',
          send_uid: '3',
          msg_type: 0,
          content_b64: '',
          decoded_content: { kind: 'text', text: '告警恢复' },
          decode_error: null,
          send_time: 1,
          content_md5: '',
          stored_at: null,
        }],
      },
    })

    expect(wrapper.text()).toContain('全部群消息')
    expect(wrapper.text()).toContain('运维群')
    expect(wrapper.text()).toContain('#20')
    expect(wrapper.text()).toContain('告警恢复')
  })
})
