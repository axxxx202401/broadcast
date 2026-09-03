<script setup lang="ts">
import { computed, onUnmounted } from 'vue'

import type { useAuth } from '../composables/useAuth'
import type { AccountSummary, PrimaryLoginType, ValidateType } from '../types/im'

// 认证组合式对象由父组件注入，面板只负责呈现状态并转发用户操作。
const props = defineProps<{
  auth: ReturnType<typeof useAuth>
  accounts: AccountSummary[]
  selectedAccountUid: string | null
}>()

// 离开登录页时调用 destroyGt4 清理入口；具体实例与 DOM 清理由认证流程负责。
onUnmounted(props.auth.destroyGt4)

// 服务端验证类型决定字段文案；密码类验证还决定输入框的遮蔽与自动填充语义。
const isPasswordValidation = (type?: ValidateType) =>
  type === 18 || type === 20 || type === 21

const validateTypeLabels: Partial<Record<ValidateType, string>> = {
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

const selectedAccount = computed(() =>
  props.accounts.find((acc) => acc.uid === props.auth.selectedAccountUid.value) ?? null,
)

function onAccountChange(event: Event) {
  const uid = (event.target as HTMLSelectElement).value
  if (!uid) {
    props.auth.resetAuthForm({ preserveSelectedAccount: false })
    return
  }
  const account = props.accounts.find((acc) => acc.uid === uid)
  if (account) props.auth.selectSavedAccount(account)
}

function chooseLoginMethod(method: PrimaryLoginType) {
  props.auth.loginMethod.value = method
  props.auth.validateValue.value = ''
  props.auth.otherMethodsOpen.value = false

  // 密码类登录根据当前已选账号是否具备已保存密码调整哨兵。
  if (method === 3 || method === 4) {
    props.auth.passwordMode.value = selectedAccount.value?.hasSavedPassword ? 'saved' : 'empty'
  } else {
    props.auth.passwordMode.value = 'empty'
  }
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
  <!-- 登录页分为产品说明和认证操作台。 -->
  <main class="login-shell">
    <section class="login-card" aria-label="登录">
      <header class="login-header">
        <img src="/icon.svg" alt="IM" class="login-logo" />
        <p class="purpose">进入实时监控控制台</p>
      </header>

      <!-- 主登录：收集账号、密码/验证码并提交。 -->
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
      </template>

      <!-- 二次验证：展示待验证项并允许提交挑战值。 -->
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
                  ? validateTypeLabels[auth.selectedChallenge.value!.validateType]
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

          <p v-if="auth.error.value" class="feedback error" role="alert">{{ auth.error.value }}</p>
          <p v-if="auth.notice.value" class="feedback notice" role="status">{{ auth.notice.value }}</p>
        </section>
      </template>
    </section>

    <div v-if="false">
      <section class="login-intro" aria-labelledby="product-title">
      <p class="eyebrow">OPERATIONS TERMINAL / 01</p>
      <h1 id="product-title">IM 实时监控<br />控制台</h1>
      <p class="intro-copy">验证操作员身份，建立只读群消息采集会话。</p>
      <ol class="protocol-track" aria-label="登录协议步骤">
        <li><b>01</b><span>选择登录验证方式</span></li>
        <li><b>02</b><span>GT4 与验证码发送</span></li>
        <li><b>03</b><span>issued / verify 链路</span></li>
        <li><b>04</b><span>登录与二次挑战</span></li>
      </ol>
      </section>

    <form class="login-console" @submit.prevent="auth.submitLogin">
      <header class="console-heading">
        <div>
          <p class="eyebrow">AUTH SEQUENCE</p>
          <h2>操作员验证</h2>
        </div>
        <span class="secure-mark">LOCAL IPC</span>
      </header>

      <!-- 没有二次挑战时收集主登录方式、账号及首次验证值。 -->
      <template v-if="!auth.challengePending.value.length">
        <label>
          <span>登录方式</span>
          <select
            v-model.number="auth.loginMethod.value"
            data-test="login-method"
            :disabled="!!auth.busy.value"
          >
            <option :value="1">手机验证码</option>
            <option :value="2">邮箱验证码</option>
            <option :value="3">手机密码</option>
            <option :value="4">邮箱密码</option>
          </select>
        </label>

        <div class="field-grid" :class="{ 'phone-grid': auth.loginMethod.value === 1 || auth.loginMethod.value === 3 }">
          <label v-if="auth.loginMethod.value === 1 || auth.loginMethod.value === 3">
            <span>国家区号</span>
            <input v-model.number="auth.countryCode.value" type="number" inputmode="numeric" />
          </label>
          <label>
            <span>{{ auth.loginMethod.value === 1 || auth.loginMethod.value === 3 ? '手机号' : '邮箱地址' }}</span>
            <input
              v-model.trim="auth.account.value"
              :type="auth.loginMethod.value === 1 || auth.loginMethod.value === 3 ? 'tel' : 'email'"
              :autocomplete="auth.loginMethod.value === 1 || auth.loginMethod.value === 3 ? 'tel' : 'email'"
              :placeholder="auth.loginMethod.value === 1 || auth.loginMethod.value === 3 ? '输入手机号' : '输入邮箱地址'"
              required
            />
          </label>
        </div>

        <!-- 验证码登录先完成 GT4；成功后的验证码发送由认证流程继续驱动。 -->
        <section v-if="auth.isCodeMode.value" class="protocol-step auth-step" aria-labelledby="captcha-step">
          <h3 id="captcha-step"><span>01</span> GT4 与验证码</h3>
          <p
            class="field-note"
            data-test="gt4-status"
            role="status"
            aria-live="polite"
          >
            GT4 {{ auth.gt4Error.value ? 'ERROR' : auth.gt4Ready.value ? 'READY' : auth.gt4Loading.value ? 'LOADING' : 'IDLE' }}
            · 验证成功后自动发送验证码
          </p>
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

        <!-- 主验证同时承载验证码模式和密码模式。 -->
        <section class="protocol-step auth-step" aria-labelledby="primary-verify-step">
          <h3 id="primary-verify-step"><span>02</span> 主验证</h3>
          <label>
            <span>{{ auth.isCodeMode.value ? '验证码' : '登录密码' }}</span>
            <input
              v-model.trim="auth.validateValue.value"
              :type="auth.isCodeMode.value ? 'text' : 'password'"
              :inputmode="auth.isCodeMode.value ? 'numeric' : 'text'"
              :autocomplete="auth.isCodeMode.value ? 'one-time-code' : 'current-password'"
              required
            />
          </label>
          <p v-if="!auth.isCodeMode.value" class="warning-note">
            请输入原始登录密码，客户端会按服务端规则自动加密后发送。
          </p>
        </section>
      </template>

      <!-- 服务端返回 pending 项后切换到挑战流程：选择验证类型，按需发送 GT4 验证码，再提交验证值重试登录。 -->
      <section
        v-else
        class="protocol-step auth-step challenge-step"
        aria-labelledby="challenge-heading"
      >
        <h3 id="challenge-heading"><span>CHALLENGE</span> 服务端二次验证</h3>
        <label>
          <span>validateToken</span>
          <input
            data-test="validate-token"
            type="password"
            :value="auth.validateToken.value"
            readonly
            autocomplete="off"
          />
        </label>
        <div class="pending-list" aria-label="服务端待验证项">
          <label
            v-for="item in auth.challengePending.value"
            :key="`${item.validateType}-${item.account ?? ''}`"
            class="pending-option"
          >
            <input
              v-model.number="auth.selectedChallengeType.value"
              type="radio"
              name="pending-validation"
              :value="item.validateType"
            />
            <span>
              <b>ValidateType {{ item.validateType }}</b>
              {{ validateTypeLabels[item.validateType] ?? '通用验证值' }}
              · {{ item.account ?? '服务端未提供账号' }}
              <small>countryCode={{ item.countryCode ?? '—' }} / accountType={{ item.accountType ?? '—' }}</small>
            </span>
          </label>
        </div>
        <div v-if="auth.isChallengeCode.value">
          <p class="field-note" role="status" aria-live="polite">
            GT4 {{ auth.gt4Error.value ? 'ERROR' : auth.gt4Ready.value ? 'READY' : auth.gt4Loading.value ? 'LOADING' : 'IDLE' }}
            · 通过滑块后发送二次验证验证码
          </p>
          <button
            class="button secondary"
            data-test="challenge-send-code"
            type="button"
            :disabled="!!auth.busy.value || auth.gt4Loading.value"
            @click="auth.sendChallengeCode"
          >
            {{
              auth.busy.value === 'challenge-captcha'
                ? '等待验证…'
                : auth.busy.value === 'challenge-code'
                  ? '发送中…'
                  : `发送${auth.selectedChallenge.value?.validateType === 16 ? '邮箱' : '手机'}验证码`
            }}
          </button>
          <p v-if="auth.gt4Error.value" class="warning-note">{{ auth.gt4Error.value }}</p>
        </div>
        <label>
          <span>
            {{ isPasswordValidation(auth.selectedChallenge.value?.validateType)
              ? validateTypeLabels[auth.selectedChallenge.value!.validateType]
              : 'validateValue' }}
          </span>
          <input
            v-model.trim="auth.challengeValue.value"
            data-test="challenge-value"
            :type="isPasswordValidation(auth.selectedChallenge.value?.validateType) ? 'password' : 'text'"
            :autocomplete="isPasswordValidation(auth.selectedChallenge.value?.validateType) ? 'current-password' : 'one-time-code'"
            required
          />
        </label>
        <p
          v-if="isPasswordValidation(auth.selectedChallenge.value?.validateType)"
          class="warning-note"
        >
          请输入原始密码，客户端会按服务端规则自动加密后发送。
        </p>
        <p
          v-if="auth.selectedChallenge.value && auth.selectedChallenge.value.validateType >= 23"
          class="warning-note"
        >
          此类型的独立 loginType 暂不支持；完成服务端 pending verify 后仅重试原登录请求，不猜测映射。
        </p>
        <button
          class="button primary login-submit"
          data-test="challenge-submit"
          type="button"
          :disabled="!!auth.busy.value || !auth.selectedChallenge.value || !auth.challengeValue.value.trim()"
          @click="auth.submitChallenge"
        >
          <span>{{ auth.busy.value === 'challenge' ? '验证并重试中…' : '完成二次验证' }}</span>
          <span aria-hidden="true">→</span>
        </button>
      </section>

      <!-- 业务处理中信息与登录成败反馈分开呈现，避免把服务端通知误当作错误。 -->
      <section
        v-if="auth.businessProcessing.value.length"
        class="business-processing"
        role="status"
        aria-live="polite"
        aria-label="服务端业务通知"
      >
        <strong>BUSINESS PROCESSING</strong>
        <p
          v-for="item in auth.businessProcessing.value"
          :key="`${item.businessCode}-${item.businessMsg ?? ''}`"
        >
          <b>{{ item.businessCode }}</b>
          {{ item.businessMsg || '服务端未提供消息' }}
        </p>
      </section>

      <p v-if="auth.error.value" class="feedback error" role="alert">{{ auth.error.value }}</p>
      <p v-if="auth.notice.value" class="feedback notice" role="status">{{ auth.notice.value }}</p>

      <!-- 仅主登录分支显示最终提交按钮；挑战分支使用上方独立动作。 -->
      <button
        v-if="!auth.challengePending.value.length"
        class="button primary login-submit"
        type="submit"
        :disabled="!!auth.busy.value || !auth.accountReady.value || !auth.validateValue.value.trim()"
      >
        <span>{{ auth.busy.value === 'login' ? '建立会话中…' : '进入监控控制台' }}</span>
        <span aria-hidden="true">→</span>
      </button>
    </form>
    </div>
  </main>
</template>
