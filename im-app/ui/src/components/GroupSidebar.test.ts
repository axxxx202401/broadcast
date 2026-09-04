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
  it('提供全部消息入口并在无单群选择时标记选中', async () => {
    const wrapper = mount(GroupSidebar, {
      props: {
        groups: [group],
        total: 1,
        monitoredCount: 0,
        selectedId: null,
        search: '',
        pending: null,
        showMatchedOnly: true,
      },
    })

    const button = wrapper.get('button.all-messages')
    expect(button.attributes('aria-pressed')).toBe('true')
    await button.trigger('click')
    expect(wrapper.emitted('select-all')).toEqual([[]])
  })

  // 可访问性交互契约：列表保留原生语义，选择与开关必须是两个非嵌套按钮并各自发出事件。
  it('uses list semantics with sibling selection and monitoring buttons', async () => {
    const wrapper = mount(GroupSidebar, {
      props: {
        groups: [group],
        total: 1,
        monitoredCount: 0,
        selectedId: null,
        search: '',
        pending: null,
        showMatchedOnly: true,
      },
    })
    // monitored=0 群组渲染在"未监听"列表中，该列表在 sections.expanded=true 时隐藏。
    // 点击标题行展开未监听区后查询列表。
    await wrapper.findAll('div.section-header')[1].trigger('click')
    const list = wrapper.get('ul.group-list.unmonitored-list')
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
