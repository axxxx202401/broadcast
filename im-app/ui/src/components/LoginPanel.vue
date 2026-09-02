<script setup lang="ts">
import { onUnmounted } from 'vue'

import type { useAuth } from '../composables/useAuth'
import type { ValidateType } from '../types/im'

const props = defineProps<{ auth: ReturnType<typeof useAuth> }>()

onUnmounted(props.auth.destroyGt4)

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
</script>

<template>
  <main class="login-shell">
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
          <label>
            <span>secondMac（可选）</span>
            <input v-model.trim="auth.secondMac.value" autocomplete="off" />
          </label>
        </section>
      </template>

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
  </main>
</template>
