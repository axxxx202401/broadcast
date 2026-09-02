<script setup lang="ts">
import GroupSidebar from './components/GroupSidebar.vue'
import LoginPanel from './components/LoginPanel.vue'
import MessagePanel from './components/MessagePanel.vue'
import StatusBadge from './components/StatusBadge.vue'
import { useAuth } from './composables/useAuth'
import { useMonitor } from './composables/useMonitor'

const monitor = useMonitor()
const auth = useAuth((groups, uid) => {
  monitor.acceptLogin(groups, uid)
  void monitor.fetchGroups()
})
</script>

<template>
  <div v-if="monitor.warning.value" class="global-error global-warning" role="status">
    <span>COMMAND WARNING</span>
    <p>{{ monitor.warning.value }}</p>
    <button type="button" aria-label="关闭警告" @click="monitor.warning.value = ''">×</button>
  </div>

  <LoginPanel v-if="!monitor.loggedIn.value" :auth="auth" />

  <main v-else class="operations-shell">
    <header class="topbar">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">IM</span>
        <div>
          <strong>实时监控控制台</strong>
          <small>INDUSTRIAL MESSAGE OBSERVATORY</small>
        </div>
      </div>
      <div class="topbar-actions">
        <StatusBadge :status="monitor.connectionStatus.value" />
        <span class="operator-id">UID / {{ monitor.uid.value }}</span>
        <button
          v-if="monitor.connectionStatus.value === 'connected'"
          class="button danger compact"
          type="button"
          :disabled="!!monitor.pending.value"
          @click="monitor.disconnect"
        >断开链路</button>
        <button
          v-else
          class="button primary compact"
          type="button"
          :disabled="monitor.connectDisabled.value"
          @click="monitor.connect"
        >{{ monitor.connectionStatus.value === 'connecting' ? '连接中…' : '连接聊天' }}</button>
        <button class="button ghost compact" type="button" :disabled="!!monitor.pending.value" @click="monitor.logout">
          退出
        </button>
      </div>
    </header>

    <div v-if="monitor.error.value" class="global-error" role="alert">
      <span>COMMAND ERROR</span>
      <p>{{ monitor.error.value }}</p>
      <button type="button" aria-label="关闭错误" @click="monitor.error.value = ''">×</button>
    </div>

    <div class="workspace">
      <GroupSidebar
        :groups="monitor.filteredGroups.value"
        :total="monitor.groups.value.length"
        :monitored-count="monitor.monitoredCount.value"
        :selected-id="monitor.selectedGroup.value?.group_id ?? null"
        :search="monitor.search.value"
        :pending="monitor.pending.value"
        @update:search="monitor.search.value = $event"
        @select="monitor.selectGroup"
        @toggle="monitor.toggleGroup"
        @refresh="monitor.refreshGroups"
      />
      <MessagePanel
        :group="monitor.selectedGroup.value"
        :messages="monitor.messages.value"
        :loading="monitor.messagesLoading.value"
      />
    </div>
  </main>
</template>
