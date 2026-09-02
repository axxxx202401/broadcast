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

/** 认证流程实际使用的后端能力子集。 */
type AuthApi = Pick<
  typeof api,
  | 'sendSmsCode'
  | 'sendEmailCode'
  | 'issueValidationToken'
  | 'verifyValidations'
  | 'listPendingValidations'
  | 'login'
>

/** GT4 组合式函数暴露的控制器类型。 */
type Gt4Controller = ReturnType<typeof useGt4>

/** 认证组合式函数的可注入依赖，便于替换 IPC 与 GT4 边界。 */
interface AuthDependencies {
  api?: AuthApi
  gt4?: Gt4Controller
}

/**
 * 主登录方式契约：1 手机验证码、2 邮箱验证码、3 手机密码、4 邮箱密码。
 * 数值仅按当前前后端协议映射记录，不扩展推测其业务含义。
 */
const methodContract: Record<
  PrimaryLoginType,
  { account: 'phone' | 'email'; validateType: ValidateType; code: boolean }
> = {
  1: { account: 'phone', validateType: 17, code: true },
  2: { account: 'email', validateType: 16, code: true },
  3: { account: 'phone', validateType: 20, code: false },
  4: { account: 'email', validateType: 21, code: false },
}

/**
 * 管理主认证、服务端追加验证和验证码发送状态。
 *
 * 主链路固定为 issued → verify → login；密码值按当前实现原样提交给后端，不在前端
 * 进行 hash。远程调用失败可能已在服务端产生部分副作用，本地只能呈现错误，不能据此
 * 判定远端操作一定未执行。可注入 API 与 GT4 控制器以隔离传输层和验证码 SDK。
 *
 * @param onLogin 登录成功后接收群组和用户标识的回调。
 * @param dependencies 可选的后端 API 与 GT4 控制器。
 * @returns 认证表单状态、派生状态、GT4 状态及四个提交操作。
 */
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

  // 登录方式统一驱动账号种类、验证类型和是否需要发送验证码，避免分支各自维护协议映射。
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

  /** 串行化当前实例内的用户操作，并统一折叠本地可观察错误；不提供跨调用事务保证。 */
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

  /** 为主验证码登录触发 GT4，并使用展示时冻结的账号与国家区号发送验证码。 */
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
    // GT4 成功可能晚于表单编辑；发送目标必须使用弹出验证码时的账号快照。
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

  // 同一验证类型、国家区号、账号及账号类型组成稳定键；后出现的同键项目覆盖前项。
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

  /** 处理登录成功或 challenge 结果；补查失败不丢弃登录响应已经携带的验证项。 */
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

  /**
   * 执行登录，并仅对业务错误码 3114169 尝试补查待验证项。
   * 该数字只作为现有协议分支条件；若补查为空或失败，重新抛出原始登录错误。
   */
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

  /** 执行主 issued → verify → login 链路；verify 若返回剩余项则暂停在 challenge 阶段。 */
  const submitLogin = () =>
    run('login', async () => {
      if (!accountReady.value) throw new Error('请填写有效登录账号')
      if (!validateValue.value.trim()) {
        throw new Error(isCodeMode.value ? '请输入验证码' : '请输入登录密码')
      }
      // 当前协议直接传递 validateValue；密码模式也不在前端 hash。
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

  /** 提交当前选中的二次验证，并在全部验证完成后按 ValidateType 映射重试登录。 */
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
      // ValidateType 16–22 按既有协议映射登录类型；23 及以上保留原登录类型，不猜测含义。
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

  /** 仅为 ValidateType 16/17 的二次验证发送邮件或短信验证码，并冻结目标账号。 */
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
    // 优先使用完整的主账号；掩码账号不能作为验证码发送目标。
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
    // 切换到密码模式不再需要验证码实例，立即释放 SDK 资源。
    if (!methodContract[method].code) gt4.destroy()
  })

  return {
    /** 当前主登录方式。 */
    loginMethod,
    /** 当前登录账号。 */
    account,
    /** 手机账号使用的国家区号。 */
    countryCode,
    /** 主验证值；验证码或按实现原样传递的密码。 */
    validateValue,
    /** 当前验证链路的服务端令牌。 */
    validateToken,
    secondMac,
    /** 服务端尚未完成的验证项。 */
    challengePending,
    selectedChallengeType,
    selectedChallenge,
    challengeValue,
    businessProcessing,
    /** 当前正在执行的用户动作标识。 */
    busy,
    error,
    notice,
    isCodeMode,
    accountReady,
    isChallengeCode,
    gt4Loading: gt4.loading,
    gt4Ready: gt4.ready,
    gt4Error: gt4.error,
    /** 主动释放 GT4 资源。 */
    destroyGt4: gt4.destroy,
    sendCode,
    sendChallengeCode,
    submitLogin,
    submitChallenge,
  }
}
