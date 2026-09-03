<script setup lang="ts">
import { computed, onUnmounted } from 'vue'

import type { useAuth } from '../composables/useAuth'
import type { AccountSummary, PrimaryLoginType } from '../types/im'

/**
 * 登录卡片：展示已保存账号、主登录字段、折叠的其他方式，以及二次验证。
 * 不渲染协议字段名、令牌或常驻 GT4 状态条。
 */
const props = defineProps<{
  auth: ReturnType<typeof useAuth>
  accounts: AccountSummary[]
  selectedAccountUid: string | null
}>()

// 离开登录页时调用 destroyGt4 清理入口；具体实例与 DOM 清理由认证流程负责。
onUnmounted(props.auth.destroyGt4)

// 服务端验证类型决定字段文案；密码类验证还决定输入框的遮蔽与自动填充语义。
const isPasswordValidation = (type?: number) =>
  type === 18 || type === 20 || type === 21

const validateTypeLabels: Partial<Record<number, string>> = {
  16: '邮箱验证码',
  17: '手机验证码',
  18: '交易密码',
  19: '谷歌验证码',
  20: '手机登录密码',
  21: '邮箱登录密码',
  22: '人脸凭据',
  23: 'Messenger 验证码',
  24: '辅助验证',
  25: 'iToken 验证码',
  26: 'iToken 生物验证',
}

const isPhoneMethod = computed(() =>
  props.auth.loginMethod.value === 1 || props.auth.loginMethod.value === 3,
)

/** 从已保存账号列表回填或清空为「添加账号」。 */
function onAccountChange(event: Event) {
  const uid = (event.target as HTMLSelectElement).value
  if (!uid) {
    props.auth.resetAuthForm({ preserveSelectedAccount: false })
    return
  }
  const account = props.accounts.find((acc) => acc.uid === uid)
  if (account) props.auth.selectSavedAccount(account)
}

/** 展开区选中一种登录方式后收起面板；身份对账由 useAuth 负责，此处不得恢复 saved。 */
function chooseLoginMethod(method: PrimaryLoginType) {
  props.auth.loginMethod.value = method
  props.auth.validateValue.value = ''
  props.auth.otherMethodsOpen.value = false
}

const canSubmitPrimary = computed(() => {
  if (props.auth.busy.value) return false
  if (!props.auth.accountReady.value) return false
  if (props.auth.isCodeMode.value) return !!props.auth.validateValue.value.trim()
  if (props.auth.passwordMode.value === 'saved') return true
  return !!props.auth.validateValue.value.trim()
})
</script>

<template>
  <!-- 居中单列登录卡片：Logo、账号、主字段、登录，其他方式折叠在按钮之后。 -->
  <main class="login-shell">
    <section class="login-card" aria-label="登录">
      <header class="login-header">
        <img src="/icon.svg" alt="IM" class="login-logo" />
        <p class="purpose">进入实时监控控制台</p>
      </header>

      <!-- 主登录：账号选择、邮箱或手机、密码哨兵、提交。 -->
      <template v-if="!auth.challengePending.value.length">
        <div v-if="props.accounts.length" class="account-picker">
          <label>
            <span>已保存账号</span>
            <select :value="auth.selectedAccountUid.value ?? ''" @change="onAccountChange">
              <option value="">添加账号</option>
              <option v-for="acc in props.accounts" :key="acc.uid" :value="acc.uid">
                {{ acc.displayAccount }}
              </option>
            </select>
          </label>
        </div>

        <form class="login-form" @submit.prevent="auth.submitLogin">
          <div class="field-grid">
            <label v-if="isPhoneMethod">
              <span>国家区号</span>
              <input v-model.number="auth.countryCode.value" type="number" inputmode="numeric" />
            </label>
            <label>
              <span>{{ isPhoneMethod ? '手机号' : '邮箱地址' }}</span>
              <input
                v-model.trim="auth.account.value"
                :type="isPhoneMethod ? 'tel' : 'email'"
                :autocomplete="isPhoneMethod ? 'tel' : 'email'"
                :placeholder="isPhoneMethod ? '输入手机号' : '输入邮箱地址'"
                required
              />
            </label>
          </div>

          <label>
            <span>{{ auth.isCodeMode.value ? '验证码' : '登录密码' }}</span>
            <span
              v-if="!auth.isCodeMode.value && auth.passwordMode.value === 'saved'"
              class="password-sentinel"
            >已保存密码</span>
            <input
              v-model.trim="auth.validateValue.value"
              :type="auth.isCodeMode.value ? 'text' : 'password'"
              :inputmode="auth.isCodeMode.value ? 'numeric' : 'text'"
              :autocomplete="auth.isCodeMode.value ? 'one-time-code' : 'current-password'"
              :required="auth.isCodeMode.value || auth.passwordMode.value !== 'saved'"
            />
          </label>

          <section v-if="auth.isCodeMode.value" class="code-send">
            <button
              class="button secondary"
              data-test="send-code"
              type="button"
              :disabled="!!auth.busy.value || auth.gt4Loading.value || !auth.accountReady.value"
              @click="auth.sendCode"
            >
              {{ auth.busy.value === 'captcha' ? '等待验证…' : auth.busy.value === 'code' ? '发送中…' : '发送验证码' }}
            </button>
            <p v-if="auth.gt4Error.value" class="warning-note">{{ auth.gt4Error.value }}</p>
          </section>

          <button
            class="button primary login-submit"
            type="submit"
            :disabled="!canSubmitPrimary"
          >
            登录
          </button>
        </form>

        <div class="other-methods">
          <button
            type="button"
            data-test="toggle-other-methods"
            class="button ghost"
            @click="auth.toggleOtherMethods"
          >
            其他登录方式
          </button>

          <div v-if="auth.otherMethodsOpen.value" class="other-methods-panel">
            <button type="button" class="button secondary" @click="chooseLoginMethod(3)">手机号密码</button>
            <button type="button" class="button secondary" @click="chooseLoginMethod(1)">手机号验证码</button>
            <button type="button" class="button secondary" @click="chooseLoginMethod(2)">邮箱验证码</button>
          </div>
        </div>
      </template>

      <!-- 二次验证：展示待验证项并允许提交挑战值，不展示协议字段名。 -->
      <template v-else>
        <section class="challenge-step" aria-label="二次验证">
          <h3>二次验证</h3>
          <div class="pending-list" aria-label="服务端待验证项">
            <label
              v-for="item in auth.challengePending.value"
              :key="`${item.validateType}-${item.account ?? ''}`"
            >
              <input
                v-model.number="auth.selectedChallengeType.value"
                type="radio"
                name="pending-validation"
                :value="item.validateType"
              />
              <span>
                {{ validateTypeLabels[item.validateType] ?? '验证值' }}
                · {{ item.account ?? '服务端未提供账号' }}
              </span>
            </label>
          </div>

          <section v-if="auth.isChallengeCode.value" class="challenge-code">
            <button
              class="button secondary"
              data-test="challenge-send-code"
              type="button"
              :disabled="!!auth.busy.value || auth.gt4Loading.value"
              @click="auth.sendChallengeCode"
            >
              {{
                auth.selectedChallenge.value?.validateType === 16
                  ? '发送邮箱验证码'
                  : '发送手机验证码'
              }}
            </button>
            <p v-if="auth.gt4Error.value" class="warning-note">{{ auth.gt4Error.value }}</p>
          </section>

          <label>
            <span>
              {{
                isPasswordValidation(auth.selectedChallenge.value?.validateType)
                  ? (validateTypeLabels[auth.selectedChallenge.value?.validateType as number] ?? '验证值')
                  : '验证值'
              }}
            </span>
            <input
              v-model.trim="auth.challengeValue.value"
              data-test="challenge-value"
              :type="isPasswordValidation(auth.selectedChallenge.value?.validateType) ? 'password' : 'text'"
              :autocomplete="isPasswordValidation(auth.selectedChallenge.value?.validateType) ? 'current-password' : 'one-time-code'"
              required
            />
          </label>

          <button
            class="button primary login-submit"
            data-test="challenge-submit"
            type="button"
            :disabled="!!auth.busy.value || !auth.selectedChallenge.value || !auth.challengeValue.value.trim()"
            @click="auth.submitChallenge"
          >
            完成二次验证
          </button>

          <section v-if="auth.businessProcessing.value.length" class="business-processing" role="status" aria-live="polite">
            <p
              v-for="item in auth.businessProcessing.value"
              :key="item.businessCode"
            >
              {{ item.businessMsg || '服务端未提供消息' }}
            </p>
          </section>
        </section>
      </template>

      <p v-if="auth.error.value" class="feedback error" role="alert">{{ auth.error.value }}</p>
      <p v-if="auth.notice.value" class="feedback notice" role="status">{{ auth.notice.value }}</p>
    </section>
  </main>
</template>
