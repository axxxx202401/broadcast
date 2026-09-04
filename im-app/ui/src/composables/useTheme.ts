import { computed, ref } from 'vue'

const THEME_KEY = 'im-theme'

/** 读取用户保存的主题偏好；未设置时回退到深色。 */
function readStored(): 'light' | 'dark' {
  try {
    return (localStorage.getItem(THEME_KEY) as 'light' | 'dark') ?? 'dark'
  } catch {
    return 'dark'
  }
}

/**
 * 管理日夜主题切换，持久化到 localStorage。
 * 同步写入 <html data-theme>，点击后立即生效无需重启。
 */
export function useTheme() {
  const mode = ref<'light' | 'dark'>(readStored())

  const isLight = computed(() => mode.value === 'light')

  /** 同步主题到 DOM，初始化时立即执行避免闪烁。 */
  function applyToDOM() {
    if (typeof document !== 'undefined') {
      document.documentElement.setAttribute('data-theme', mode.value)
    }
  }

  // 同步写入，不依赖 Vue 生命周期，确保 SSR/测试/桌面 WebView 均能立即生效。
  applyToDOM()

  function toggle() {
    mode.value = mode.value === 'dark' ? 'light' : 'dark'
    applyToDOM()
    try {
      localStorage.setItem(THEME_KEY, mode.value)
    } catch {
      // 存储不可用时静默忽略。
    }
  }

  return { mode, isLight, toggle }
}
