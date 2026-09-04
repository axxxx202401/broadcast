import { ref } from 'vue'

import { api } from '../services/tauri'
import type {
  AccountSummary,
  RemoveAccountResult,
  RestoreSessionResult,
} from '../types/im'
import { toPrimaryLoginType } from '../types/im'

/** 启动恢复与账号切换的前端阶段。 */
export type AccountPhase = 'recovering' | 'ready' | 'needsLogin'

/** 最近一次恢复类操作；可重试失败时决定重试调用 restore 还是 switch。 */
export type AccountOp = 'restore' | 'switch'

/** 账号组合式函数实际使用的后端能力子集。 */
type AccountsApi = Pick<
  typeof api,
  'restoreSession' | 'listAccounts' | 'switchAccount' | 'removeAccount' | 'pauseSession'
>

/** 账号组合式函数的可注入依赖，便于替换 IPC 边界。 */
interface AccountsDependencies {
  api?: AccountsApi
}

/** 传输失败时返回给用户的普通文案，不得包含协议码或内部实现细节。 */
const NETWORK_RETRY_MESSAGE = '网络连接失败，请重试'
/** Rust 退出未确认时已经给出的用户文案，可原样展示。 */
const LOGOUT_UNCONFIRMED_MESSAGE = '本次无法确认已退出，请重试'

/**
 * 把恢复/切换拒绝值折叠为可展示的重试文案。
 * 仅放行已经面向用户的退出未确认消息；其余一律降级为网络重试提示。
 */
function toRetryableMessage(reason: unknown): string {
  if (reason === LOGOUT_UNCONFIRMED_MESSAGE) return LOGOUT_UNCONFIRMED_MESSAGE
  if (reason instanceof Error && reason.message === LOGOUT_UNCONFIRMED_MESSAGE) {
    return LOGOUT_UNCONFIRMED_MESSAGE
  }
  if (reason && typeof reason === 'object') {
    const commandError = reason as Record<string, unknown>
    if (commandError.message === LOGOUT_UNCONFIRMED_MESSAGE) {
      return LOGOUT_UNCONFIRMED_MESSAGE
    }
    if (commandError.kind === 'other' && commandError.message === LOGOUT_UNCONFIRMED_MESSAGE) {
      return LOGOUT_UNCONFIRMED_MESSAGE
    }
  }
  return NETWORK_RETRY_MESSAGE
}

/** 把 needsLogin 结果收成前端可用的账号摘要，供登录页回填。 */
function accountFromNeedsLogin(
  result: Extract<RestoreSessionResult, { status: 'needsLogin' }>,
): AccountSummary {
  return {
    uid: result.uid,
    displayAccount: result.displayAccount,
    loginType: toPrimaryLoginType(result.loginType),
    hasSavedPassword: result.hasSavedPassword,
    isCurrent: false,
  }
}

/** 把 IPC 返回的账号摘要收敛到前端约束，防止异常 loginType 破坏界面状态。 */
function normalizeAccountSummary(account: AccountSummary): AccountSummary {
  return {
    ...account,
    loginType: toPrimaryLoginType(account.loginType),
  }
}

/**
 * 管理已保存账号摘要、启动恢复和切换状态。
 *
 * 所有异步动作使用 operation token：后发起的恢复/切换会使先前结果失效。
 * 组合式函数不自动调用 `restore`，由根组件在挂载时触发，以免重复请求。
 * 拒绝值不会把 Rust/HTTP 内部细节交给界面。
 *
 * @param dependencies 可选的后端 API，测试可注入假实现。
 * @returns 恢复阶段、账号列表、当前选择以及恢复/切换/移除动作。
 */
export function useAccounts(dependencies: AccountsDependencies = {}) {
  const backend = dependencies.api ?? api
  const phase = ref<AccountPhase>('recovering')
  const accounts = ref<AccountSummary[]>([])
  const selectedAccount = ref<AccountSummary | null>(null)
  const retryableMessage = ref('')
  const busy = ref<string | null>(null)
  /** 最近一次恢复类操作；切换可重试失败时 UI 显示“正在切换账号”。 */
  const lastAccountOp = ref<AccountOp>('restore')
  /** 切换进入 retryable 时记住目标 UID，供 `retryRestore` 再次 switch。 */
  let pendingSwitchUid: string | null = null
  /**
   * 从主界面「添加账号」进入登录页时记住上一账号 UID。
   * 返回时用该 UID 走 Token 恢复，不得在暂停会话时删除 Token。
   */
  const returnToUid = ref<string | null>(null)
  let operationToken = 0

  /** 在当前代次仍有效时刷新非密钥账号列表；列表失败不得把恢复打成 retryable。 */
  async function refreshAccounts(token: number) {
    try {
      const listed = await backend.listAccounts()
      if (token !== operationToken) return
      accounts.value = listed.map(normalizeAccountSummary)
    } catch {
      if (token !== operationToken) return
    }
  }

  /** 按恢复联合结果更新阶段；陈旧调用不得进入此函数。 */
  function applyRestoreResult(result: RestoreSessionResult, token: number): RestoreSessionResult {
    switch (result.status) {
      case 'success':
        phase.value = 'ready'
        selectedAccount.value = normalizeAccountSummary(result.account)
        retryableMessage.value = ''
        pendingSwitchUid = null
        returnToUid.value = null
        void refreshAccounts(token)
        return result
      case 'needsLogin':
        phase.value = 'needsLogin'
        selectedAccount.value = accountFromNeedsLogin(result)
        retryableMessage.value = ''
        pendingSwitchUid = null
        void refreshAccounts(token)
        return result
      case 'noAccount':
        phase.value = 'needsLogin'
        selectedAccount.value = null
        retryableMessage.value = ''
        pendingSwitchUid = null
        void refreshAccounts(token)
        return result
      case 'retryable':
        phase.value = 'recovering'
        retryableMessage.value = result.message
        return result
    }
  }

  /** 执行一次带代次的恢复类请求；过期结果返回 `null`。 */
  async function runRestore(
    key: AccountOp,
    operation: () => Promise<RestoreSessionResult>,
    fallbackUid: string,
  ): Promise<RestoreSessionResult | null> {
    const token = ++operationToken
    busy.value = key
    lastAccountOp.value = key
    if (key === 'switch') {
      pendingSwitchUid = fallbackUid
    } else {
      pendingSwitchUid = null
    }
    retryableMessage.value = ''
    phase.value = 'recovering'
    try {
      const result = await operation()
      if (token !== operationToken) return null
      return applyRestoreResult(result, token)
    } catch (reason) {
      if (token !== operationToken) return null
      return applyRestoreResult({
        status: 'retryable',
        uid: fallbackUid,
        message: toRetryableMessage(reason),
      }, token)
    } finally {
      if (token === operationToken) busy.value = null
    }
  }

  /**
   * 请求后端恢复最后使用的账号。
   * 成功、需要登录或无账号都会离开启动态；可重试失败保持 `recovering` 并设置用户文案。
   */
  const restore = () =>
    runRestore('restore', () => backend.restoreSession(), selectedAccount.value?.uid ?? '')

  /**
   * 再次执行最近一次恢复类操作。
   * 若上次是切换且仍记住目标 UID，则再次 `switchAccount`；否则走启动恢复。
   */
  const retryRestore = () => {
    if (lastAccountOp.value === 'switch' && pendingSwitchUid) {
      return switchAccount(pendingSwitchUid)
    }
    return restore()
  }

  /**
   * 切换到指定 UID 的已保存账号。
   * 后发起的切换会使先前结果失效，避免旧账号会话覆盖新选择。
   */
  const switchAccount = (uid: string) =>
    runRestore('switch', () => backend.switchAccount(uid), uid)

  /**
   * 放弃当前恢复/切换，进入空白登录页。
   * 递增代次以忽略仍在飞行中的旧结果。
   */
  function useOtherAccount() {
    operationToken += 1
    busy.value = null
    phase.value = 'needsLogin'
    selectedAccount.value = null
    retryableMessage.value = ''
    pendingSwitchUid = null
    lastAccountOp.value = 'restore'
  }

  /**
   * 移除指定账号的索引与凭据。
   * 当前选中账号被移除时清空选择；若当时已在主界面，则回到需要登录。
   */
  async function removeAccount(uid: string): Promise<RemoveAccountResult | null> {
    const token = ++operationToken
    busy.value = 'remove'
    try {
      const result = await backend.removeAccount(uid)
      if (token !== operationToken) return null
      if (selectedAccount.value?.uid === uid) {
        selectedAccount.value = null
        if (phase.value === 'ready') phase.value = 'needsLogin'
      }
      await refreshAccounts(token)
      return result
    } catch (reason) {
      if (token !== operationToken) return null
      retryableMessage.value = toRetryableMessage(reason)
      return null
    } finally {
      if (token === operationToken) busy.value = null
    }
  }

  /**
   * 手动登录成功后立刻写入当前账号摘要，供头部后续展示。
   * 不读取或保存密码明文。
   */
  function applyManualLogin(account: AccountSummary) {
    phase.value = 'ready'
    selectedAccount.value = { ...normalizeAccountSummary(account), isCurrent: true }
    retryableMessage.value = ''
    pendingSwitchUid = null
    returnToUid.value = null
    void refreshAccounts(operationToken)
  }

  /**
   * 添加账号：暂停当前会话（只断 TCP，保留 Token），进入空白登录页。
   * 记住当前 UID，供登录页返回时用原 Token 恢复。
   */
  async function beginAddAccount(): Promise<void> {
    const previousUid = selectedAccount.value?.uid ?? null
    const token = ++operationToken
    busy.value = 'add'
    returnToUid.value = previousUid
    try {
      await backend.pauseSession()
      if (token !== operationToken) return
      phase.value = 'needsLogin'
      selectedAccount.value = null
      retryableMessage.value = ''
      pendingSwitchUid = null
    } catch {
      if (token !== operationToken) return
      phase.value = 'needsLogin'
      selectedAccount.value = null
      retryableMessage.value = ''
    } finally {
      if (token === operationToken) busy.value = null
    }
  }

  /**
   * 从添加账号登录页回到上一账号：用记住的 UID 走切换恢复。
   * 没有可返回 UID 时不发请求。
   */
  function returnFromAddAccount(): Promise<RestoreSessionResult | null> {
    const uid = returnToUid.value
    if (!uid) return Promise.resolve(null)
    returnToUid.value = null
    return switchAccount(uid)
  }

  return {
    /** 启动时为 recovering，避免在 IPC 完成前闪现登录页或主界面。 */
    phase,
    /** 非密钥账号摘要列表；列表刷新失败不会把恢复打成 retryable。 */
    accounts,
    /** 当前恢复成功或待登录的账号；无账号或改用其他账号时为 null。 */
    selectedAccount,
    /** 可重试失败时的用户文案；成功或需要登录后清空。 */
    retryableMessage,
    /** 非空时表示恢复、切换或移除仍在进行。 */
    busy,
    /** 最近一次恢复类操作，供可重试界面区分文案与重试目标。 */
    lastAccountOp,
    /** 添加账号登录页可返回的上一账号 UID；无返回上下文时为 null。 */
    returnToUid,
    restore,
    retryRestore,
    switchAccount,
    removeAccount,
    useOtherAccount,
    applyManualLogin,
    beginAddAccount,
    returnFromAddAccount,
  }
}
