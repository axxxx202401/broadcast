<script setup lang="ts">
import { ref } from 'vue'
import type { GroupDto } from '../types/im'

// 父组件提供筛选后的群组、统计与当前操作状态；侧栏不自行维护业务数据。
const props = withDefaults(defineProps<{
  groups: GroupDto[]
  total: number
  monitoredCount: number
  selectedId: string | null
  search: string
  pending: string | null
  showMatchedOnly?: boolean
  /** 侧栏当前是否处于收起窄条模式。 */
  collapsed?: boolean
}>(), {
  showMatchedOnly: true,
  collapsed: false,
})

// 搜索采用受控值更新，其余事件把选择、监控切换和刷新意图交还父组件。
defineEmits<{
  'update:search': [value: string]
  select: [groupId: string]
  'select-all': []
  toggle: [group: GroupDto]
  refresh: []
  'update:showMatchedOnly': [value: boolean]
  collapse: []
}>()

/** 各分组独立的展开/收起状态。 */
const sections = ref({
  monitored: true,
  unmonitored: false,
})
</script>

<template>
  <aside class="group-sidebar" aria-label="群组监控列表">
    <div class="sidebar-tools">
      <label class="search-field">
        <span class="sr-only">搜索群组</span>
        <span aria-hidden="true">⌕</span>
        <input
          :value="search"
          type="search"
          placeholder="搜索名称或群 ID"
          @input="$emit('update:search', ($event.target as HTMLInputElement).value)"
        />
      </label>
      <button class="icon-button" type="button" title="刷新全量群列表" :disabled="!!pending" @click="$emit('refresh')">
        <span :class="{ spinning: pending === 'refresh' }" aria-hidden="true">↻</span>
        <span class="sr-only">刷新群列表</span>
      </button>
      <button
        class="collapse-btn"
        type="button"
        :title="collapsed ? '展开群列表' : '收起群列表'"
        :aria-label="collapsed ? '展开群列表' : '收起群列表'"
        @click="$emit('collapse')"
      >
        <span aria-hidden="true" class="collapse-icon">{{ collapsed ? '›' : '‹' }}</span>
      </button>
    </div>

    <!-- 匹配消息开关 -->
    <div class="matched-toggle">
      <label class="toggle-label">
        <span>只显示匹配消息</span>
        <input
          type="checkbox"
          :checked="showMatchedOnly"
          @change="$emit('update:showMatchedOnly', ($event.target as HTMLInputElement).checked)"
        />
        <span class="toggle-slider"></span>
      </label>
    </div>

    <!-- 全部消息按钮 -->
    <button
      class="all-messages"
      :class="{ selected: selectedId === null }"
      type="button"
      :aria-pressed="selectedId === null"
      @click="$emit('select-all')"
    >
      <span aria-hidden="true">⌘</span>
      <span><strong>全部消息</strong><small>所有已监控群组</small></span>
    </button>

    <!-- 群组列表分组显示 -->
    <div class="section-header" @click="sections.monitored = !sections.monitored">
      <span>监听中（{{ monitoredCount }}）</span>
      <span class="chevron" :class="{ collapsed: !sections.monitored }">▼</span>
    </div>
    <ul v-if="sections.monitored" class="group-list monitored-list" aria-label="监听中的群组">
      <li
        v-for="group in groups.filter(g => g.monitored !== 0)"
        :key="group.group_id"
        class="group-row"
        :class="{ selected: selectedId === group.group_id }"
      >
        <button
          class="group-select"
          type="button"
          :aria-pressed="selectedId === group.group_id"
          :aria-label="`查看${group.name || `群组 ${group.group_id}`}消息`"
          @click="$emit('select', group.group_id)"
        >
          <span class="group-avatar">{{ group.name.slice(0, 1).toUpperCase() }}</span>
          <span class="group-identity">
            <strong>{{ group.name || `群组 ${group.group_id}` }}</strong>
            <small>#{{ group.group_id }} · {{ group.member_count }} 成员</small>
          </span>
        </button>
        <button
          class="monitor-switch"
          :class="{ active: group.monitored !== 0 }"
          type="button"
          role="switch"
          :aria-checked="group.monitored !== 0"
          :aria-label="`${group.name}监控`"
          @click.stop="$emit('toggle', group)"
        ><i></i></button>
      </li>
    </ul>

    <div class="section-header" @click="sections.unmonitored = !sections.unmonitored">
      <span>未监听（{{ total - monitoredCount }}）</span>
      <span class="chevron" :class="{ collapsed: !sections.unmonitored }">▼</span>
    </div>
    <ul v-if="sections.unmonitored" class="group-list unmonitored-list" aria-label="未监听的群组">
      <li
        v-for="group in groups.filter(g => g.monitored === 0)"
        :key="group.group_id"
        class="group-row"
        :class="{ selected: selectedId === group.group_id }"
      >
        <button
          class="group-select"
          type="button"
          :aria-pressed="selectedId === group.group_id"
          :aria-label="`查看${group.name || `群组 ${group.group_id}`}消息`"
          @click="$emit('select', group.group_id)"
        >
          <span class="group-avatar">{{ group.name.slice(0, 1).toUpperCase() }}</span>
          <span class="group-identity">
            <strong>{{ group.name || `群组 ${group.group_id}` }}</strong>
            <small>#{{ group.group_id }} · {{ group.member_count }} 成员</small>
          </span>
        </button>
        <button
          class="monitor-switch"
          :class="{ active: group.monitored !== 0 }"
          type="button"
          role="switch"
          :aria-checked="group.monitored !== 0"
          :aria-label="`${group.name}监控`"
          @click.stop="$emit('toggle', group)"
        ><i></i></button>
      </li>
    </ul>

    <div v-if="groups.length === 0" class="compact-empty">
      <span>没有匹配的群</span>
      <p>没有匹配的群组</p>
    </div>
  </aside>
</template>

<style scoped>
.matched-toggle {
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color, #e2e2e6);
}

.toggle-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  cursor: pointer;
  font-size: 13px;
  color: var(--text-primary, #1d1d1f);
}

.toggle-label input[type="checkbox"] {
  display: none;
}

.toggle-slider {
  position: relative;
  width: 40px;
  height: 22px;
  background: var(--border-color, #e2e2e6);
  border-radius: 11px;
  transition: background 0.2s;
}

.toggle-slider::after {
  content: '';
  position: absolute;
  top: 2px;
  left: 2px;
  width: 18px;
  height: 18px;
  background: white;
  border-radius: 50%;
  transition: transform 0.2s;
  box-shadow: 0 1px 3px rgba(0,0,0,0.2);
}

.toggle-label input:checked + .toggle-slider {
  background: var(--accent-color, #007aff);
}

.toggle-label input:checked + .toggle-slider::after {
  transform: translateX(18px);
}

.section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary, #6e6e73);
  cursor: pointer;
  user-select: none;
}

.section-header:hover {
  color: var(--text-primary, #1d1d1f);
}

.chevron {
  font-size: 10px;
  transition: transform 0.2s;
}

.chevron.collapsed {
  transform: rotate(-90deg);
}

.group-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  list-style: none;
  margin: 0;
  padding: 0 4px 8px;
}

.monitored-list {
  border-bottom: 1px solid var(--border-color, #e2e2e6);
  margin-bottom: 4px;
}

.unmonitored-list {
  opacity: 0.8;
}
</style>
