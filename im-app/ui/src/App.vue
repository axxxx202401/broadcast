<script setup lang="ts">
import { onMounted } from 'vue'

import GroupSidebar from './components/GroupSidebar.vue'
import LoginPanel from './components/LoginPanel.vue'
import MessagePanel from './components/MessagePanel.vue'
import StatusBadge from './components/StatusBadge.vue'
import { useAccounts } from './composables/useAccounts'
import { useAuth } from './composables/useAuth'
import { useMonitor } from './composables/useMonitor'
import type { RestoreSessionResult } from './types/im'

// 根组件编排账号恢复、认证与监控状态。启动时先恢复上次登录，避免闪现登录页。
const monitor = useMonitor()
const accounts = useAccounts()
const auth = useAuth((payload) => {
  accounts.applyManualLogin(payload.account)
  monitor.acceptLogin(payload.groups, payload.account.uid)
  // 必须在成功路径清空旧 warnings；空数组应回退为 ''。
  monitor.warning.value = payload.warnings.join('\n')
  void monitor.fetchGroups()
})

/** 把恢复/切换结果发布到监控会话或登录回填；过期结果为 `null` 时忽略。 */
function applyRestoreOutcome(result: RestoreSessionResult | null) {
  if (!result) return
  if (result.status === 'success') {
    monitor.acceptLogin(result.groups, result.account.uid)
    // 必须在成功路径清空旧 warnings；空数组应回退为 ''。
    monitor.warning.value = result.warnings.join('\n')
    void monitor.fetchGroups()
    return
  }
  if (result.status === 'needsLogin') {
    auth.resetAuthForm({ preserveSelectedAccount: true })
    auth.selectSavedAccount({
      uid: result.uid,
      displayAccount: result.displayAccount,
      loginType: result.loginType,
      hasSavedPassword: result.hasSavedPassword,
      isCurrent: false,
    })
  }
  if (result.status === 'noAccount') {
    auth.resetAuthForm({ preserveSelectedAccount: false })
  }
}

onMounted(() => {
  void accounts.restore().then(applyRestoreOutcome)
})

const retryRestore = () => {
  void accounts.retryRestore().then(applyRestoreOutcome)
}

const useOtherAccount = () => {
  accounts.useOtherAccount()
}

/** 退出后进入登录页时必须重置 auth 的瞬态状态。 */
const logout = () =>
  monitor.logout().finally(() => {
    accounts.phase.value = 'needsLogin'
    if (accounts.selectedAccount.value) {
      // Task 8 会补全“退出后选中账号”的菜单体验；这里至少保持刚退出账号的输入上下文。
      auth.selectSavedAccount(accounts.selectedAccount.value)
    } else {
      auth.resetAuthForm({ preserveSelectedAccount: false })
    }
  })
</script>

<template>
  <!-- 警告独立于登录状态展示，允许操作员关闭；错误则在控制台内作为命令失败反馈。 -->
  <div v-if="monitor.warning.value" class="global-error global-warning" role="status">
    <span>COMMAND WARNING</span>
    <p>{{ monitor.warning.value }}</p>
    <button type="button" aria-label="关闭警告" @click="monitor.warning.value = ''">×</button>
  </div>

  <!-- 恢复完成前（含可重试）只显示启动状态，不得闪现登录页或操作主界面。 -->
  <section v-if="accounts.phase.value === 'recovering'" class="restore-shell" role="status">
    <p>正在恢复上次登录</p>
    <p v-if="accounts.retryableMessage.value">{{ accounts.retryableMessage.value }}</p>
    <div v-if="accounts.retryableMessage.value" class="restore-actions">
      <button
        class="button primary"
        type="button"
        data-test="retry-restore"
        :disabled="!!accounts.busy.value"
        @click="retryRestore"
      >重试</button>
      <button
        class="button ghost"
        type="button"
        data-test="use-other-account"
        :disabled="!!accounts.busy.value"
        @click="useOtherAccount"
      >使用其他账号</button>
    </div>
  </section>

  <!-- 无账号或需要重新登录时展示登录入口；成功恢复后进入监控控制台。 -->
  <LoginPanel v-else-if="accounts.phase.value === 'needsLogin' || !monitor.loggedIn.value" :auth="auth" />

  <main v-else class="operations-shell">
    <!-- 顶栏集中呈现连接状态、操作员身份及连接和退出操作。 -->
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
        <button
          class="button ghost compact"
          type="button"
          :disabled="!!monitor.pending.value"
          @click="logout"
        >
          退出
        </button>
      </div>
    </header>

    <!-- 命令错误仅属于已登录控制台，由操作员确认后关闭。 -->
    <div v-if="monitor.error.value" class="global-error" role="alert">
      <span>COMMAND ERROR</span>
      <p>{{ monitor.error.value }}</p>
      <button type="button" aria-label="关闭错误" @click="monitor.error.value = ''">×</button>
    </div>

    <!-- 工作区由群组筛选与监控操作、当前群消息流两部分组成。 -->
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
        @select-all="monitor.showAllMessages"
        @toggle="monitor.toggleGroup"
        @refresh="monitor.refreshGroups"
      />
      <MessagePanel
        :group="monitor.selectedGroup.value"
        :messages="monitor.messages.value"
        :loading="monitor.messagesLoading.value"
        :has-older="monitor.hasOlder.value"
        :loading-older="monitor.loadingOlder.value"
        :older-request-token="monitor.olderRequestToken.value"
        @load-older="monitor.loadOlderMessages"
        @older-settled="monitor.handleOlderSettled"
      />
    </div>
  </main>
</template>

<style scoped>
.restore-shell {
  display: grid;
  place-content: center;
  gap: 16px;
  min-height: 100%;
  padding: 48px 24px;
  text-align: center;
}

.restore-actions {
  display: flex;
  justify-content: center;
  gap: 12px;
}
</style>
