import { computed, onUnmounted, ref, watch } from 'vue'

import { api } from '../services/tauri'
import type {
  AccountSummary,
  BusinessProcessing,
  GroupDto,
  LoginRequest,
  LoginResult,
  PendingValidation,
  PendingValidationDto,
  PrimaryLoginType,
  ValidateType,
} from '../types/im'
import { errorMessage, userFacingError } from '../utils/protocol'
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

/** 判断拒绝值是否表示登录密码已被复用，兼容 Display 文案与 AuthCommandError。 */
function isPasswordAlreadyReused(reason: unknown): boolean {
  const text = errorMessage(reason)
  return text.includes('登录密码已被使用，禁止重复消费') || text.includes('PasswordAlreadyReused')
}

/**
 * 按脱敏串中第一个与最后一个 `*` 提取可见前后缀，用于补全目标的本地一致性校验。
 * 不含 `*` 时要求全文相等。
 */
function matchesMaskedTarget(full: string, masked: string): boolean {
  const firstStar = masked.indexOf('*')
  const lastStar = masked.lastIndexOf('*')
  if (firstStar < 0) return full === masked
  const prefix = masked.slice(0, firstStar)
  const suffix = masked.slice(lastStar + 1)
  return full.startsWith(prefix)
    && full.endsWith(suffix)
    && full.length >= prefix.length + suffix.length
}

/**
 * 登录成功后交给应用层的会话发布材料。
 * 账号摘要缺失时由组合式函数按 uid 合成空展示字段，避免调用方因缺字段崩溃。
 */
export interface LoginSuccessPayload {
  /** 当前账号的非密钥摘要，供头部展示。 */
  account: AccountSummary
  /** 本次远程快照同步后得到的本地群组列表。 */
  groups: GroupDto[]
  /** 非阻塞提示，例如本次无法安全保存登录信息。 */
  warnings: string[]
}

/**
 * 管理主认证、服务端追加验证和验证码发送状态。
 *
 * 主链路固定为 issued → verify → login；密码值按当前实现原样提交给后端，不在前端
 * 进行 hash。远程调用失败可能已在服务端产生部分副作用，本地只能呈现错误，不能据此
 * 确认远端操作是否执行。可注入 API 与 GT4 控制器以隔离传输层和验证码 SDK。
 *
 * @param onLogin 登录成功后接收账号摘要、群组和 warnings 的回调。
 * @param dependencies 可选的后端 API 与 GT4 控制器。
 * @returns 认证表单状态、二次验证步数与倒计时、GT4 状态及登录/挑战动作。
 */
export function useAuth(
  onLogin: (payload: LoginSuccessPayload) => void,
  dependencies: AuthDependencies = {},
) {
  const backend = dependencies.api ?? api
  const gt4 = dependencies.gt4 ?? useGt4()
  /** 主登录方式；Task 6 规定默认邮箱密码。 */
  const loginMethod = ref<PrimaryLoginType>(4)
  const account = ref('')
  const selectedAccountUid = ref<string | null>(null)
  const countryCode = ref(86)
  const validateValue = ref('')
  /**
   * 密码模式：驱动“已保存密码”哨兵展示与提交参数构造。
   *
   * - empty：当前账号无已保存密码，提交时必须带 validateValue
   * - saved：当前账号有已保存密码，提交时仅带 savedPasswordUid，绝不构造 validateValue
   * - manual：用户编辑了密码输入框，提交时带 validateValue
   */
  const passwordMode = ref<'empty' | 'saved' | 'manual'>('empty')
  /**
   * 已选保存账号的身份快照；仅当当前表单仍与该快照一致时才允许 saved 模式。
   * 不包含密码明文。
   */
  let selectedSavedIdentity: {
    uid: string
    loginType: PrimaryLoginType
    displayAccount: string
    hasSavedPassword: boolean
  } | null = null
  /** 回填保存账号时跳过身份对账，避免中间赋值误清哨兵。 */
  let applyingSavedAccount = false
  /** “其他登录方式”折叠面板是否展开。 */
  const otherMethodsOpen = ref(false)
  const validateToken = ref('')
  const challengePending = ref<PendingValidation[]>([])
  const selectedChallengeType = ref<ValidateType | null>(null)
  const challengeValue = ref('')
  /** 当前连续二次验证步序号；未进入挑战时为 0。 */
  const challengeStep = ref(0)
  /** 验证码重发倒计时剩余秒数；为 0 时允许再次发送。 */
  const resendSeconds = ref(0)
  /** 本轮已完成的待验证项稳定键，仅用于步数推进，不回传服务端。 */
  const completedChallengeKeys = ref<string[]>([])
  /** 用户为脱敏邮箱/手机补全的完整目标；只用于本次发送，不写入账号索引。 */
  const supplementedTarget = ref('')
  /** 登录密码自动复用已失败，界面应展示密码输入。 */
  const passwordReuseFailed = ref(false)
  /** 本轮是否已发起过一次 reuseLoginPassword；发起后 20/21 必须保持可输入。 */
  const passwordReuseAttempted = ref(false)
  const businessProcessing = ref<BusinessProcessing[]>([])
  const busy = ref<string | null>(null)
  const error = ref('')
  const notice = ref('')
  let lastLoginRequest: LoginRequest | null = null
  /**
   * 二次验证世代：resetChallenge 时递增。
   * 自动复用响应必须与发起时的世代一致，否则丢弃。
   */
  let challengeGeneration = 0
  /** 重发倒计时的唯一 interval；卸载或 resetChallenge 时必须清除。 */
  let resendTimer: ReturnType<typeof setInterval> | null = null

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

  /**
   * 主登录账号是否已是当前验证码类型所需的完整、且与脱敏前后缀一致的目标。
   * 满足时不必再展示补全面板。
   */
  function hasCompletePrimaryChallengeTarget(pending: PendingValidation): boolean {
    const wantsPhone = pending.validateType === 17
    const wantsEmail = pending.validateType === 16
    if (!wantsPhone && !wantsEmail) return false
    const primaryMatches = contract.value.account === (wantsPhone ? 'phone' : 'email')
    const primary = account.value.trim()
    if (!primaryMatches || !primary || primary.includes('*')) return false
    const masked = (pending.account ?? '').trim()
    if (masked.includes('*') && !matchesMaskedTarget(primary, masked)) return false
    return true
  }

  /** 仅当发送验证码缺少完整目标时，才要求用户补充脱敏邮箱或手机号。 */
  const needsSupplementedTarget = computed(() => {
    const pending = selectedChallenge.value
    if (!pending || !isChallengeCode.value) return false
    if (hasCompletePrimaryChallengeTarget(pending)) return false
    const pendingAccount = (pending.account ?? '').trim()
    return !pendingAccount || pendingAccount.includes('*')
  })

  /** 串行化当前实例内的用户操作，并统一折叠本地可观察错误；不提供跨调用事务保证。 */
  async function run(step: string, operation: () => Promise<void>) {
    if (busy.value) return
    busy.value = step
    error.value = ''
    notice.value = ''
    try {
      await operation()
    } catch (reason) {
      error.value = userFacingError(reason)
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
        error.value = ''
        notice.value = '验证码已发送'
        gt4.destroy()
      } catch (reason) {
        error.value = userFacingError(reason)
        gt4.reset()
      } finally {
        busy.value = null
      }
    }, () => {
      if (busy.value === 'captcha') busy.value = null
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

  /** 停止倒计时 interval，避免测试与页面卸载后残留定时器。 */
  const clearResendTimer = () => {
    if (resendTimer !== null) {
      clearInterval(resendTimer)
      resendTimer = null
    }
  }

  /** 发送成功后启动 60 秒倒计时；同一时刻只保留一个 interval。 */
  const startResendCountdown = () => {
    clearResendTimer()
    resendSeconds.value = 60
    resendTimer = setInterval(() => {
      if (resendSeconds.value <= 1) {
        resendSeconds.value = 0
        clearResendTimer()
        return
      }
      resendSeconds.value -= 1
    }, 1000)
  }

  /**
   * 进入或推进二次验证待办列表。
   * `advance` 表示已在挑战中完成一项后接到下一项，步数加一并记录已完成键。
   */
  const activateChallenge = (pending: PendingValidation[], advance: boolean) => {
    if (advance && selectedChallenge.value) {
      completedChallengeKeys.value = [
        ...completedChallengeKeys.value,
        pendingKey(selectedChallenge.value),
      ]
    }
    setPending(pending)
    if (!challengePending.value.length) return
    if (advance) challengeStep.value += 1
    else if (challengeStep.value < 1) challengeStep.value = 1
  }

  /**
   * 退出二次验证：清空令牌、待办、临时输入、补全目标、倒计时和自动复用标志。
   * 不删除已保存账号，也不改写主登录表单中的账号与密码模式。
   */
  const resetChallenge = () => {
    challengeGeneration += 1
    clearResendTimer()
    validateToken.value = ''
    challengePending.value = []
    selectedChallengeType.value = null
    challengeValue.value = ''
    challengeStep.value = 0
    resendSeconds.value = 0
    completedChallengeKeys.value = []
    supplementedTarget.value = ''
    passwordReuseFailed.value = false
    passwordReuseAttempted.value = false
    lastLoginRequest = null
    businessProcessing.value = []
    notice.value = ''
    error.value = ''
  }

  onUnmounted(() => {
    clearResendTimer()
  })

  /**
   * 当用户编辑密码输入框时切换到 manual 模式。
   * code 模式下 validateValue 表示验证码，不参与 passwordMode 状态机。
   */
  watch(validateValue, (value) => {
    if (isCodeMode.value) return
    if (value.trim().length) passwordMode.value = 'manual'
  }, { flush: 'sync' })

  /** 当前表单是否仍指向已选保存账号的登录方式与展示账号。 */
  function savedIdentityMatches() {
    const saved = selectedSavedIdentity
    return !!saved
      && selectedAccountUid.value === saved.uid
      && loginMethod.value === saved.loginType
      && account.value.trim() === saved.displayAccount.trim()
  }

  /**
   * 登录方式或账号与已选保存账号不一致时退出 saved，并丢掉 uid。
   * 用户已改写密码的 manual 模式保留，仅撤销 savedPasswordUid 资格。
   */
  function syncSavedPasswordEligibility() {
    if (applyingSavedAccount) return
    if (savedIdentityMatches()) return
    selectedAccountUid.value = null
    selectedSavedIdentity = null
    if (passwordMode.value === 'saved') passwordMode.value = 'empty'
  }

  watch([loginMethod, account], syncSavedPasswordEligibility, { flush: 'sync' })

  const applyVerifyResponse = (
    response: Awaited<ReturnType<AuthApi['verifyValidations']>>,
  ) => {
    businessProcessing.value = response.businessProcessing
    if (!response.validateModelVOS.length) return false
    const advance = challengePending.value.length > 0
    activateChallenge(response.validateModelVOS, advance)
    notice.value = '服务端要求继续完成验证'
    return true
  }

  /** ValidateType 16–22 按既有协议映射登录类型；23 及以上保留原登录类型。 */
  function mappedLoginTypeFor(pending: PendingValidation): LoginRequest['loginType'] {
    if (!lastLoginRequest) return 4
    return pending.validateType === 16
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
  }

  /** 二次验证通过后按映射类型重试登录。 */
  async function retryLoginAfterChallenge(pending: PendingValidation) {
    if (!lastLoginRequest) throw new Error('缺少原始登录请求，无法重试')
    const retry: LoginRequest = {
      ...lastLoginRequest,
      loginType: mappedLoginTypeFor(pending),
      validateToken: validateToken.value.trim(),
      ...(pending.validateType === 22
        ? { credentials: challengeValue.value.trim() }
        : {}),
    }
    lastLoginRequest = retry
    const result = await loginWithMissingValidationRecovery(retry)
    if (result) await handleLoginResult(result)
  }

  /**
   * 进入 ValidateType 20/21 时自动以 reuseLoginPassword 提交一次。
   * 已复用或 AuthCommandError 含 PasswordAlreadyReused 时只展示密码框，不自动重试。
   */
  async function maybeReuseLoginPassword() {
    const pending = selectedChallenge.value
    if (!pending || (pending.validateType !== 20 && pending.validateType !== 21)) return
    if (passwordReuseAttempted.value) return
    if (!validateToken.value.trim()) return
    passwordReuseAttempted.value = true
    const generation = challengeGeneration
    try {
      const verifyResponse = await backend.verifyValidations({
        validateToken: validateToken.value.trim(),
        pendingValidateDTOS: [{
          ...pending,
          reuseLoginPassword: true,
        }],
      })
      if (generation !== challengeGeneration) return
      if (applyVerifyResponse(verifyResponse)) {
        challengeValue.value = ''
        return
      }
      await retryLoginAfterChallenge(pending)
    } catch (reason) {
      if (generation !== challengeGeneration) return
      passwordReuseFailed.value = true
      if (isPasswordAlreadyReused(reason)) return
      error.value = userFacingError(reason)
    }
  }

  /** 登录成功但缺少账号摘要时，按 uid 合成空展示字段，避免调用方崩溃。 */
  function accountFromLogin(result: Extract<LoginResult, { status: 'success' }>): AccountSummary {
    return result.account ?? {
      uid: result.uid,
      displayAccount: '',
      loginType: loginMethod.value,
      hasSavedPassword: false,
      isCurrent: true,
    }
  }

  /**
   * 回填已保存账号的展示字段，供后续登录页使用。
   * 只保存 uid、展示账号和登录方式；不得写入密码明文。
   */
  function selectSavedAccount(saved: AccountSummary) {
    // 切换账号后必须清理上一会话的瞬态状态，避免挑战/错误信息残留。
    applyingSavedAccount = true
    try {
      resetAuthForm({ preserveSelectedAccount: true })
      selectedSavedIdentity = {
        uid: saved.uid,
        loginType: saved.loginType,
        displayAccount: saved.displayAccount,
        hasSavedPassword: saved.hasSavedPassword,
      }
      selectedAccountUid.value = saved.uid
      loginMethod.value = saved.loginType
      account.value = saved.displayAccount
      validateValue.value = ''
      passwordMode.value = saved.hasSavedPassword ? 'saved' : 'empty'
      otherMethodsOpen.value = false
    } finally {
      applyingSavedAccount = false
    }
  }

  /**
   * 清理当前会话的瞬态认证状态，用于退出/恢复到登录页时的“干净起点”。
   *
   * `preserveSelectedAccount` 用于 needsLogin 后立即 `selectSavedAccount` 的衔接流程；
   * 默认不覆盖当前选中的账号摘要，避免 UI 错配。
   */
  function resetAuthForm(options: { preserveSelectedAccount?: boolean } = {}) {
    const preserveSelectedAccount = options.preserveSelectedAccount ?? true

    resetChallenge()
    otherMethodsOpen.value = false
    validateValue.value = ''
    busy.value = null

    if (!preserveSelectedAccount) {
      selectedAccountUid.value = null
      selectedSavedIdentity = null
      account.value = ''
      loginMethod.value = 4
      countryCode.value = 86
      passwordMode.value = 'empty'
    }
  }

  /** 处理登录成功或 challenge 结果；补查失败不丢弃登录响应已经携带的验证项。 */
  async function handleLoginResult(result: LoginResult) {
    if (result.status === 'success') {
      challengePending.value = []
      selectedChallengeType.value = null
      challengeValue.value = ''
      challengeStep.value = 0
      completedChallengeKeys.value = []
      supplementedTarget.value = ''
      passwordReuseFailed.value = false
      passwordReuseAttempted.value = false
      clearResendTimer()
      resendSeconds.value = 0
      onLogin({
        account: accountFromLogin(result),
        groups: result.groups,
        warnings: result.warnings ?? [],
      })
      return
    }
    validateToken.value = result.validateToken
    notice.value = userFacingError(result.message)
    const fromLoginPending = result.pending ?? []
    const alreadyChallenging = challengePending.value.length > 0
    try {
      const listed = await backend.listPendingValidations({
        validateToken: result.validateToken,
      })
      activateChallenge(mergePending(fromLoginPending, listed), alreadyChallenging)
    } catch (reason) {
      activateChallenge(fromLoginPending, alreadyChallenging)
      error.value = userFacingError(reason)
    }
    await maybeReuseLoginPassword()
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
        activateChallenge(pending, challengePending.value.length > 0)
        challengeValue.value = ''
        notice.value = String(commandError.msg || '该场景下验证项缺失，请继续完成验证')
        await maybeReuseLoginPassword()
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
      if (isCodeMode.value) {
        if (!validateValue.value.trim()) throw new Error('请输入验证码')
      } else if (passwordMode.value === 'saved' && savedIdentityMatches() && selectedSavedIdentity?.hasSavedPassword) {
        if (!selectedAccountUid.value) throw new Error('缺少已保存账号信息')
      } else {
        if (!validateValue.value.trim()) throw new Error('请输入登录密码')
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
      }
      const pendingValidateDTOS: PendingValidationDto[] = isCodeMode.value
        ? [{
          ...pendingAccountFields(),
          validateType: contract.value.validateType,
          validateValue: validateValue.value.trim(),
        }]
        : passwordMode.value === 'saved'
          && savedIdentityMatches()
          && selectedSavedIdentity?.hasSavedPassword
          && selectedAccountUid.value
          ? [{
            ...pendingAccountFields(),
            validateType: contract.value.validateType,
            savedPasswordUid: selectedAccountUid.value,
          }]
          : [{
            ...pendingAccountFields(),
            validateType: contract.value.validateType,
            validateValue: validateValue.value.trim(),
          }]
      const verifyResponse = await backend.verifyValidations({
        validateToken: issued.validateToken,
        pendingValidateDTOS,
      })
      if (applyVerifyResponse(verifyResponse)) {
        await maybeReuseLoginPassword()
        return
      }
      const result = await loginWithMissingValidationRecovery(lastLoginRequest)
      if (result) await handleLoginResult(result)
    })

  /** 提交当前二次验证；verify 未返回剩余项时按 ValidateType 映射重试登录。 */
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
      })
      if (applyVerifyResponse(verifyResponse)) {
        challengeValue.value = ''
        await maybeReuseLoginPassword()
        return
      }
      await retryLoginAfterChallenge(pending)
    })

  /** 仅为 ValidateType 16/17 的二次验证发送邮件或短信验证码；GT4 只在用户点击发送时启动。 */
  const sendChallengeCode = async () => {
    if (busy.value) return
    if (resendSeconds.value > 0) return
    error.value = ''
    notice.value = ''
    const pending = selectedChallenge.value
    if (!pending || (pending.validateType !== 16 && pending.validateType !== 17)) {
      error.value = '当前二次验证方式不需要发送验证码'
      return
    }

    const wantsPhone = pending.validateType === 17
    const primaryMatches = contract.value.account === (wantsPhone ? 'phone' : 'email')
    const primaryAccount = account.value.trim()
    const maskedAccount = (pending.account ?? '').trim()
    const masked = maskedAccount.includes('*')
    let targetAccount = ''
    if (primaryMatches && primaryAccount && !primaryAccount.includes('*')) {
      // 主账号可用时也必须通过脱敏前后缀校验，不能跳过。
      if (!masked || matchesMaskedTarget(primaryAccount, maskedAccount)) {
        targetAccount = primaryAccount
      }
    } else if (supplementedTarget.value.trim()) {
      const supplemented = supplementedTarget.value.trim()
      if (masked && !matchesMaskedTarget(supplemented, maskedAccount)) {
        error.value = wantsPhone
          ? '补充的手机号与脱敏前后缀不一致'
          : '补充的邮箱与脱敏前后缀不一致'
        return
      }
      targetAccount = supplemented
    } else {
      targetAccount = maskedAccount
    }

    if (!targetAccount || targetAccount.includes('*')) {
      error.value = wantsPhone
        ? '请补充完整手机号后再发送验证码'
        : '请补充完整邮箱后再发送验证码'
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
    // 补全目标只传给本次发送；不得写回 account，以免污染账号索引。
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
        error.value = ''
        notice.value = '二次验证验证码已发送'
        startResendCountdown()
        gt4.destroy()
      } catch (reason) {
        error.value = userFacingError(reason)
        gt4.reset()
      } finally {
        busy.value = null
      }
    }, () => {
      if (busy.value === 'challenge-captcha') busy.value = null
    })
    if (!shown) {
      busy.value = null
      error.value = gt4.error.value || 'GT4 尚未就绪'
    }
  }

  watch(selectedChallengeType, (next, prev) => {
    // 切换验证方式时不得沿用上一方式的倒计时与临时输入。
    if (prev != null && next != null && prev !== next) {
      clearResendTimer()
      resendSeconds.value = 0
      challengeValue.value = ''
      supplementedTarget.value = ''
    }
    void maybeReuseLoginPassword()
  }, { flush: 'sync' })

  watch(gt4.error, (gt4Error) => {
    // 滑块失败不再写入 gt4.error；此处只承接 SDK 异常等仍需展示的错误。
    if (gt4Error && (busy.value === 'captcha' || busy.value === 'challenge-captcha')) {
      busy.value = null
      error.value = gt4Error
    }
  })

  watch(loginMethod, (method) => {
    // 切换到密码模式不再需要验证码实例，此处调用 destroy 释放 SDK 资源。
    if (!methodContract[method].code) gt4.destroy()
  })

  /** 折叠/展开“其他登录方式”面板。 */
  const toggleOtherMethods = () => {
    otherMethodsOpen.value = !otherMethodsOpen.value
  }

  return {
    /** 表单 ref 可由视图双向绑定；切换登录方式不会自动清空账号或验证值。 */
    loginMethod,
    account,
    /** 已选择的保存账号 UID；selectSavedAccount 写入，不得关联密码明文。 */
    selectedAccountUid,
    countryCode,
    validateValue,
    passwordMode,
    otherMethodsOpen,
    validateToken,
    /** challenge 状态由 verify/login 响应重建；成功登录后清空待验证项和选择。 */
    challengePending,
    selectedChallengeType,
    selectedChallenge,
    challengeValue,
    challengeStep,
    resendSeconds,
    completedChallengeKeys,
    supplementedTarget,
    passwordReuseFailed,
    passwordReuseAttempted,
    needsSupplementedTarget,
    businessProcessing,
    /** busy 非空时主要 actions 会拒绝重入；error/notice 由新动作开始时重置。 */
    busy,
    error,
    notice,
    isCodeMode,
    accountReady,
    isChallengeCode,
    gt4Loading: gt4.loading,
    gt4Ready: gt4.ready,
    gt4Error: gt4.error,
    /** GT4 状态保持响应式；destroyGt4 仅释放验证码资源，不清理认证表单。 */
    destroyGt4: gt4.destroy,
    /** 发送与提交 actions 各自检查账号、验证方式及 busy 等前置条件。 */
    sendCode,
    sendChallengeCode,
    submitLogin,
    submitChallenge,
    /** 回填已保存账号的展示字段；Task 6 会继续完善登录页交互。 */
    selectSavedAccount,
    /** 退出二次验证并回到主登录卡片，不删除已保存账号。 */
    resetChallenge,
    /** 退出/恢复到登录页时清理瞬态认证状态。 */
    resetAuthForm,
    /** 折叠/展开“其他登录方式”。 */
    toggleOtherMethods,
  }
}
