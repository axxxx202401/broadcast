// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  loadOlderMessages: vi.fn(),
  monitor: {
    loggedIn: { value: true },
    warning: { value: '' },
    error: { value: '' },
    connectionStatus: { value: 'connected' },
    uid: { value: '42' },
    pending: { value: null },
    connectDisabled: { value: false },
    filteredGroups: { value: [] },
    groups: { value: [] },
    monitoredCount: { value: 0 },
    selectedGroup: { value: null },
    search: { value: '' },
    messages: { value: [] },
    messagesLoading: { value: false },
    hasOlder: { value: true },
    loadingOlder: { value: false },
    olderRequestToken: { value: 7 },
    disconnect: vi.fn(),
    connect: vi.fn(),
    logout: vi.fn(),
    selectGroup: vi.fn(),
    showAllMessages: vi.fn(),
    toggleGroup: vi.fn(),
    refreshGroups: vi.fn(),
    fetchGroups: vi.fn(),
    acceptLogin: vi.fn(),
    loadOlderMessages: vi.fn(),
    handleOlderSettled: vi.fn(),
  },
}))

vi.mock('./composables/useMonitor', () => ({
  useMonitor: () => mocks.monitor,
}))
vi.mock('./composables/useAuth', () => ({
  useAuth: () => ({}),
}))

import App from './App.vue'

describe('App 消息分页接线', () => {
  it('把分页状态和请求代次传给消息面板并转发双向握手事件', async () => {
    const wrapper = mount(App, {
      global: {
        stubs: {
          GroupSidebar: true,
          LoginPanel: true,
          StatusBadge: true,
          MessagePanel: {
            name: 'MessagePanel',
            props: ['hasOlder', 'loadingOlder', 'olderRequestToken'],
            emits: ['load-older', 'older-settled'],
            template: `
              <div>
                <button class="load-older" @click="$emit('load-older')" />
                <button class="older-settled" @click="$emit('older-settled', olderRequestToken)" />
              </div>
            `,
          },
        },
      },
    })

    const panel = wrapper.getComponent({ name: 'MessagePanel' })
    expect(panel.props('hasOlder')).toBe(true)
    expect(panel.props('loadingOlder')).toBe(false)
    expect(panel.props('olderRequestToken')).toBe(7)
    await wrapper.get('.load-older').trigger('click')
    expect(mocks.monitor.loadOlderMessages).toHaveBeenCalledOnce()
    await wrapper.get('.older-settled').trigger('click')
    expect(mocks.monitor.handleOlderSettled).toHaveBeenCalledWith(7)
  })
})
