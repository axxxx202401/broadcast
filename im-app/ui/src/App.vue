<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'

import AccountMenu from './components/AccountMenu.vue'
import GroupSidebar from './components/GroupSidebar.vue'
import LotteryPanel from './components/LotteryPanel.vue'
import LoginPanel from './components/LoginPanel.vue'
import MessagePanel from './components/MessagePanel.vue'
import StatusBadge from './components/StatusBadge.vue'
import { useAccounts } from './composables/useAccounts'
import { useAuth } from './composables/useAuth'
import { useLottery } from './composables/useLottery'
import { useMonitor } from './composables/useMonitor'
import { useResponsiveSidebar } from './composables/useResponsiveSidebar'
import { useTheme } from './composables/useTheme'
import type { RestoreSessionResult } from './types/im'

// 根组件编排账号恢复、认证与监控状态。启动时先恢复上次登录，避免闪现登录页。
const monitor = useMonitor()
const lottery = useLottery(monitor.loggedIn)
const accounts = useAccounts()
/** 窄屏把群列表收成抽屉；宽屏保持侧栏展开。仅已登录工作区使用开关与遮罩。 */
const layout = useResponsiveSidebar()
const theme = useTheme()
const auth = useAuth((payload) => {
  accounts.applyManualLogin(payload.account)
  monitor.acceptLogin(payload.groups, payload.account.uid)
  // 必须在成功路径清空旧 warnings；空数组应回退为 ''。
  monitor.warning.value = payload.warnings.join('\n')
  void monitor.fetchGroups()
})

/** 切换进行中：保持主界面并让 AccountMenu 禁用动作、展示“正在切换账号”。 */
const switching = computed(() => accounts.busy.value === 'switch')

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

/** Escape 关闭窄屏群列表抽屉，不拦截输入框以外的其它快捷键。 */
function onWorkspaceEscape(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    layout.closeSidebar()
  }
}

onMounted(() => {
  void accounts.restore().then(applyRestoreOutcome)
  window.addEventListener('keydown', onWorkspaceEscape)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onWorkspaceEscape)
})

const retryRestore = () => {
  void accounts.retryRestore().then(applyRestoreOutcome)
}

const useOtherAccount = () => {
  accounts.useOtherAccount()
}

/** 退出后进入登录页；保留刚退出账号的输入上下文供再次登录。 */
const logout = () =>
  monitor.logout().finally(() => {
    accounts.phase.value = 'needsLogin'
    if (accounts.selectedAccount.value) {
      auth.selectSavedAccount(accounts.selectedAccount.value)
    } else {
      auth.resetAuthForm({ preserveSelectedAccount: false })
    }
  })

/**
 * 切换到已保存账号；结果与启动恢复共用 `applyRestoreOutcome`。
 * Token 失效时进入目标账号的预填登录页。
 */
const onSwitchAccount = (uid: string) => {
  void accounts.switchAccount(uid).then(applyRestoreOutcome)
}

/**
 * 添加账号：只断开当前 TCP，保留 Token，进入空白邮箱密码登录页。
 * 登录页可返回上一账号并用原 Token 重连；不得调用退出以免删 Token。
 */
const addAccount = () =>
  accounts.beginAddAccount().finally(() => {
    monitor.detachLocalSession()
    auth.resetAuthForm({ preserveSelectedAccount: false })
  })

/** 从添加账号登录页返回上一账号；Token 有效则恢复主界面，失效则进入该账号登录页。 */
const onReturnFromAddAccount = () => {
  void accounts.returnFromAddAccount().then(applyRestoreOutcome)
}

/**
 * 移除当前账号：先退出会话，再删除索引与凭据。
 * 有剩余账号时按 `nextUid`（索引 `last_used_uid`）回填登录页；没有则空白邮箱密码表单。
 * 不得用刷新列表的首项冒充最近使用账号。
 */
const removeCurrentAccount = async (uid: string) => {
  await monitor.logout()
  accounts.phase.value = 'needsLogin'
  const result = await accounts.removeAccount(uid)
  if (!result) return
  if (result.warnings.length) {
    monitor.warning.value = result.warnings.join('\n')
  }
  const nextUid = result.nextUid
  const next = nextUid
    ? accounts.accounts.value.find((item) => item.uid === nextUid) ?? null
    : null
  if (next) {
    auth.selectSavedAccount(next)
  } else {
    auth.resetAuthForm({ preserveSelectedAccount: false })
  }
}

/** 选择单个群后交给监控状态，窄屏再关闭抽屉以免挡住消息区。 */
function onSelectGroup(groupId: string) {
  monitor.selectGroup(groupId)
  layout.selectGroup()
}

/** 回到全部群消息汇总，窄屏同样关闭抽屉。 */
function onShowAllMessages() {
  monitor.showAllMessages()
  layout.selectGroup()
}
</script>

<template>
  <!-- 警告独立于登录状态展示，允许用户关闭；错误则在已登录控制台内反馈。 -->
  <div v-if="monitor.warning.value" class="global-error global-warning" role="status">
    <span>警告</span>
    <p>{{ monitor.warning.value }}</p>
    <button type="button" aria-label="关闭警告" @click="monitor.warning.value = ''">×</button>
  </div>

  <!-- 恢复完成前（含可重试）只显示启动状态；账号切换中 busy=switch 时保留主界面。 -->
  <section
    v-if="accounts.phase.value === 'recovering' && !switching"
    class="restore-shell"
    role="status"
  >
    <p>{{ accounts.lastAccountOp.value === 'switch' ? '正在切换账号' : '正在恢复上次登录' }}</p>
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
  <LoginPanel
    v-else-if="accounts.phase.value === 'needsLogin' || !monitor.loggedIn.value"
    :auth="auth"
    :accounts="accounts.accounts.value"
    :selectedAccountUid="auth.selectedAccountUid.value"
    :can-return="!!accounts.returnToUid.value"
    @back="onReturnFromAddAccount"
  />

  <main v-else class="operations-shell">
    <!-- 顶栏集中呈现连接状态、当前账号菜单及连接操作。 -->
    <header class="topbar">
      <!-- 窄屏优先展示消息区，用顶部按钮展开群列表抽屉。 -->
      <button
        v-if="layout.isNarrow.value"
        class="button ghost compact sidebar-toggle"
        type="button"
        :aria-expanded="layout.sidebarOpen.value"
        aria-controls="group-sidebar-drawer"
        @click="layout.toggleSidebar"
      >群列表</button>
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">IM</span>
        <div>
          <strong>实时监控控制台</strong>
        </div>
      </div>
      <div class="topbar-actions">
        <button
          class="icon-button"
          type="button"
          aria-label="切换日夜主题"
          @click="theme.toggle"
        >
          <span v-if="theme.isLight.value" aria-hidden="true">☀</span>
          <span v-else aria-hidden="true">☾</span>
        </button>
        <StatusBadge :status="monitor.connectionStatus.value" />
        <AccountMenu
          v-if="accounts.selectedAccount.value"
          :current="accounts.selectedAccount.value"
          :accounts="accounts.accounts.value"
          :switching="switching"
          :switch-account="onSwitchAccount"
          @logout="logout"
          @add-account="addAccount"
          @remove-account="removeCurrentAccount"
        />
        <button
          v-if="monitor.connectionStatus.value === 'connected'"
          class="button danger compact"
          type="button"
          :disabled="!!monitor.pending.value"
          @click="monitor.disconnect"
        >断开连接</button>
        <button
          v-else
          class="button primary compact"
          type="button"
          :disabled="monitor.connectDisabled.value"
          @click="monitor.connect"
        >{{ monitor.connectionStatus.value === 'connecting' ? '连接中…' : '连接聊天' }}</button>
      </div>
    </header>

    <!-- 错误仅属于已登录控制台，由用户确认后关闭。 -->
    <div v-if="monitor.error.value" class="global-error" role="alert">
      <span>错误</span>
      <p>{{ monitor.error.value }}</p>
      <button type="button" aria-label="关闭错误" @click="monitor.error.value = ''">×</button>
    </div>

    <!-- 工作区由群组筛选与监控操作、当前群消息流两部分组成；窄屏侧栏改为遮罩抽屉。 -->
    <div
      class="workspace"
      :class="{
        'is-narrow': layout.isNarrow.value,
        'is-sidebar-open': layout.sidebarOpen.value,
        'is-sidebar-collapsed': layout.sidebarCollapsed.value,
      }"
    >
      <div
        v-if="layout.isNarrow.value && layout.sidebarOpen.value"
        class="sidebar-mask"
        @click="layout.closeSidebar"
      />
      <div
        id="group-sidebar-drawer"
        class="sidebar-drawer"
        :inert="layout.isNarrow.value && !layout.sidebarOpen.value"
        :aria-hidden="layout.isNarrow.value && !layout.sidebarOpen.value ? true : undefined"
      >
        <GroupSidebar
          :groups="monitor.filteredGroups.value"
          :total="monitor.groups.value.length"
          :monitored-count="monitor.monitoredCount.value"
          :selected-id="monitor.selectedGroup.value?.group_id ?? null"
          :search="monitor.search.value"
          :pending="monitor.pending.value"
          :show-matched-only="monitor.showMatchedOnly.value"
          :collapsed="layout.sidebarCollapsed.value"
          @update:search="monitor.search.value = $event"
          @select="onSelectGroup"
          @select-all="onShowAllMessages"
          @toggle="monitor.toggleGroup"
          @refresh="monitor.refreshGroups"
          @update:show-matched-only="monitor.showMatchedOnly.value = $event"
          @collapse="layout.toggleCollapsed"
        />
      </div>
      <MessagePanel
        :group="monitor.selectedGroup.value"
        :messages="monitor.filteredMessages.value"
        :loading="monitor.messagesLoading.value"
        :has-older="monitor.hasOlder.value"
        :loading-older="monitor.loadingOlder.value"
        :older-request-token="monitor.olderRequestToken.value"
        :monitored-group-ids="monitor.monitoredGroupIds.value"
        :total-groups="monitor.groups.value.length"
        :monitored-count="monitor.monitoredCount.value"
        :lottery="lottery"
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
