import { computed, ref, watch } from 'vue'

import { api } from '../services/tauri'
import type {
  BusinessProcessing,
  GroupDto,
  LoginRequest,
  LoginResult,
  PendingValidation,
  PrimaryLoginType,
  ValidateType,
} from '../types/im'
import { errorMessage } from '../utils/protocol'
import { useGt4 } from './useGt4'

type AuthApi = Pick<
  typeof api,
  | 'sendSmsCode'
  | 'sendEmailCode'
  | 'issueValidationToken'
  | 'verifyValidations'
  | 'listPendingValidations'
  | 'login'
>

type Gt4Controller = ReturnType<typeof useGt4>

interface AuthDependencies {
  api?: AuthApi
  gt4?: Gt4Controller
}

const methodContract: Record<
  PrimaryLoginType,
  { account: 'phone' | 'email'; validateType: ValidateType; code: boolean }
> = {
  1: { account: 'phone', validateType: 17, code: true },
  2: { account: 'email', validateType: 16, code: true },
  3: { account: 'phone', validateType: 20, code: false },
  4: { account: 'email', validateType: 21, code: false },
}

export function useAuth(
  onLogin: (groups: GroupDto[], uid: string) => void,
  dependencies: AuthDependencies = {},
) {
  const backend = dependencies.api ?? api
  const gt4 = dependencies.gt4 ?? useGt4()
  const loginMethod = ref<PrimaryLoginType>(1)
  const account = ref('')
  const countryCode = ref(86)
  const validateValue = ref('')
  const validateToken = ref('')
  const secondMac = ref('')
  const challengePending = ref<PendingValidation[]>([])
  const selectedChallengeType = ref<ValidateType | null>(null)
  const challengeValue = ref('')
  const businessProcessing = ref<BusinessProcessing[]>([])
  const busy = ref<string | null>(null)
  const error = ref('')
  const notice = ref('')
  let lastLoginRequest: LoginRequest | null = null

  const contract = computed(() => methodContract[loginMethod.value])
  const isCodeMode = computed(() => contract.value.code)
  const accountReady = computed(() =>
    account.value.trim().length > 0
    && (contract.value.account === 'email' || Number.isFinite(countryCode.value)),
  )
  const selectedChallenge = computed(() =>
    challengePending.value.find(
      (item) => item.validateType === selectedChallengeType.value,
    ) ?? null,
  )
  const isChallengeCode = computed(() =>
    selectedChallenge.value?.validateType === 16
    || selectedChallenge.value?.validateType === 17,
  )

  async function run(step: string, operation: () => Promise<void>) {
    if (busy.value) return
    busy.value = step
    error.value = ''
    notice.value = ''
    try {
      await operation()
    } catch (reason) {
      error.value = errorMessage(reason)
    } finally {
      busy.value = null
    }
  }

  const sendCode = async () => {
    if (busy.value) return
    error.value = ''
    notice.value = ''
    if (!isCodeMode.value) {
      error.value = '密码登录无需发送验证码'
      return
    }
    if (!accountReady.value) {
      error.value = contract.value.account === 'phone'
        ? '请填写手机号和有效国家区号'
        : '请填写邮箱地址'
      return
    }
    if (!gt4.ready.value) {
      busy.value = 'gt4'
      const initialized = await gt4.initialize()
      busy.value = null
      if (!initialized) {
        error.value = gt4.error.value || 'GT4 初始化失败'
        return
      }
    }
    busy.value = 'captcha'
    const snapshot = account.value.trim()
    const snapshotCountryCode = countryCode.value
    const snapshotAccountKind = contract.value.account
    let consumed = false
    const shown = gt4.show(snapshot, async (verifiedAccount, fields) => {
      if (consumed) return
      consumed = true
      busy.value = 'code'
      try {
        if (snapshotAccountKind === 'phone') {
          await backend.sendSmsCode({
            phone: verifiedAccount,
            countryCode: snapshotCountryCode,
            codeType: 1,
            gt4DTO: fields,
          })
        } else {
          await backend.sendEmailCode({
            email: verifiedAccount,
            codeType: 1,
            gt4DTO: fields,
          })
        }
        notice.value = '验证码已发送'
        gt4.destroy()
      } catch (reason) {
        error.value = errorMessage(reason)
        gt4.reset()
      } finally {
        busy.value = null
      }
    })
    if (!shown) {
      busy.value = null
      error.value = gt4.error.value || 'GT4 尚未就绪'
    }
  }

  const accountFields = () => contract.value.account === 'phone'
    ? { phone: account.value.trim(), countryCode: countryCode.value }
    : { email: account.value.trim(), countryCode: 0 }
  const pendingAccountFields = () => ({
    account: account.value.trim(),
    countryCode: contract.value.account === 'phone' ? countryCode.value : 0,
  })

  const pendingKey = (item: PendingValidation) => [
    item.validateType,
    item.countryCode ?? '',
    item.account ?? '',
    item.accountType ?? '',
  ].join('|')

  const mergePending = (...groups: PendingValidation[][]) => {
    const merged = new Map<string, PendingValidation>()
    for (const item of groups.flat()) merged.set(pendingKey(item), item)
    return [...merged.values()]
  }

  const setPending = (pending: PendingValidation[]) => {
    challengePending.value = mergePending(pending)
    if (!challengePending.value.some(
      (item) => item.validateType === selectedChallengeType.value,
    )) {
      selectedChallengeType.value = challengePending.value[0]?.validateType ?? null
    }
  }

  const applyVerifyResponse = (
    response: Awaited<ReturnType<AuthApi['verifyValidations']>>,
  ) => {
    businessProcessing.value = response.businessProcessing
    if (!response.validateModelVOS.length) return false
    setPending(response.validateModelVOS)
    notice.value = '服务端要求继续完成验证'
    return true
  }

  async function handleLoginResult(result: LoginResult) {
    if (result.status === 'success') {
      challengePending.value = []
      selectedChallengeType.value = null
      onLogin(result.groups, result.uid)
      return
    }
    validateToken.value = result.validateToken
    notice.value = result.message
    setPending(result.pending ?? [])
    try {
      const listed = await backend.listPendingValidations({
        validateToken: result.validateToken,
      })
      setPending(mergePending(challengePending.value, listed))
    } catch (reason) {
      error.value = errorMessage(reason)
    }
  }

  async function loginWithMissingValidationRecovery(request: LoginRequest) {
    try {
      return await backend.login(request)
    } catch (reason) {
      const commandError = reason && typeof reason === 'object'
        ? reason as Record<string, unknown>
        : null
      if (
        commandError?.kind !== 'business'
        || commandError.code !== 3114169
        || !request.validateToken?.trim()
      ) {
        throw reason
      }

      try {
        const pending = await backend.listPendingValidations({
          validateToken: request.validateToken,
        })
        if (!pending.length) throw reason
        setPending(pending)
        challengeValue.value = ''
        notice.value = String(commandError.msg || '该场景下验证项缺失，请继续完成验证')
        return null
      } catch {
        throw reason
      }
    }
  }

  const submitLogin = () =>
    run('login', async () => {
      if (!accountReady.value) throw new Error('请填写有效登录账号')
      if (!validateValue.value.trim()) {
        throw new Error(isCodeMode.value ? '请输入验证码' : '请输入登录密码')
      }
      gt4.destroy()
      const issued = await backend.issueValidationToken({
        validateScene: 5,
        validateTypes: [contract.value.validateType],
      })
      if (!issued.validateToken.trim()) throw new Error('issued 未返回 validateToken')
      validateToken.value = issued.validateToken
      lastLoginRequest = {
        loginType: loginMethod.value,
        ...accountFields(),
        validateToken: issued.validateToken,
        ...(secondMac.value.trim() ? { secondMac: secondMac.value.trim() } : {}),
      }
      const verifyResponse = await backend.verifyValidations({
        validateToken: issued.validateToken,
        pendingValidateDTOS: [{
          ...pendingAccountFields(),
          validateType: contract.value.validateType,
          validateValue: validateValue.value.trim(),
        }],
        ...(secondMac.value.trim() ? { secondMac: secondMac.value.trim() } : {}),
      })
      if (applyVerifyResponse(verifyResponse)) return
      const result = await loginWithMissingValidationRecovery(lastLoginRequest)
      if (result) await handleLoginResult(result)
    })

  const submitChallenge = () =>
    run('challenge', async () => {
      const pending = selectedChallenge.value
      if (!pending || !validateToken.value.trim()) {
        throw new Error('请选择服务端要求的二次验证方式')
      }
      if (!challengeValue.value.trim()) throw new Error('请输入二次验证值')
      if (!lastLoginRequest) throw new Error('缺少原始登录请求，无法重试')
      const verifyResponse = await backend.verifyValidations({
        validateToken: validateToken.value.trim(),
        pendingValidateDTOS: [{
          ...pending,
          ...(
            pending.validateType === 16 || pending.validateType === 21
              ? { countryCode: pending.countryCode ?? 0 }
              : {}
          ),
          validateValue: challengeValue.value.trim(),
        }],
        ...(secondMac.value.trim() ? { secondMac: secondMac.value.trim() } : {}),
      })
      if (applyVerifyResponse(verifyResponse)) {
        challengeValue.value = ''
        return
      }
      const mappedLoginType = pending.validateType === 16
        ? 2
        : pending.validateType === 17
          ? 1
          : pending.validateType === 18
            ? 8
            : pending.validateType === 19
              ? 9
              : pending.validateType === 20
                ? 3
                : pending.validateType === 21
                  ? 4
                  : pending.validateType === 22
                    ? 7
                    : lastLoginRequest.loginType
      const retry: LoginRequest = {
        ...lastLoginRequest,
        loginType: mappedLoginType,
        validateToken: validateToken.value.trim(),
        ...(pending.validateType === 22
          ? { credentials: challengeValue.value.trim() }
          : {}),
      }
      lastLoginRequest = retry
      const result = await loginWithMissingValidationRecovery(retry)
      if (result) await handleLoginResult(result)
    })

  const sendChallengeCode = async () => {
    if (busy.value) return
    error.value = ''
    notice.value = ''
    const pending = selectedChallenge.value
    if (!pending || (pending.validateType !== 16 && pending.validateType !== 17)) {
      error.value = '当前二次验证方式不需要发送验证码'
      return
    }

    const wantsPhone = pending.validateType === 17
    const primaryMatches = contract.value.account === (wantsPhone ? 'phone' : 'email')
    const targetAccount = (primaryMatches ? account.value : pending.account ?? '').trim()
    if (!targetAccount || targetAccount.includes('*')) {
      error.value = wantsPhone
        ? '缺少可用的完整手机号，无法发送二次验证验证码'
        : '缺少可用的完整邮箱，无法发送二次验证验证码'
      return
    }
    const targetCountryCode = primaryMatches ? countryCode.value : pending.countryCode
    if (wantsPhone && !Number.isFinite(targetCountryCode)) {
      error.value = '缺少有效国家区号，无法发送二次验证验证码'
      return
    }

    if (!gt4.ready.value) {
      busy.value = 'challenge-gt4'
      const initialized = await gt4.initialize()
      busy.value = null
      if (!initialized) {
        error.value = gt4.error.value || 'GT4 初始化失败'
        return
      }
    }

    busy.value = 'challenge-captcha'
    let consumed = false
    const shown = gt4.show(targetAccount, async (verifiedAccount, fields) => {
      if (consumed) return
      consumed = true
      busy.value = 'challenge-code'
      try {
        if (wantsPhone) {
          await backend.sendSmsCode({
            phone: verifiedAccount,
            countryCode: targetCountryCode!,
            codeType: 1,
            gt4DTO: fields,
          })
        } else {
          await backend.sendEmailCode({
            email: verifiedAccount,
            codeType: 1,
            gt4DTO: fields,
          })
        }
        notice.value = '二次验证验证码已发送'
        gt4.destroy()
      } catch (reason) {
        error.value = errorMessage(reason)
        gt4.reset()
      } finally {
        busy.value = null
      }
    })
    if (!shown) {
      busy.value = null
      error.value = gt4.error.value || 'GT4 尚未就绪'
    }
  }

  watch(gt4.error, (gt4Error) => {
    if (gt4Error && (busy.value === 'captcha' || busy.value === 'challenge-captcha')) {
      busy.value = null
      error.value = gt4Error
    }
  })

  watch(loginMethod, (method) => {
    if (!methodContract[method].code) gt4.destroy()
  })

  return {
    loginMethod,
    account,
    countryCode,
    validateValue,
    validateToken,
    secondMac,
    challengePending,
    selectedChallengeType,
    selectedChallenge,
    challengeValue,
    businessProcessing,
    busy,
    error,
    notice,
    isCodeMode,
    accountReady,
    isChallengeCode,
    gt4Loading: gt4.loading,
    gt4Ready: gt4.ready,
    gt4Error: gt4.error,
    destroyGt4: gt4.destroy,
    sendCode,
    sendChallengeCode,
    submitLogin,
    submitChallenge,
  }
}
