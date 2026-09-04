// @vitest-environment jsdom

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import MonitoredGroupSummary from './MonitoredGroupSummary.vue'

describe('MonitoredGroupSummary', () => {
  /** 无监控群时只提示空态，不渲染展开控件。 */
  it('空数组显示尚未选择监控群', () => {
    const wrapper = mount(MonitoredGroupSummary, {
      props: { groupIds: [] },
    })
    expect(wrapper.text()).toContain('尚未选择监控群')
    expect(wrapper.find('button').exists()).toBe(false)
  })

  /** 不超过折叠阈值时全部可见，避免多余交互。 */
  it('一到五个群只展示 ID 且不出现展开按钮', () => {
    const wrapper = mount(MonitoredGroupSummary, {
      props: { groupIds: ['1', '2', '3', '4', '5'] },
    })
    expect(wrapper.text()).toContain('#1')
    expect(wrapper.text()).toContain('#5')
    expect(wrapper.find('button').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('另有')
    expect(wrapper.text()).not.toContain('展开全部')
  })

  /** 默认只露出前五个 ID；展开后才出现被折叠的 ID，并提供收起。 */
  it('超过五个群时默认折叠并可展开', async () => {
    const wrapper = mount(MonitoredGroupSummary, {
      props: { groupIds: ['1', '2', '3', '4', '5', '6', '7'] },
    })
    expect(wrapper.text()).toContain('另有 2 个')
    expect(wrapper.text()).not.toContain('#7')
    await wrapper.get('button').trigger('click')
    expect(wrapper.text()).toContain('#7')
    expect(wrapper.text()).toContain('收起')
  })
})
