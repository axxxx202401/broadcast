<script setup lang="ts">
import { computed, ref } from 'vue'

import type { AccountSummary } from '../types/im'

/**
 * 头部账号菜单：展示当前邮箱/手机号，并提供切换、添加、退出与移除入口。
 *
 * UID 仅作为次要“用户 ID”展示，不得以顶栏 `UID / ...` 形式出现。
 * `switching` 为真时展示“正在切换账号”并禁用全部账号动作，防止重复切换。
 */
const props = defineProps<{
  /** 当前已登录或选中的账号摘要。 */
  current: AccountSummary
  /** 全部已保存账号摘要；当前账号会从可切换列表中排除。 */
  accounts: AccountSummary[]
  /** 账号切换进行中时为真，用于禁用菜单动作。 */
  switching: boolean
  /** 切换到指定 UID；由根组件接线 `useAccounts.switchAccount`。 */
  switchAccount: (uid: string) => unknown
}>()

const emit = defineEmits<{
  /** 用户选择退出登录，回到登录页并保留当前账号回填。 */
  logout: []
  /** 用户选择添加账号，进入空白邮箱密码登录且不清除索引中的其他账号。 */
  addAccount: []
  /** 用户确认后移除指定 UID；由根组件执行确认后的本地清理与 IPC。 */
  removeAccount: [uid: string]
}>()

/** 菜单面板是否展开；列表节点始终挂载以便测试与键盘可达。 */
const open = ref(false)

/** 可切换的其他已保存账号，排除当前 UID。 */
const otherAccounts = computed(() =>
  props.accounts.filter((account) => account.uid !== props.current.uid),
)

/** 展开或收起菜单；切换中仍允许查看当前账号与提示文案。 */
function toggle() {
  open.value = !open.value
}

/** 点击其他账号触发切换；切换中忽略重复点击。 */
function onSwitch(uid: string) {
  if (props.switching) return
  props.switchAccount(uid)
}

/** 添加账号入口；切换中禁用。 */
function onAddAccount() {
  if (props.switching) return
  emit('addAccount')
}

/** 退出登录；切换中禁用。 */
function onLogout() {
  if (props.switching) return
  emit('logout')
}

/**
 * 移除此账号：先弹出确认，确认后才向上抛出 UID。
 * 取消确认不得发出事件。
 */
function onRemoveAccount() {
  if (props.switching) return
  const confirmed = window.confirm('确定移除此账号？这将删除本地登录信息，但保留聊天数据。')
  if (!confirmed) return
  emit('removeAccount', props.current.uid)
}
</script>

<template>
  <div class="account-menu" data-test="account-menu">
    <!-- 主按钮只显示邮箱或手机号，不展示 UID。 -->
    <button
      class="button ghost compact account-menu-trigger"
      type="button"
      data-test="account-menu-trigger"
      :aria-expanded="open ? 'true' : 'false'"
      @click="toggle"
    >
      {{ current.displayAccount }}
    </button>

    <div v-show="open" class="account-menu-panel" data-test="account-menu-panel" role="menu">
      <p v-if="switching" class="account-menu-status" role="status">正在切换账号</p>

      <div class="account-menu-current">
        <strong>{{ current.displayAccount }}</strong>
        <small>用户 ID {{ current.uid }}</small>
      </div>

      <button
        v-for="account in otherAccounts"
        :key="account.uid"
        class="account-menu-item"
        type="button"
        role="menuitem"
        :data-test="`account-${account.uid}`"
        :disabled="switching"
        @click="onSwitch(account.uid)"
      >
        {{ account.displayAccount }}
      </button>

      <button
        class="account-menu-item"
        type="button"
        role="menuitem"
        data-test="add-account"
        :disabled="switching"
        @click="onAddAccount"
      >
        添加账号
      </button>

      <button
        class="account-menu-item"
        type="button"
        role="menuitem"
        data-test="logout"
        :disabled="switching"
        @click="onLogout"
      >
        退出登录
      </button>

      <button
        class="account-menu-item danger"
        type="button"
        role="menuitem"
        data-test="remove-account"
        :disabled="switching"
        @click="onRemoveAccount"
      >
        移除此账号
      </button>
    </div>
  </div>
</template>
