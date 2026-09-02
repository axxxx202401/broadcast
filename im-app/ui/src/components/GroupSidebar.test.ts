// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import GroupSidebar from './GroupSidebar.vue'
import type { GroupDto } from '../types/im'

const group: GroupDto = {
  group_id: '9223372036854775807',
  name: '生产告警群',
  pic: '',
  host_id: null,
  member_count: 12,
  created_at: 0,
  monitored: 0,
  updated_at: 0,
}

describe('GroupSidebar', () => {
  it('uses list semantics with sibling selection and monitoring buttons', async () => {
    const wrapper = mount(GroupSidebar, {
      props: {
        groups: [group],
        total: 1,
        monitoredCount: 0,
        selectedId: null,
        search: '',
        pending: null,
      },
    })
    const list = wrapper.get('ul.group-list')
    const item = list.get('li.group-row')
    const buttons = item.findAll(':scope > button')

    expect(list.attributes('role')).toBeUndefined()
    expect(item.attributes('role')).toBeUndefined()
    expect(buttons).toHaveLength(2)
    expect(buttons[0]?.classes()).toContain('group-select')
    expect(buttons[0]?.attributes('aria-pressed')).toBe('false')
    expect(buttons[1]?.classes()).toContain('monitor-switch')
    expect(buttons[0]?.find('button').exists()).toBe(false)
    expect(buttons[1]?.find('button').exists()).toBe(false)

    await buttons[0]?.trigger('click')
    await buttons[1]?.trigger('click')

    expect(wrapper.emitted('select')).toEqual([[group.group_id]])
    expect(wrapper.emitted('toggle')).toEqual([[group]])
  })
})
