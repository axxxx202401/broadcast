// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import type { MessageDto } from '../types/im'
import MessageBody from './MessageBody.vue'

const mocks = vi.hoisted(() => ({
  download: vi.fn(),
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
}))

vi.mock('../services/tauri', () => ({
  api: { downloadMessageAttachment: mocks.download },
}))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: mocks.convertFileSrc,
}))

function message(decodedContent: MessageDto['decoded_content']): MessageDto {
  return {
    msg_id: '1',
    group_id: '2',
    group_name: '测试群',
    send_uid: '3',
    msg_type: decodedContent?.kind === 'file' ? 7 : 0,
    content_b64: '',
    decoded_content: decodedContent,
    decode_error: null,
    send_time: 1,
    content_md5: '',
    stored_at: null,
  }
}

describe('MessageBody', () => {
  it('直接显示已解密文本', () => {
    const wrapper = mount(MessageBody, {
      props: { message: message({ kind: 'text', text: '可读正文' }) },
    })

    expect(wrapper.text()).toContain('可读正文')
  })

  it('按需解密文件并提供原始文件名下载', async () => {
    mocks.download.mockResolvedValueOnce({
      path: '/tmp/report.pdf',
      mime_type: 'application/pdf',
    })
    const wrapper = mount(MessageBody, {
      props: {
        message: message({
          kind: 'file',
          url: 'https://cdn.test/file',
          name: '报告.pdf',
          mime_type: 'application/pdf',
          file_size: 1024,
        }),
      },
    })

    await wrapper.get('button').trigger('click')
    await vi.waitFor(() => expect(wrapper.find('a').exists()).toBe(true))

    expect(mocks.download).toHaveBeenCalledWith('1', false)
    expect(wrapper.get('a').attributes('download')).toBe('报告.pdf')
    expect(wrapper.get('a').attributes('href')).toBe('asset:///tmp/report.pdf')
  })
})
