<script setup lang="ts">
import { computed, ref } from 'vue'

/**
 * 在「全部群消息」标题下展示正在监控的 groupId。
 * 默认最多显示 5 个，超出部分折叠；展开后可收起。空列表提示尚未选择监控群。
 */
const props = defineProps<{
  /** 正在监控的群 ID，顺序由调用方保证。 */
  groupIds: string[]
}>()

const expanded = ref(false)
const visibleIds = computed(() =>
  expanded.value ? props.groupIds : props.groupIds.slice(0, 5),
)
const hiddenCount = computed(() => Math.max(0, props.groupIds.length - 5))
</script>

<template>
  <div class="monitored-group-summary">
    <p v-if="groupIds.length === 0" class="empty">尚未选择监控群</p>
    <template v-else>
      <ul class="id-list">
        <li v-for="id in visibleIds" :key="id">#{{ id }}</li>
      </ul>
      <button
        v-if="hiddenCount > 0"
        type="button"
        @click="expanded = !expanded"
      >
        {{ expanded ? '收起' : `另有 ${hiddenCount} 个，展开全部` }}
      </button>
    </template>
  </div>
</template>

<style scoped>
.monitored-group-summary {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}

.empty,
.id-list,
button {
  margin: 0;
  color: var(--text-500);
  font-size: 12px;
}

.id-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 10px;
  padding: 0;
  list-style: none;
}

button {
  padding: 0;
  background: none;
  border: 0;
  color: var(--text-300, var(--text-500));
  cursor: pointer;
}
</style>
