<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'

import type { useAuth } from '../composables/useAuth'
import type { AccountSummary, PrimaryLoginType } from '../types/im'

/**
 * 登录卡片：展示已保存账号、单行登录方式 tab、主登录字段与二次验证。
 * 从主界面「添加账号」进入时显示返回；不渲染协议字段名、令牌或滑块失败文案。
 */
const props = defineProps<{
  auth: ReturnType<typeof useAuth>
  accounts: AccountSummary[]
  selectedAccountUid: string | null
  /** 仅从已登录主界面添加账号进入时为 true，用于显示返回上一页。 */
  canReturn?: boolean
}>()

const emit = defineEmits<{
  /** 返回添加账号之前的主界面；由根组件用原 Token 恢复会话。 */
  back: []
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

/** 主登录方式单行 tab；顺序固定为邮箱优先。 */
const loginMethodTabs: { method: PrimaryLoginType; label: string }[] = [
  { method: 4, label: '邮箱密码' },
  { method: 2, label: '邮箱验证码' },
  { method: 3, label: '手机密码' },
  { method: 1, label: '手机验证码' },
]

/** 多种待验证方式时，展开“改用其他验证方式”后才显示选项列表。 */
const choosingOtherMethods = ref(false)

const challengeMethodLabel = computed(() => {
  const type = props.auth.selectedChallenge.value?.validateType
  return type == null ? '验证' : (validateTypeLabels[type] ?? '安全验证')
})

const showChallengeSecretInput = computed(() => {
  const type = props.auth.selectedChallenge.value?.validateType
  if (type === 20 || type === 21) {
    return props.auth.passwordReuseAttempted.value || props.auth.passwordReuseFailed.value
  }
  return true
})

const needsSupplementedTarget = computed(() => props.auth.needsSupplementedTarget.value)

const challengeValueAutocomplete = computed(() => {
  const type = props.auth.selectedChallenge.value?.validateType
  if (type === 20 || type === 21) return 'current-password'
  return 'off'
})

const challengeTotalKnown = computed(() => {
  const completed = props.auth.completedChallengeKeys.value.length
  const remaining = props.auth.challengePending.value.length
  // 仅当已经完成过至少一项且当前仍有待办时，才能把“已完成 + 当前待办”视为已知总数。
  return completed > 0 && remaining > 0 ? completed + remaining : null
})

watch(() => props.auth.challengePending.value.length, (length) => {
  if (length === 0) choosingOtherMethods.value = false
})

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

/** 选中一种登录方式；身份对账由 useAuth 负责，此处不得恢复 saved。 */
function chooseLoginMethod(method: PrimaryLoginType) {
  props.auth.loginMethod.value = method
  props.auth.validateValue.value = ''
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
  <!-- 居中宽表单登录卡片：Logo、账号、主字段、登录，其他方式始终可见。 -->
  <main class="login-shell">
    <section class="login-card" aria-label="登录">
      <button
        v-if="props.canReturn"
        class="button ghost login-back"
        data-test="login-back"
        type="button"
        @click="emit('back')"
      >
        返回
      </button>
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

        <div
          class="login-primary-panel"
        >
          <div
            class="login-method-tabs"
            role="tablist"
            aria-label="登录方式"
          >
            <button
              v-for="tab in loginMethodTabs"
              :key="tab.method"
              type="button"
              role="tab"
              class="login-method-tab"
              data-test="login-method-tab"
              :class="{ 'is-active': auth.loginMethod.value === tab.method }"
              :aria-selected="auth.loginMethod.value === tab.method"
              @click="chooseLoginMethod(tab.method)"
            >
              {{ tab.label }}
            </button>
          </div>

          <form class="login-form" @submit.prevent="auth.submitLogin">
          <div class="login-form-fields">
            <div class="account-row" :class="{ 'is-phone': isPhoneMethod }">
              <label v-if="isPhoneMethod" class="country-code-cell">
                <span>国家区号</span>
                <input
                  v-model.number="auth.countryCode.value"
                  type="text"
                  inputmode="numeric"
                  pattern="[0-9]*"
                  autocomplete="tel-country-code"
                />
              </label>
              <label class="account-cell">
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

            <label class="secret-field">
              <span>{{ auth.isCodeMode.value ? '验证码' : '登录密码' }}</span>
              <span
                class="password-sentinel"
                :class="{ 'is-visible': !auth.isCodeMode.value && auth.passwordMode.value === 'saved' }"
                aria-hidden="true"
              >已保存密码</span>
              <div class="field-control" :class="{ 'code-input-row': auth.isCodeMode.value }">
                <input
                  v-model.trim="auth.validateValue.value"
                  :type="auth.isCodeMode.value ? 'text' : 'password'"
                  :inputmode="auth.isCodeMode.value ? 'numeric' : 'text'"
                  :autocomplete="auth.isCodeMode.value ? 'one-time-code' : 'current-password'"
                  :required="auth.isCodeMode.value || auth.passwordMode.value !== 'saved'"
                />
                <button
                  v-if="auth.isCodeMode.value"
                  class="button secondary code-send-inline"
                  data-test="send-code"
                  type="button"
                  :disabled="!!auth.busy.value || auth.gt4Loading.value || !auth.accountReady.value"
                  @click="auth.sendCode"
                >
                  {{ auth.busy.value === 'captcha' ? '等待验证…' : auth.busy.value === 'code' ? '发送中…' : '发送验证码' }}
                </button>
              </div>
            </label>
          </div>

          <button
            class="button primary login-submit"
            type="submit"
            :disabled="!canSubmitPrimary"
          >
            登录
          </button>
        </form>
        </div>
      </template>

      <!-- 二次验证：只展示用户可理解的步骤与方式，不展示协议字段、业务码或 GT4 状态。 -->
      <template v-else>
        <section class="challenge-step" aria-label="二次验证">
          <h3>还差一步，请确认是你本人</h3>
          <p class="challenge-progress">
            安全验证第 {{ auth.challengeStep.value || 1 }} 步
            <template v-if="challengeTotalKnown !== null">
              （共 {{ challengeTotalKnown }} 步）
            </template>
          </p>
          <p class="challenge-method">{{ challengeMethodLabel }}</p>
          <p
            v-if="auth.selectedChallenge.value?.account"
            class="challenge-target"
          >
            {{ auth.selectedChallenge.value.account }}
          </p>

          <button
            v-if="auth.challengePending.value.length > 1"
            class="button ghost"
            data-test="challenge-switch"
            type="button"
            @click="choosingOtherMethods = !choosingOtherMethods"
          >
            改用其他验证方式
          </button>

          <div
            v-if="choosingOtherMethods && auth.challengePending.value.length > 1"
            class="pending-list"
            aria-label="其他验证方式"
          >
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
              <span>{{ validateTypeLabels[item.validateType] ?? '安全验证' }}</span>
            </label>
          </div>

          <label v-if="needsSupplementedTarget">
            <span>补充完整{{ auth.selectedChallenge.value?.validateType === 17 ? '手机号' : '邮箱' }}</span>
            <input
              v-model.trim="auth.supplementedTarget.value"
              data-test="challenge-supplement"
              :type="auth.selectedChallenge.value?.validateType === 17 ? 'tel' : 'email'"
              autocomplete="off"
            />
          </label>

          <label v-if="showChallengeSecretInput">
            <span>
              {{
                isPasswordValidation(auth.selectedChallenge.value?.validateType)
                  ? (validateTypeLabels[auth.selectedChallenge.value?.validateType as number] ?? '验证值')
                  : '验证值'
              }}
            </span>
            <div class="field-control" :class="{ 'code-input-row': auth.isChallengeCode.value }">
              <input
                v-model.trim="auth.challengeValue.value"
                data-test="challenge-value"
                :type="isPasswordValidation(auth.selectedChallenge.value?.validateType) ? 'password' : 'text'"
                :autocomplete="challengeValueAutocomplete"
                required
              />
              <button
                v-if="auth.isChallengeCode.value"
                class="button secondary code-send-inline"
                data-test="challenge-send-code"
                type="button"
                :disabled="!!auth.busy.value || auth.gt4Loading.value || auth.resendSeconds.value > 0"
                @click="auth.sendChallengeCode"
              >
                {{
                  auth.resendSeconds.value > 0
                    ? `${auth.resendSeconds.value}s 后可重发`
                    : auth.selectedChallenge.value?.validateType === 16
                      ? '发送邮箱验证码'
                      : '发送手机验证码'
                }}
              </button>
            </div>
          </label>

          <button
            v-if="showChallengeSecretInput"
            class="button primary login-submit"
            data-test="challenge-submit"
            type="button"
            :disabled="!!auth.busy.value || !auth.selectedChallenge.value || !auth.challengeValue.value.trim()"
            @click="auth.submitChallenge"
          >
            完成验证
          </button>

          <button
            class="button ghost"
            data-test="challenge-back"
            type="button"
            @click="auth.resetChallenge"
          >
            返回登录
          </button>
        </section>
      </template>

      <p v-if="auth.error.value" class="feedback error" role="alert">{{ auth.error.value }}</p>
      <p v-if="auth.notice.value" class="feedback notice" role="status">{{ auth.notice.value }}</p>
    </section>
  </main>
</template>
