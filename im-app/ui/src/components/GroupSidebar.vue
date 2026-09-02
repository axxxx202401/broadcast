<script setup lang="ts">
import type { GroupDto } from '../types/im'

defineProps<{
  groups: GroupDto[]
  total: number
  monitoredCount: number
  selectedId: string | null
  search: string
  pending: string | null
}>()

defineEmits<{
  'update:search': [value: string]
  select: [groupId: string]
  toggle: [group: GroupDto]
  refresh: []
}>()
</script>

<template>
  <aside class="group-sidebar" aria-label="群组监控列表">
    <div class="sidebar-metrics">
      <div><span>群组总数</span><strong>{{ total }}</strong></div>
      <div><span>监控中</span><strong class="metric-live">{{ monitoredCount }}</strong></div>
    </div>
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
    </div>

    <div class="list-label"><span>ALL CHANNELS</span><span>{{ groups.length }}</span></div>
    <ul class="group-list" aria-label="群组">
      <li
        v-for="group in groups"
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
      <li v-if="groups.length === 0" class="compact-empty">
        <span>NO MATCH</span>
        <p>没有匹配的群组</p>
      </li>
    </ul>
  </aside>
</template>
