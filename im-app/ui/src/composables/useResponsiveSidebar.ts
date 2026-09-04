import { getCurrentInstance, onUnmounted, ref } from 'vue'

/** 窄屏断点，与 `console.css` 的 `@media (max-width: 900px)` 保持一致。 */
const NARROW_QUERY = '(max-width: 900px)'

/**
 * 管理已登录工作区的群列表侧栏开关。
 *
 * 使用 `window.matchMedia('(max-width: 900px)')` 判断窄屏并监听 `change`：
 * 窄屏默认关闭侧栏，优先展示消息；宽屏默认展开，保持双栏。
 * `selectGroup` 仅在窄屏关闭抽屉。存在组件实例时，卸载会移除监听以免泄漏。
 */
export function useResponsiveSidebar() {
  const media = window.matchMedia(NARROW_QUERY)
  const isNarrow = ref(media.matches)
  const sidebarOpen = ref(!media.matches)
  /** 宽屏下侧栏整体收起为窄条模式（仅保留图标），由用户手动切换。 */
  const sidebarCollapsed = ref(false)

  /** 断点变化时同步窄屏标志，并按宽/窄默认值重置开合状态。 */
  const applyBreakpoint = (narrow: boolean) => {
    isNarrow.value = narrow
    sidebarOpen.value = !narrow
    // 断点切换时重置手动收起状态，避免状态残留
    if (narrow) sidebarCollapsed.value = false
  }

  const onChange = (event: MediaQueryListEvent) => {
    applyBreakpoint(event.matches)
  }

  media.addEventListener('change', onChange)
  if (getCurrentInstance()) {
    onUnmounted(() => {
      media.removeEventListener('change', onChange)
    })
  }

  /** 展开或收起群列表抽屉。 */
  const toggleSidebar = () => {
    sidebarOpen.value = !sidebarOpen.value
  }

  /**
   * 用户选择某个群或「全部群消息」后调用。
   * 仅窄屏关闭抽屉，避免挡住消息区；宽屏保持展开。
   */
  const selectGroup = () => {
    if (isNarrow.value) {
      sidebarOpen.value = false
    }
  }

  /** 关闭抽屉，供遮罩点击和 Escape 使用。 */
  const closeSidebar = () => {
    sidebarOpen.value = false
  }

  /** 宽屏下切换侧栏收起/展开（窄条 ↔ 全宽）。 */
  const toggleCollapsed = () => {
    sidebarCollapsed.value = !sidebarCollapsed.value
  }

  return {
    isNarrow,
    sidebarOpen,
    sidebarCollapsed,
    toggleSidebar,
    selectGroup,
    closeSidebar,
    toggleCollapsed,
  }
}
