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
    matched: 0,
  }
}

describe('MessageBody', () => {
  it('直接显示已解密文本', () => {
    const wrapper = mount(MessageBody, {
      props: { message: message({ kind: 'text', text: '可读正文' }) },
    })

    expect(wrapper.text()).toContain('可读正文')
  })

  it('文本消息不显示类型标签', () => {
    const wrapper = mount(MessageBody, {
      props: { message: message({ kind: 'text', text: '可读正文' }) },
    })

    expect(wrapper.text()).toContain('可读正文')
    expect(wrapper.text()).not.toContain('文本')
    expect(wrapper.find('.message-kind').exists()).toBe(false)
  })

  // 文本分支必须接入 MessageText：别名替换走文本节点，标签不得被解析为 DOM。
  it('文本分支使用文本节点渲染 Emoji 且不解析 HTML', () => {
    const wrapper = mount(MessageBody, {
      props: { message: message({ kind: 'text', text: '<img src=x onerror=alert(1)>[呲牙]' }) },
    })

    expect(wrapper.text()).toContain('<img src=x onerror=alert(1)>😁')
    expect(wrapper.find('img').exists()).toBe(false)
    expect(wrapper.find('.message-kind').exists()).toBe(false)
  })

  it('媒体按钮没有解密字样', () => {
    const cases: Array<{ content: NonNullable<MessageDto['decoded_content']>; label: string }> = [
      {
        content: {
          kind: 'image',
          url: 'https://cdn.test/i',
          thumbnail_url: '',
          file_size: 1,
          width: 1,
          height: 1,
        },
        label: '打开图片',
      },
      {
        content: { kind: 'audio', url: 'https://cdn.test/a', duration: 3, file_size: 1 },
        label: '打开音频',
      },
      {
        content: {
          kind: 'video',
          url: 'https://cdn.test/v',
          thumbnail_url: '',
          duration: 3,
          file_size: 1,
          width: 1,
          height: 1,
        },
        label: '打开视频',
      },
      {
        content: {
          kind: 'file',
          url: 'https://cdn.test/f',
          name: '报告.pdf',
          mime_type: 'application/pdf',
          file_size: 1024,
        },
        label: '打开文件',
      },
    ]

    for (const item of cases) {
      const wrapper = mount(MessageBody, {
        props: { message: message(item.content) },
      })
      expect(wrapper.text()).toContain(item.label)
      expect(wrapper.text()).not.toContain('解密')
      expect(wrapper.find('.message-kind').exists()).toBe(false)
      wrapper.unmount()
    }
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
